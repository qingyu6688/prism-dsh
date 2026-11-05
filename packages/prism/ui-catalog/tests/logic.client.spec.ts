/** 不涉及渲染的部分：金额日期格式、表单和接口字段的转换、分页加载、Remote 结果的拆解。 */
import { describe, expect, it, vi } from 'vitest'
import { RemoteError } from '@deepseek-ai/dsh-client-test-runtime'
import type { Product, ProductFilter, ProductPage } from '@prism/dsh-host-catalog/types'
import { CatalogRequestError, createCatalogApi, fileUrl, unwrap } from '../src/client/api.ts'
import { parseTags } from '../src/client/AssetPanel.tsx'
import { EMPTY_DRAFT, draftOf, formatDate, formatMoney, inputOf, parseAttributes, parseCost } from '../src/client/format.ts'
import { PAGE_SIZE, ProductRows } from '../src/client/product-rows.ts'

function product(n: number, overrides: Partial<Product> = {}): Product {
  return {
    id: `p${n}`, code: `C-${n}`, name: `商品 ${n}`, category: null, brand: null, description: null,
    sellingPoints: [], attributes: [], costCents: null, currency: 'CNY', targetMarkets: [], archived: false,
    createdAt: 0, updatedAt: 0, ...overrides,
  }
}

describe('格式', () => {
  it('金额按分显示，常见币种带符号', () => {
    expect(formatMoney(2580, 'CNY')).toBe('¥25.80')
    expect(formatMoney(5, 'USD')).toBe('$0.05')
    expect(formatMoney(100, 'THB')).toBe('THB 1.00')
    expect(formatMoney(null, 'CNY')).toBe('—')
  })

  it('日期按本地时间', () => {
    expect(formatDate(new Date(2026, 0, 5, 23, 59).getTime())).toBe('2026-01-05')
  })

  it('成本：元转分，可带符号和千分位，最多两位小数', () => {
    expect(parseCost('25.8')).toBe(2580)
    expect(parseCost('¥1,299.99')).toBe(129999)
    expect(parseCost(' ')).toBeNull()
    expect(parseCost('abc')).toBeUndefined()
    expect(parseCost('1.234')).toBeUndefined()
    expect(parseCost('-3')).toBeUndefined()
  })

  it('参数：一行一项，中英文冒号或等号', () => {
    expect(parseAttributes('材质：纯棉\n克重 = 180g\n\nSize: M')).toEqual([
      { name: '材质', value: '纯棉' }, { name: '克重', value: '180g' }, { name: 'Size', value: 'M' },
    ])
    expect(parseAttributes('没有冒号')).toBeUndefined()
  })
})

describe('商品表单', () => {
  it('转成接口输入：去空白、空的可选字段为 null、市场代码大写', () => {
    const result = inputOf({
      ...EMPTY_DRAFT, name: ' 纯棉T恤 ', code: '', cost: '25.8', currency: 'usd',
      markets: 'us, uk、de', sellingPoints: '透气\n\n不起球', attributes: '材质：棉',
    })
    expect(result).toEqual({
      input: {
        name: '纯棉T恤', code: null, category: null, brand: null, description: null,
        sellingPoints: ['透气', '不起球'], attributes: [{ name: '材质', value: '棉' }],
        costCents: 2580, currency: 'USD', targetMarkets: ['US', 'UK', 'DE'],
      },
    })
  })

  it('名称必填、成本和参数格式不对时列出问题', () => {
    expect(inputOf({ ...EMPTY_DRAFT, cost: 'x', attributes: '坏行' })).toEqual({
      problems: { name: 'required', cost: 'cost', attributes: 'attribute' },
    })
  })

  it('已有商品转成表单再转回来，内容不变', () => {
    const original = product(1, {
      name: 'T恤', code: 'TS-1', costCents: 2580, targetMarkets: ['US'], sellingPoints: ['透气'],
      attributes: [{ name: '材质', value: '棉' }], description: '说明',
    })
    const back = inputOf(draftOf(original))
    if (!('input' in back)) throw new Error('应该能转回输入')
    expect(back.input).toMatchObject({ name: 'T恤', code: 'TS-1', costCents: 2580, targetMarkets: ['US'], attributes: [{ name: '材质', value: '棉' }] })
  })
})

describe('分页加载', () => {
  /** 一共 total 个商品的假接口，记下每次请求。 */
  function fakeLoad(total: number) {
    const requests: ProductFilter[] = []
    const pending: { resolve: () => void }[] = []
    const load = vi.fn((filter: ProductFilter): Promise<ProductPage> => {
      requests.push(filter)
      const offset = filter.offset ?? 0
      const items = Array.from({ length: Math.max(0, Math.min(PAGE_SIZE, total - offset)) }, (_, i) => product(offset + i))
      return new Promise((resolve) => { pending.push({ resolve: () => { resolve({ items, total }) } }) })
    })
    const flush = async (): Promise<void> => {
      for (const p of pending.splice(0)) p.resolve()
      await Promise.resolve()
      await Promise.resolve()
    }
    return { load, requests, flush }
  }

  it('先取第一页，滚到哪取到哪，取过的页不重复取', async () => {
    const fake = fakeLoad(1000)
    const rows = new ProductRows(fake.load)
    const seen: number[] = []
    rows.subscribe(() => { seen.push(rows.snapshot().version) })
    rows.setFilter({ keyword: ' T恤 ', archived: false })
    expect(rows.snapshot().status).toBe('loading')
    await fake.flush()
    expect(rows.snapshot()).toMatchObject({ status: 'ready', total: 1000 })
    expect(fake.requests[0]).toEqual({ keyword: 'T恤', archived: false, limit: PAGE_SIZE, offset: 0 })
    expect(rows.get(199)?.id).toBe('p199')
    expect(rows.get(200)).toBeUndefined()

    rows.ensure(150, 450)
    expect(fake.requests.map(r => r.offset)).toEqual([0, 200, 400])
    rows.ensure(150, 450)
    expect(fake.load).toHaveBeenCalledTimes(3)
    await fake.flush()
    expect(rows.get(450)?.id).toBe('p450')
    // 超出总数的范围不请求
    rows.ensure(990, 5000)
    expect(fake.requests.map(r => r.offset)).toEqual([0, 200, 400, 800])
    expect(seen.length).toBeGreaterThan(2)
  })

  it('换了筛选条件后，之前发出去的请求回来也不采用', async () => {
    const fake = fakeLoad(10)
    const rows = new ProductRows(fake.load)
    rows.setFilter({ keyword: '旧', archived: false })
    rows.setFilter({ keyword: '新', archived: true })
    await fake.flush()
    expect(rows.snapshot()).toMatchObject({ status: 'ready', total: 10 })
    expect(fake.requests.at(-1)).toMatchObject({ keyword: '新', archived: true })
    // 第一个请求回来时被丢掉了，版本号只因为第二个请求涨
  })

  it('第一页失败显示错误，重新加载后恢复', async () => {
    let fail = true
    const rows = new ProductRows(() => fail
      ? Promise.reject(new CatalogRequestError('棱镜业务服务不可用', 'unavailable'))
      : Promise.resolve({ items: [product(0)], total: 1 }))
    rows.setFilter({ keyword: '', archived: false })
    await Promise.resolve(); await Promise.resolve()
    expect(rows.snapshot()).toMatchObject({ status: 'error', error: '棱镜业务服务不可用' })
    fail = false
    rows.reload()
    await Promise.resolve(); await Promise.resolve()
    expect(rows.snapshot()).toMatchObject({ status: 'ready', total: 1, error: null })
  })
})

describe('unwrap', () => {
  it('成功返回值；业务错误带服务的说明和分类；其他错误换成能看懂的话', async () => {
    await expect(unwrap(Promise.resolve({ ok: true as const, value: 1 }), '连不上')).resolves.toBe(1)

    const business = await unwrap(Promise.resolve({
      ok: false as const, error: new RemoteError('prism/failed', '商品不存在', { code: 'not_found', detail: null }),
    }), '连不上').catch((e: unknown) => e)
    expect(business).toBeInstanceOf(CatalogRequestError)
    expect(business).toMatchObject({ message: '商品不存在', code: 'not_found' })

    const transport = await unwrap(Promise.resolve({
      ok: false as const, error: new RemoteError('gateway/internal', 'socket hang up', {}),
    }), '连不上棱镜业务服务').catch((e: unknown) => e)
    expect(transport).toMatchObject({ message: '连不上棱镜业务服务（gateway/internal）', code: 'gateway/internal' })
  })
})

describe('上传和文件地址', () => {
  const remote = {} as Parameters<typeof createCatalogApi>[0]
  const file = new File(['货号\n'], '商品 表.csv')

  it('上传：文件名进查询参数，成功返回 Host 上的路径', async () => {
    const fetchImpl = vi.fn<typeof fetch>(() => Promise.resolve(Response.json({ path: 'D:/data/inbox/1/商品 表.csv', name: '商品 表.csv', size: 6 })))
    const api = createCatalogApi(remote, '连不上', fetchImpl)
    await expect(api.upload(file)).resolves.toMatchObject({ path: 'D:/data/inbox/1/商品 表.csv' })
    const [url, init] = fetchImpl.mock.calls[0]!
    expect(url).toBe('/api/prism/upload?name=%E5%95%86%E5%93%81%20%E8%A1%A8.csv')
    expect(init).toMatchObject({ method: 'POST', body: file, credentials: 'same-origin' })
  })

  it('上传失败用路由给的中文说明；连不上时用通用说明', async () => {
    const rejected = createCatalogApi(remote, '连不上', () => Promise.resolve(Response.json({ code: 'too_large', message: '文件超过 50 MB' }, { status: 413 })))
    await expect(rejected.upload(file)).rejects.toMatchObject({ message: '文件超过 50 MB', code: 'too_large' })
    const broken = createCatalogApi(remote, '连不上', () => Promise.resolve(new Response('oops', { status: 502 })))
    await expect(broken.upload(file)).rejects.toMatchObject({ message: '连不上（HTTP 502）', code: 'http_502' })
    const offline = createCatalogApi(remote, '连不上', () => Promise.reject(new TypeError('Failed to fetch')))
    await expect(offline.upload(file)).rejects.toMatchObject({ message: '连不上', code: 'unreachable' })
  })

  it('图片地址把相对路径编码进查询参数', () => {
    expect(fileUrl('thumbs/ab/c d.webp')).toBe('/api/prism/file?path=thumbs%2Fab%2Fc%20d.webp')
  })

  it('标签按中英文逗号、顿号拆开，去空白和重复', () => {
    expect(parseTags('主图, 红色，白底、主图, ')).toEqual(['主图', '红色', '白底'])
    expect(parseTags('  ')).toEqual([])
  })
})
