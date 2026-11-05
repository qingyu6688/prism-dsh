// @vitest-environment jsdom
/**
 * 商品库页面和店铺分区的组件测试：接口用内存里的假实现，检查列表、空状态、错误状态、详情抽屉、
 * 新建和编辑表单、归档恢复、删除确认，以及服务端报错的显示。
 */
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest'
import { makeTranslate } from '@deepseek-ai/dsh-client-test-runtime'
import type { Product, ProductDetail, ProductInput, Shop, ShopInput } from '@prism/dsh-host-catalog/types'
import { CatalogRequestError, type CatalogApi } from '../src/client/api.ts'
import { zh } from '../src/client/locales.ts'
import type { Translate } from '../src/client/ProductForm.tsx'
import { ProductsPage } from '../src/client/ProductsPage.tsx'
import { ShopsSection } from '../src/client/ShopsSection.tsx'

const t = makeTranslate(zh) as Translate

// jsdom 不排版，元素尺寸都是 0，虚拟列表会认为没有可见区域。给滚动区域一个窗口大小。
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    x: 0, y: 0, top: 0, left: 0, right: 960, bottom: 640, width: 960, height: 640, toJSON: () => ({}),
  })
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockReturnValue(640)
  vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockReturnValue(960)
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

/** 每个方法都是带类型的 mock，测试里可以检查调用、临时改返回值。 */
type FakeApi = { [K in keyof CatalogApi]: Mock<CatalogApi[K]> } & { products: Product[]; shops: Shop[] }

/** 内存里的商品库和店铺，行为和服务端一致的部分：货号唯一、归档、按关键词筛选。 */
function fakeApi(initial: Product[] = [], shops: Shop[] = []): FakeApi {
  let next = 100
  const state = { products: [...initial], shops: [...shops] }
  const toProduct = (id: string, input: ProductInput, archived = false): Product => ({
    id, name: input.name, code: input.code ?? null, category: input.category ?? null, brand: input.brand ?? null,
    description: input.description ?? null, sellingPoints: [...(input.sellingPoints ?? [])], attributes: [...(input.attributes ?? [])],
    costCents: input.costCents ?? null, currency: input.currency ?? 'CNY', targetMarkets: [...(input.targetMarkets ?? [])],
    archived, createdAt: 0, updatedAt: 0,
  })
  const find = (id: string): Product => {
    const product = state.products.find(p => p.id === id)
    if (product === undefined) throw new CatalogRequestError('商品不存在', 'not_found')
    return product
  }
  const checkCode = (input: ProductInput, id?: string): void => {
    if (input.code != null && state.products.some(p => p.code === input.code && p.id !== id)) {
      throw new CatalogRequestError(`商品的编码「${input.code}」已经存在`, 'invalid_input')
    }
  }
  const toShop = (id: string, input: ShopInput): Shop => ({
    id, name: input.name, platform: input.platform, site: input.site ?? null, language: input.language ?? 'zh-CN',
    currency: input.currency ?? 'CNY', notes: input.notes ?? null, sortOrder: 0, createdAt: 0, updatedAt: 0,
  })
  return Object.assign(state, {
    listProducts: vi.fn<CatalogApi['listProducts']>((filter) => {
      const keyword = filter.keyword ?? ''
      const items = state.products.filter(p => p.archived === (filter.archived ?? false) && p.name.includes(keyword))
      const offset = filter.offset ?? 0
      return Promise.resolve({ items: items.slice(offset, offset + (filter.limit ?? 50)), total: items.length })
    }),
    productDetail: vi.fn<CatalogApi['productDetail']>((id: string): Promise<ProductDetail> => Promise.resolve({
      product: find(id), skus: [],
      summary: { skuCount: 0, imageAssets: 2, textAssets: 0, starredAssets: 0, copies: [], batches: [], lastBatchAt: null },
    })),
    createProduct: vi.fn<CatalogApi['createProduct']>((input: ProductInput) => {
      checkCode(input)
      const product = toProduct(`p${next++}`, input)
      state.products.unshift(product)
      return Promise.resolve(product)
    }),
    updateProduct: vi.fn<CatalogApi['updateProduct']>((id: string, input: ProductInput) => {
      checkCode(input, id)
      const updated = toProduct(id, input, find(id).archived)
      state.products = state.products.map(p => p.id === id ? updated : p)
      return Promise.resolve(updated)
    }),
    archiveProduct: vi.fn<CatalogApi['archiveProduct']>((id: string) => {
      state.products = state.products.map(p => p.id === id ? { ...p, archived: true } : p)
      return Promise.resolve()
    }),
    restoreProduct: vi.fn<CatalogApi['restoreProduct']>((id: string) => {
      state.products = state.products.map(p => p.id === id ? { ...p, archived: false } : p)
      return Promise.resolve()
    }),
    deleteProduct: vi.fn<CatalogApi['deleteProduct']>((id: string) => {
      state.products = state.products.filter(p => p.id !== id)
      return Promise.resolve()
    }),
    listShops: vi.fn<CatalogApi['listShops']>(() => Promise.resolve([...state.shops])),
    createShop: vi.fn<CatalogApi['createShop']>((input: ShopInput) => {
      const shop = toShop(`s${next++}`, input)
      state.shops.push(shop)
      return Promise.resolve(shop)
    }),
    updateShop: vi.fn<CatalogApi['updateShop']>((id: string, input: ShopInput) => {
      const shop = toShop(id, input)
      state.shops = state.shops.map(s => s.id === id ? shop : s)
      return Promise.resolve(shop)
    }),
    deleteShop: vi.fn<CatalogApi['deleteShop']>((id: string) => {
      state.shops = state.shops.filter(s => s.id !== id)
      return Promise.resolve()
    }),
  })
}

function product(n: number, overrides: Partial<Product> = {}): Product {
  return {
    id: `p${n}`, code: `C-${n}`, name: `商品 ${n}`, category: '上衣', brand: null, description: null,
    sellingPoints: [], attributes: [], costCents: 1000 + n, currency: 'CNY', targetMarkets: [], archived: false,
    createdAt: 0, updatedAt: 0, ...overrides,
  }
}

/** 按标签找输入框；必填项的标签文字后面带一个星号。 */
function field(label: string): HTMLInputElement {
  return screen.getByLabelText(content => content === label || content === `${label}*`) as HTMLInputElement
}

function type(label: string | HTMLElement, value: string): void {
  const input = typeof label === 'string' ? field(label) : label
  fireEvent.change(input, { target: { value } })
}

describe('商品库页面', () => {
  it('列出商品和总数；大列表只渲染可见的行', async () => {
    const api = fakeApi(Array.from({ length: 500 }, (_, i) => product(i)))
    render(<ProductsPage api={api} t={t} />)
    expect(await screen.findByText('共 500 个')).toBeTruthy()
    expect(screen.getByText('商品 0')).toBeTruthy()
    expect(screen.getByText('¥10.00')).toBeTruthy()
    const rendered = screen.getAllByRole('row').length
    expect(rendered).toBeGreaterThan(5)
    expect(rendered).toBeLessThan(60)
  })

  it('没有商品时显示空状态和新建按钮', async () => {
    render(<ProductsPage api={fakeApi()} t={t} />)
    expect(await screen.findByText('还没有商品')).toBeTruthy()
    expect(screen.getAllByRole('button', { name: '新建商品' })).toHaveLength(2)
  })

  it('加载失败时显示原因，点重试再取', async () => {
    const api = fakeApi([product(1)])
    api.listProducts.mockRejectedValueOnce(new CatalogRequestError('棱镜业务服务不可用：没找到 prism-service.exe', 'unavailable'))
    render(<ProductsPage api={api} t={t} />)
    expect(await screen.findByText(/商品列表没有加载出来：棱镜业务服务不可用/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(await screen.findByText('商品 1')).toBeTruthy()
  })

  it('搜索停止输入后再查', async () => {
    const api = fakeApi([product(1, { name: '纯棉T恤' }), product(2, { name: '陶瓷杯' })])
    render(<ProductsPage api={api} t={t} />)
    await screen.findByText('共 2 个')
    type('搜索名称、货号、类目、品牌', '陶瓷')
    expect(await screen.findByText('共 1 个')).toBeTruthy()
    expect(screen.queryByText('纯棉T恤')).toBeNull()
    expect(api.listProducts).toHaveBeenLastCalledWith(expect.objectContaining({ keyword: '陶瓷' }))
    type('搜索名称、货号、类目、品牌', '没有的东西')
    expect(await screen.findByText('没有找到符合条件的商品')).toBeTruthy()
  })

  it('新建商品：校验、保存后打开详情并刷新列表', async () => {
    const api = fakeApi()
    render(<ProductsPage api={api} t={t} />)
    await screen.findByText('还没有商品')
    fireEvent.click(screen.getAllByRole('button', { name: '新建商品' })[0]!)
    type('成本（元）', 'abc')
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(screen.getByText('必填')).toBeTruthy()
    expect(screen.getByText('填数字，最多两位小数')).toBeTruthy()
    expect(api.createProduct).not.toHaveBeenCalled()

    type('名称', '纯棉T恤')
    type('成本（元）', '25.8')
    type('参数', '材质：纯棉')
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => { expect(api.createProduct).toHaveBeenCalledTimes(1) })
    expect(api.createProduct.mock.calls[0]![0]).toMatchObject({ name: '纯棉T恤', costCents: 2580, attributes: [{ name: '材质', value: '纯棉' }] })
    const drawer = await screen.findByRole('complementary', { name: '纯棉T恤' })
    expect(within(drawer).getByText('¥25.80')).toBeTruthy()
    expect(within(drawer).getByText('图片 2 张')).toBeTruthy()
    expect(await screen.findByText('共 1 个')).toBeTruthy()
  })

  it('货号重复时显示服务端的说明，表单保留', async () => {
    const api = fakeApi([product(1, { code: 'TS-001' })])
    render(<ProductsPage api={api} t={t} />)
    await screen.findByText('共 1 个')
    fireEvent.click(screen.getByRole('button', { name: '新建商品' }))
    type('名称', '另一个')
    type('货号', 'TS-001')
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(await screen.findByText('没有保存：商品的编码「TS-001」已经存在')).toBeTruthy()
    expect(field('名称').value).toBe('另一个')
  })

  it('详情里编辑、归档、恢复', async () => {
    const api = fakeApi([product(1, { name: '纯棉T恤' })])
    render(<ProductsPage api={api} t={t} />)
    fireEvent.click(await screen.findByText('纯棉T恤'))
    const drawer = await screen.findByRole('complementary', { name: '纯棉T恤' })

    fireEvent.click(within(drawer).getByRole('button', { name: '编辑' }))
    expect(field('名称').value).toBe('纯棉T恤')
    type('名称', '纯棉短袖T恤')
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(await screen.findByRole('complementary', { name: '纯棉短袖T恤' })).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: '归档' }))
    expect(await screen.findByText('这个商品已归档，不在在用列表里显示。')).toBeTruthy()
    expect(api.archiveProduct).toHaveBeenCalledWith('p1')
    expect(await screen.findByText('还没有商品')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: '恢复为在用' }))
    await waitFor(() => { expect(api.restoreProduct).toHaveBeenCalledWith('p1') })
    expect(await screen.findByText('共 1 个')).toBeTruthy()
  })

  it('删除要确认，确认后关闭详情', async () => {
    const api = fakeApi([product(1, { name: '纯棉T恤' })])
    render(<ProductsPage api={api} t={t} />)
    fireEvent.click(await screen.findByText('纯棉T恤'))
    await screen.findByRole('complementary', { name: '纯棉T恤' })
    fireEvent.click(screen.getByRole('button', { name: '删除' }))
    expect(screen.getByText('删除「纯棉T恤」？')).toBeTruthy()
    expect(api.deleteProduct).not.toHaveBeenCalled()
    const dialog = screen.getByRole('dialog', { name: '删除「纯棉T恤」？' })
    fireEvent.click(within(dialog).getByRole('button', { name: '删除' }))
    await waitFor(() => { expect(api.deleteProduct).toHaveBeenCalledWith('p1') })
    await waitFor(() => { expect(screen.queryByRole('complementary')).toBeNull() })
    expect(await screen.findByText('还没有商品')).toBeTruthy()
  })
})

describe('设置里的店铺', () => {
  const shop: Shop = {
    id: 's1', name: '美国主站', platform: 'amazon', site: 'US', language: 'en-US', currency: 'USD',
    notes: null, sortOrder: 0, createdAt: 0, updatedAt: 0,
  }

  it('列出店铺，平台显示中文名', async () => {
    render(<ShopsSection api={fakeApi([], [shop, { ...shop, id: 's2', name: '自建站', platform: 'shopify', site: null }])} t={t} />)
    expect(await screen.findByText('美国主站')).toBeTruthy()
    expect(screen.getByText('亚马逊')).toBeTruthy()
    // 不认识的平台原样显示
    expect(screen.getByText('shopify')).toBeTruthy()
  })

  it('添加店铺：名称、平台必填，站点和币种转大写', async () => {
    const api = fakeApi()
    render(<ShopsSection api={api} t={t} />)
    expect(await screen.findByText('还没有添加店铺')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '添加店铺' }))
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(screen.getAllByText('必填')).toHaveLength(2)
    expect(api.createShop).not.toHaveBeenCalled()

    type('名称', '美国主站')
    // 按钮关联了"平台"标签，无障碍名称是标签文字；按钮上显示的是"选择平台"
    const picker = screen.getByRole('button', { name: /^平台/ })
    expect(picker.textContent).toBe('选择平台')
    fireEvent.click(picker)
    fireEvent.click(await screen.findByText('亚马逊'))
    type('站点', 'us')
    type('币种', 'usd')
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => { expect(api.createShop).toHaveBeenCalledTimes(1) })
    expect(api.createShop.mock.calls[0]![0]).toEqual({
      name: '美国主站', platform: 'amazon', site: 'US', language: 'zh-CN', currency: 'USD', notes: null,
    })
    expect(await screen.findByRole('cell', { name: '美国主站' })).toBeTruthy()
  })

  it('删除要确认', async () => {
    const api = fakeApi([], [shop])
    render(<ShopsSection api={api} t={t} />)
    fireEvent.click(await screen.findByRole('button', { name: '删除 美国主站' }))
    const dialog = screen.getByRole('dialog', { name: '删除店铺「美国主站」？' })
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: '删除' })) })
    await waitFor(() => { expect(api.deleteShop).toHaveBeenCalledWith('s1') })
    expect(await screen.findByText('还没有添加店铺')).toBeTruthy()
  })
})
