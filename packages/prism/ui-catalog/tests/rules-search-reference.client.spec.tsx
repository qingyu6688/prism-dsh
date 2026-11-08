// @vitest-environment jsdom
/**
 * D3.4 的组件和逻辑测试：设置里的规则包、全局搜索框和跳转、输入框 `@` 引用商品和聊天记录里的显示。
 */
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { makeTranslate } from '@deepseek-ai/dsh-client-test-runtime'
import type { InputTriggerPick } from '@deepseek-ai/dsh-client-ui-input-trigger/client'
import type { SessionId } from '@deepseek-ai/dsh-session/types'
import { CatalogRequestError } from '../src/client/api.ts'
import { AssetsPage } from '../src/client/AssetsPage.tsx'
import { zh } from '../src/client/locales.ts'
import { CatalogNavigator } from '../src/client/navigator.ts'
import {
  createProductSource, parseProductMention, ProductReference, productMention, productReferenceRule,
} from '../src/client/product-reference.tsx'
import type { Translate } from '../src/client/ProductForm.tsx'
import { ProductsPage } from '../src/client/ProductsPage.tsx'
import { imageSummary, localized, RulesSection } from '../src/client/RulesSection.tsx'
import { SearchPalette } from '../src/client/SearchPalette.tsx'
import { asset, fakeApi, product, RULES } from './fake-api.client.ts'

const t = makeTranslate(zh) as Translate
const session = { sessionId: 's1' as SessionId }

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function navigator(): { nav: CatalogNavigator; select: ReturnType<typeof vi.fn> } {
  const select = vi.fn()
  return { nav: new CatalogNavigator(select, { products: 'prism-products', assets: 'prism-assets' }), select }
}

describe('规则包', () => {
  it('列出规则包、词表和加载失败的文件；来源、核对状态、标题上限、主图要求', async () => {
    render(<RulesSection api={fakeApi()} t={t} />)
    const amazon = (await screen.findByText('亚马逊')).closest('tr') as HTMLElement
    expect(within(amazon).getByText('已核对')).toBeTruthy()
    expect(within(amazon).getByText('200 字符')).toBeTruthy()
    expect(within(amazon).getByText('最短边 1000px · 首图白底')).toBeTruthy()
    expect(within(amazon).getByText('导入')).toBeTruthy()
    expect(within(amazon).getByText('盖过内置修订号 1')).toBeTruthy()
    const taobao = screen.getByText('淘宝 / 天猫').closest('tr') as HTMLElement
    expect(within(taobao).getByText('未核对')).toBeTruthy()
    expect(within(taobao).getByText('60 字符（汉字算 2 个）')).toBeTruthy()
    expect(within(taobao).getByText('内置')).toBeTruthy()
    // 内置版本不能删
    expect(within(taobao).queryByRole('button')).toBeNull()
    expect(screen.getByText('广告法极限词')).toBeTruthy()
    expect(screen.getByText('1 个规则文件有问题，没有加载')).toBeTruthy()
    expect(screen.getByText('缺少必填字段 formatVersion')).toBeTruthy()
  })

  it('导入规则文件：先上传再导入；校验不通过时逐条列出问题', async () => {
    const api = fakeApi()
    api.importRuleFile.mockRejectedValueOnce(new CatalogRequestError('文件有 2 处问题：…', 'invalid_input', [
      { path: 'formatVersion', message: '缺少必填字段 formatVersion' },
      { path: 'revision', message: '修订号要是正整数' },
    ]))
    render(<RulesSection api={api} t={t} />)
    await screen.findByText('亚马逊')
    const input = screen.getByLabelText('导入规则文件')
    fireEvent.change(input, { target: { files: [new File(['{}'], 'bad.json')] } })
    expect(await screen.findByText('没有导入：文件有 2 处问题')).toBeTruthy()
    expect(screen.getByText('修订号要是正整数')).toBeTruthy()

    fireEvent.change(input, { target: { files: [new File(['{}'], 'amazon.json')] } })
    expect(await screen.findByText('已导入「amazon」，修订号 3')).toBeTruthy()
    expect(api.importRuleFile).toHaveBeenLastCalledWith(expect.stringContaining('amazon.json'))
    expect(api.listRules).toHaveBeenCalledTimes(2)
  })

  it('删除导入的版本要确认', async () => {
    const api = fakeApi()
    render(<RulesSection api={api} t={t} />)
    fireEvent.click(await screen.findByRole('button', { name: '删除导入的版本 亚马逊' }))
    const dialog = screen.getByRole('dialog', { name: '删除导入的「亚马逊」？' })
    await act(async () => { fireEvent.click(within(dialog).getByRole('button', { name: '删除' })) })
    expect(api.removeUserRule).toHaveBeenCalledWith('rule_pack', 'amazon')
    await waitFor(() => { expect(screen.queryByText('亚马逊')).toBeNull() })
  })

  it('名称按界面语言取，摘要只列有的项', () => {
    expect(localized({ 'zh-CN': '亚马逊', en: 'Amazon' }, 'en')).toBe('Amazon')
    expect(localized({ ja: 'アマゾン' }, 'zh-CN')).toBe('アマゾン')
    const [amazon] = RULES.packs
    expect(imageSummary({ ...amazon!, value: { ...amazon!.value, images: {} } }, t)).toBe('—')
  })
})

describe('全局搜索', () => {
  it('输入后搜索，商品和素材分组；点商品跳到商品库', async () => {
    const api = fakeApi([product(1, { name: '夏季亚麻衬衫', code: 'NEW-001' })], [], [asset(1, { title: '亚麻主图' })])
    const { nav, select } = navigator()
    render(<SearchPalette api={api} t={t} navigator={nav} />)
    expect(screen.queryByRole('dialog')).toBeNull()
    act(() => { nav.openSearch() })
    const input = await screen.findByLabelText('商品名称、货号、类目、品牌，或素材标题、标签、批次号')
    expect(screen.getByText('上下键选择，回车打开。会话用左侧的搜索（Ctrl+K）。')).toBeTruthy()
    fireEvent.change(input, { target: { value: ' 亚麻 ' } })
    const products = await screen.findByRole('region', { name: '商品' })
    expect(api.search).toHaveBeenCalledWith('亚麻')
    expect(within(products).getByText('NEW-001 · 上衣')).toBeTruthy()
    expect(within(screen.getByRole('region', { name: '素材' })).getByText('亚麻主图')).toBeTruthy()

    fireEvent.click(within(products).getByRole('button', { name: /夏季亚麻衬衫/ }))
    expect(select).toHaveBeenCalledWith('prism-products')
    expect(nav.snapshot()).toMatchObject({ searchOpen: false, product: { value: 'p1' } })
  })

  it('上下键选、回车打开：素材跳到素材库并带上关键词', async () => {
    const api = fakeApi([product(1, { name: '亚麻衬衫' })], [], [asset(1, { title: '亚麻主图' })])
    const { nav, select } = navigator()
    render(<SearchPalette api={api} t={t} navigator={nav} />)
    act(() => { nav.openSearch() })
    const input = await screen.findByLabelText('商品名称、货号、类目、品牌，或素材标题、标签、批次号')
    fireEvent.change(input, { target: { value: '亚麻' } })
    await screen.findByRole('region', { name: '素材' })
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(select).toHaveBeenCalledWith('prism-assets')
    expect(nav.snapshot().assetKeyword?.value).toBe('亚麻')
  })

  it('按批次号搜索并打开对应的生图结果', async () => {
    const api = fakeApi()
    api.search.mockResolvedValue({ products: [], assets: [], batches: [{ id: 'b7', seq: 7, status: 'completed', total: 2 }] })
    const { nav, select } = navigator()
    render(<SearchPalette api={api} t={t} navigator={nav} />)
    act(() => { nav.openSearch() })
    fireEvent.change(await screen.findByLabelText('商品名称、货号、类目、品牌，或素材标题、标签、批次号'), { target: { value: '#7' } })
    fireEvent.click(await screen.findByRole('button', { name: /批次 #7/ }))
    expect(api.search).toHaveBeenCalledWith('#7')
    expect(select).toHaveBeenCalledWith('prism-images')
    expect(nav.snapshot()).toMatchObject({ searchOpen: false, imageBatch: 'b7' })
  })

  it('没有结果、出错时说明', async () => {
    const api = fakeApi()
    const { nav } = navigator()
    render(<SearchPalette api={api} t={t} navigator={nav} />)
    act(() => { nav.openSearch() })
    const input = await screen.findByLabelText('商品名称、货号、类目、品牌，或素材标题、标签、批次号')
    fireEvent.change(input, { target: { value: '没有的东西' } })
    expect(await screen.findByText('没有找到')).toBeTruthy()
    api.search.mockRejectedValueOnce(new CatalogRequestError('棱镜业务服务不可用', 'unavailable'))
    fireEvent.change(input, { target: { value: '别的' } })
    expect(await screen.findByText('没有搜出来：棱镜业务服务不可用')).toBeTruthy()
  })

  it('跳转请求让商品库打开详情、素材库按关键词筛选', async () => {
    const api = fakeApi([product(1, { name: '亚麻衬衫', archived: true })], [], [asset(1, { title: '主图' }), asset(2, { title: '细节' })])
    const { nav } = navigator()
    const { unmount } = render(<ProductsPage api={api} t={t} navigator={nav} />)
    act(() => { nav.showProduct('p1') })
    // 归档的商品不在在用列表里，也能从搜索打开详情
    expect(await screen.findByRole('complementary', { name: '亚麻衬衫' })).toBeTruthy()
    expect(screen.getByRole('button', { name: '搜索商品、素材和批次' })).toBeTruthy()
    unmount()

    render(<AssetsPage api={api} t={t} navigator={nav} />)
    await screen.findByText('共 2 个')
    act(() => { nav.showAssets('主图') })
    expect(await screen.findByText('共 1 个')).toBeTruthy()
    expect((screen.getByLabelText<HTMLInputElement>('搜索标题、文字内容、标签')).value).toBe('主图')
    fireEvent.click(screen.getByRole('button', { name: '搜索商品、素材和批次' }))
    expect(nav.snapshot().searchOpen).toBe(true)
  })
})

describe('@ 引用商品', () => {
  it('引用写法：名称里的方括号和换行换成空格，id 编码；能解析回来', () => {
    const mention = productMention({ id: 'p 1', name: '夏季[新款]\n衬衫' })
    expect(mention).toBe('@[夏季 新款 衬衫](prism-product:p%201)')
    expect(parseProductMention(mention)).toEqual({ label: '夏季 新款 衬衫', id: 'p 1' })
    expect(parseProductMention('@[x](dsh-session:1)')).toBeUndefined()
    expect(parseProductMention(`${mention} 多余`)).toBeUndefined()
  })

  it('候选：按关键词查商品，最多 8 个，带货号和类目；带引号的路径不查', async () => {
    const api = fakeApi(Array.from({ length: 12 }, (_, n) => product(n, { name: `亚麻衬衫 ${n}` })))
    const source = createProductSource(api, t)
    const signal = new AbortController().signal
    const candidates = await source.candidates(session, { query: ' 亚麻 ', position: 'inline', drilled: false, signal })
    expect(api.listProducts).toHaveBeenCalledWith({ keyword: '亚麻', limit: 8 })
    expect(candidates).toHaveLength(8)
    expect(candidates[0]).toMatchObject({ label: '亚麻衬衫 0', description: 'C-0 · 上衣', section: '商品', value: '@[亚麻衬衫 0](prism-product:p0)' })
    expect(await source.candidates(session, { query: 'a', quoted: true, position: 'inline', drilled: false, signal })).toEqual([])
  })

  it('选中后插入原子引用；发给模型的文字就是引用写法', async () => {
    const source = createProductSource(fakeApi(), t)
    const value = '@[亚麻衬衫](prism-product:p1)'
    const pick = { candidate: { name: '亚麻衬衫', value }, session, position: 'inline', via: 'menu', action: 'pick' } as unknown as InputTriggerPick
    expect(source.onPick(pick)).toEqual({
      insert: { source: 'prism-products', ref: value, label: '亚麻衬衫', clipboardText: value },
    })
    await expect(source.codec?.serialize(value, new AbortController().signal)).resolves.toBe(value)
  })

  it('聊天记录里找出引用并显示成标签', () => {
    const text = '帮我看看 @[亚麻衬衫](prism-product:p1) 和 @[另一个](dsh-session:s2)'
    const mention = '@[亚麻衬衫](prism-product:p1)'
    const start = text.indexOf(mention)
    const end = start + mention.length
    const node = productReferenceRule.match(text, 0, { read: () => undefined } as never)
    expect(node).toEqual({ id: 'prism-product', start, end, data: { label: '亚麻衬衫', id: 'p1' } })
    // 会话引用不是商品引用
    expect(productReferenceRule.match(text, end, { read: () => undefined } as never)).toBeUndefined()

    if (node?.id !== 'prism-product') throw new Error('没有识别出商品引用')
    render(<ProductReference node={node} raw={mention} />)
    const chip = screen.getByText('亚麻衬衫')
    expect(chip.getAttribute('title')).toBe('@[亚麻衬衫](prism-product:p1)')
  })
})
