// @vitest-environment jsdom
/**
 * 导入商品对话框和素材库页面的组件测试：接口用 fake-api.ts 里的内存实现。
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { makeTranslate } from '@deepseek-ai/dsh-client-test-runtime'
import { CatalogRequestError } from '../src/client/api.ts'
import { AssetsPage } from '../src/client/AssetsPage.tsx'
import { ImportDialog } from '../src/client/ImportDialog.tsx'
import { zh } from '../src/client/locales.ts'
import type { Translate } from '../src/client/ProductForm.tsx'
import { asset, fakeApi, PREVIEW, product } from './fake-api.client.ts'

const t = makeTranslate(zh) as Translate

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

function pick(input: HTMLElement, ...files: File[]): void {
  fireEvent.change(input, { target: { files } })
}

function csv(name = '供应商商品.csv'): File {
  return new File(['款号,品名,采购价\n'], name, { type: 'text/csv' })
}

function png(name: string): File {
  return new File([new Uint8Array([1, 2, 3])], name, { type: 'image/png' })
}

/** 菜单选项（dsh 的菜单渲染在 body 下）。 */
async function choose(trigger: HTMLElement, option: string): Promise<void> {
  fireEvent.click(trigger)
  fireEvent.click(await screen.findByText(option, { selector: '[role^=menuitem] *, [role^=menuitem]' }))
}

describe('导入商品', () => {
  it('选文件后上传并预览，自动对好的列显示在下拉里，导入后列出结果和问题行', async () => {
    const api = fakeApi()
    const onClose = vi.fn()
    render(<ImportDialog open api={api} t={t} onClose={onClose} />)
    pick(screen.getByLabelText('选择表格文件'), csv())
    expect(await screen.findByText('供应商商品.csv，共 3 行')).toBeTruthy()
    expect(api.upload).toHaveBeenCalledTimes(1)
    expect(api.previewProductImport).toHaveBeenCalledWith({ path: expect.stringContaining('供应商商品.csv') as string, sheet: null, headerLine: null })
    // 自动对好的列和第一行的值
    expect((screen.getByRole('button', { name: /^商品名称/ })).textContent).toBe('品名')
    expect(screen.getByText('亚麻衬衫')).toBeTruthy()
    expect((screen.getByRole('button', { name: /^成本/ })).textContent).toBe('不导入')

    await choose(screen.getByRole('button', { name: /^成本/ }), '采购价')
    expect(screen.getByText('59.9')).toBeTruthy()
    fireEvent.click(screen.getByRole('tab', { name: '跳过这一行' }))
    fireEvent.click(screen.getByRole('checkbox', { name: /保存这次的对列方式/ }))
    fireEvent.change(screen.getByLabelText('对列方式的名称'), { target: { value: '供应商表' } })
    fireEvent.click(screen.getByRole<HTMLButtonElement>('button', { name: '导入 3 行' }))

    expect(await screen.findByText('导入完成')).toBeTruthy()
    expect(api.lastImport).toMatchObject({
      mapping: { columns: { 'product.name': '品名', 'product.code': '款号', 'product.cost': '采购价' } },
      mode: 'createOnly',
      saveAs: '供应商表',
      sheet: null,
      headerLine: 1,
    })
    expect(screen.getByText('新建 1 个')).toBeTruthy()
    expect(screen.getByText('第 3 行')).toBeTruthy()
    expect(screen.getByText('商品名称是必填的，这一行是空的')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '完成' }))
    expect(onClose).toHaveBeenCalledWith(true)
  })

  it('必填字段没有对应的列时不能导入', async () => {
    render(<ImportDialog open api={fakeApi()} t={t} onClose={vi.fn()} />)
    pick(screen.getByLabelText('选择表格文件'), csv())
    await screen.findByText('供应商商品.csv，共 3 行')
    await choose(screen.getByRole('button', { name: /^商品名称/ }), '不导入')
    expect(screen.getByText('还有必填字段没有对应的列：商品名称')).toBeTruthy()
    expect((screen.getByRole<HTMLButtonElement>('button', { name: '导入 3 行' })).disabled).toBe(true)
  })

  it('改表头行号后重新读取；套用了保存的对列方式时提示', async () => {
    const api = fakeApi()
    api.previewProductImport.mockResolvedValueOnce({ ...PREVIEW, matched: { id: 'm1', name: '供应商表', overlap: 1 } })
    render(<ImportDialog open api={api} t={t} onClose={vi.fn()} />)
    pick(screen.getByLabelText('选择表格文件'), csv())
    expect(await screen.findByText('已套用之前保存的对列方式「供应商表」')).toBeTruthy()
    // 已经套用了保存的方式，就不再问要不要保存
    expect(screen.queryByRole('checkbox')).toBeNull()

    const line = screen.getByLabelText('表头在第几行')
    fireEvent.change(line, { target: { value: '2' } })
    fireEvent.click(screen.getByRole('button', { name: '重新读取' }))
    await waitFor(() => { expect(api.previewProductImport).toHaveBeenLastCalledWith(expect.objectContaining({ headerLine: 2 })) })
  })

  it('上传失败时显示原因，留在选文件这一步', async () => {
    const api = fakeApi()
    api.upload.mockRejectedValueOnce(new CatalogRequestError('只能上传表格（xlsx、xls、ods、csv）', 'invalid_input'))
    const onClose = vi.fn()
    render(<ImportDialog open api={api} t={t} onClose={onClose} />)
    pick(screen.getByLabelText('选择表格文件'), csv('商品.txt'))
    expect(await screen.findByText('没有完成：只能上传表格（xlsx、xls、ods、csv）')).toBeTruthy()
    expect(api.previewProductImport).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(onClose).toHaveBeenCalledWith(false)
  })
})

describe('素材库', () => {
  it('网格显示缩略图、尺寸；没有素材时显示空状态', async () => {
    const { unmount } = render(<AssetsPage api={fakeApi([], [], [asset(1, { title: '主图' })])} t={t} />)
    const card = await screen.findByRole('button', { name: '主图' })
    const img = card.querySelector('img')
    expect(img?.getAttribute('src')).toBe('/api/prism/file?path=thumbs%2Fab%2F1.webp')
    expect(within(card).getByText('800 × 600')).toBeTruthy()
    expect(screen.getByText('共 1 个')).toBeTruthy()
    unmount()

    render(<AssetsPage api={fakeApi()} t={t} />)
    expect(await screen.findByText('还没有素材')).toBeTruthy()
  })

  it('导入图片：逐个上传，再一起导入；失败和重复的分别说明', async () => {
    const api = fakeApi([], [], [asset(1, { title: '已有的图' })])
    render(<AssetsPage api={api} t={t} />)
    await screen.findByText('共 1 个')
    pick(screen.getByLabelText('导入图片'), png('主图-红.png'), png('坏图.png'), png('已有的图.png'))
    expect(await screen.findByText('导入了 2 张图片')).toBeTruthy()
    expect(screen.getByText('其中 1 张库里已经有了，没有重复保存')).toBeTruthy()
    expect(screen.getByText('1 个文件没有导入')).toBeTruthy()
    expect(screen.getByText('坏图.png')).toBeTruthy()
    expect(api.upload).toHaveBeenCalledTimes(3)
    expect(api.importAssetFiles).toHaveBeenCalledTimes(1)
    expect(await screen.findByText('共 2 个')).toBeTruthy()
  })

  it('卡片上收藏；按标签和只看收藏筛选；数量多时加载更多', async () => {
    const many = Array.from({ length: 70 }, (_, i) => asset(i, { tags: i < 3 ? ['主图'] : [] }))
    const api = fakeApi([], [], many)
    render(<AssetsPage api={api} t={t} />)
    expect(await screen.findByText('共 70 个')).toBeTruthy()
    expect(screen.getAllByRole('button', { name: /^图片 / })).toHaveLength(60)
    fireEvent.click(screen.getByRole('button', { name: '加载更多' }))
    await waitFor(() => { expect(screen.getAllByRole('button', { name: /^图片 / })).toHaveLength(70) })

    const card = screen.getByRole('button', { name: '图片 5' }).parentElement as HTMLElement
    fireEvent.click(within(card).getByRole('button', { name: '收藏' }))
    await waitFor(() => { expect(api.starAsset).toHaveBeenCalledWith('a5', true) })
    expect(within(card).getByRole('button', { name: '取消收藏' })).toBeTruthy()

    fireEvent.click(screen.getByRole('checkbox', { name: '只看收藏' }))
    expect(await screen.findByText('共 1 个')).toBeTruthy()
    fireEvent.click(screen.getByRole('checkbox', { name: '只看收藏' }))
    await screen.findByText('共 70 个')
    await choose(screen.getByRole('button', { name: '标签' }), '主图（3）')
    expect(await screen.findByText('共 3 个')).toBeTruthy()
  })

  it('详情面板：保存标签、关联商品、解除关联、删除', async () => {
    const api = fakeApi([product(1, { name: '亚麻衬衫', code: 'NEW-001' })], [], [asset(1, { title: '主图' })])
    render(<AssetsPage api={api} t={t} />)
    fireEvent.click(await screen.findByRole('button', { name: '主图' }))
    const panel = await screen.findByRole('complementary', { name: '主图' })
    expect(within(panel).getByRole('img', { name: '主图' }).getAttribute('src')).toBe('/api/prism/file?path=assets%2Fab%2F1.png')
    expect(within(panel).getByText('没有关联商品')).toBeTruthy()

    fireEvent.change(within(panel).getByLabelText('标签'), { target: { value: '红色， 主图,主图' } })
    fireEvent.click(within(panel).getByRole('button', { name: '保存标签' }))
    await waitFor(() => { expect(api.setAssetTags).toHaveBeenCalledWith('a1', ['红色', '主图']) })

    fireEvent.click(within(panel).getByRole('button', { name: '关联到商品' }))
    fireEvent.change(within(panel).getByLabelText('搜索商品名称或货号'), { target: { value: '亚麻' } })
    fireEvent.click(await within(panel).findByRole('button', { name: /亚麻衬衫/ }))
    await waitFor(() => { expect(api.linkAsset).toHaveBeenCalledWith('a1', 'p1') })
    expect(await within(panel).findByText('亚麻衬衫 · NEW-001')).toBeTruthy()
    fireEvent.click(within(panel).getByRole('button', { name: '解除关联' }))
    await waitFor(() => { expect(api.linkAsset).toHaveBeenLastCalledWith('a1', null) })
    expect(await within(panel).findByText('没有关联商品')).toBeTruthy()

    fireEvent.click(within(panel).getByRole('button', { name: '删除' }))
    const dialog = screen.getByRole('dialog', { name: '删除这个素材？' })
    fireEvent.click(within(dialog).getByRole('button', { name: '删除' }))
    await waitFor(() => { expect(api.deleteAsset).toHaveBeenCalledWith('a1') })
    expect(await screen.findByText('还没有素材')).toBeTruthy()
    expect(screen.queryByRole('complementary')).toBeNull()
  })
})
