/**
 * 组件测试共用的假接口：商品、店铺、素材、表格导入都放在内存里，行为和服务端一致的部分
 * （货号唯一、归档、按关键词 / 标签 / 收藏筛选、图片去重）照着做，其余返回固定值。
 */
import { vi, type Mock } from 'vitest'
import type {
  Asset, ImportPreview, ImportProductsRequest, ImportResult, Product, ProductDetail, ProductInput, RulesOverview, Shop, ShopInput,
} from '@prism/dsh-host-catalog/types'
import { CatalogRequestError, type CatalogApi } from '../src/client/api.ts'

/** 每个方法都是带类型的 mock，测试里可以检查调用、临时改返回值。 */
export type FakeApi = { [K in keyof CatalogApi]: Mock<CatalogApi[K]> } & {
  products: Product[]
  shops: Shop[]
  assets: Asset[]
  /** 最近一次导入商品的请求。 */
  lastImport: ImportProductsRequest | undefined
  rules: RulesOverview
}

/** 规则总览：内置的淘宝规则包、用户导入盖过内置的亚马逊规则包、一个词表、一个加载失败的文件。 */
export const RULES: RulesOverview = {
  packs: [
    {
      origin: { type: 'user', path: 'D:/data/rules/packs/amazon.json' },
      shadowedRevision: 1,
      value: {
        id: 'amazon', revision: 2, name: { 'zh-CN': '亚马逊', en: 'Amazon' }, status: 'verified', market: 'cross_border',
        text: { title: { maxLength: 200, unit: 'char' } },
        images: { main: { minSide: 1000, aspectRatios: [], whiteBackground: 'first' } },
      },
    },
    {
      origin: { type: 'builtin' },
      shadowedRevision: null,
      value: {
        id: 'taobao', revision: 1, name: { 'zh-CN': '淘宝 / 天猫', en: 'Taobao' }, status: 'draft', market: 'domestic',
        text: { title: { maxLength: 60, unit: 'cjk_double' } },
        images: { main: { minSide: 800, aspectRatios: ['1:1', '3:4'], whiteBackground: 'none' } },
      },
    },
  ],
  wordLists: [{
    origin: { type: 'builtin' }, shadowedRevision: null,
    value: { id: 'ad_superlatives_zh', revision: 1, name: { 'zh-CN': '广告法极限词' }, status: 'draft' },
  }],
  exportTemplates: [],
  problems: [{ file: 'D:/data/rules/packs/broken.json', problems: [{ path: 'formatVersion', message: '缺少必填字段 formatVersion' }] }],
}

/** 预览固定返回的表格：表头"款号、品名、采购价"，3 行。 */
export const PREVIEW: ImportPreview = {
  sheetNames: [],
  sheet: '',
  headerLine: 1,
  headers: ['款号', '品名', '采购价'],
  sample: [{ line: 2, cells: ['NEW-001', '亚麻衬衫', 59.9] }],
  totalRows: 3,
  fields: [
    { key: 'product.name', label: '商品名称', aliases: [], required: true },
    { key: 'product.code', label: '货号', aliases: [], required: false },
    { key: 'product.cost', label: '成本', aliases: [], required: false },
  ],
  mapping: { columns: { 'product.name': '品名', 'product.code': '款号' } },
  matched: null,
}

/** 导入固定返回的结果：新建 1、更新 1、跳过 1。 */
export const IMPORT_RESULT: ImportResult = {
  created: 1,
  updated: 1,
  skipped: 1,
  problems: [{ line: 3, field: '商品名称', message: '商品名称是必填的，这一行是空的' }],
  savedMappingId: null,
}

export function product(n: number, overrides: Partial<Product> = {}): Product {
  return {
    id: `p${n}`, code: `C-${n}`, name: `商品 ${n}`, category: '上衣', brand: null, description: null,
    sellingPoints: [], attributes: [], costCents: 1000 + n, currency: 'CNY', targetMarkets: [], archived: false,
    createdAt: 0, updatedAt: 0, ...overrides,
  }
}

export function asset(n: number, overrides: Partial<Asset> = {}): Asset {
  return {
    id: `a${n}`, kind: 'image', productId: null, source: 'upload', title: `图片 ${n}`,
    filePath: `assets/ab/${n}.png`, thumbPath: `thumbs/ab/${n}.webp`, textContent: null, mime: 'image/png',
    width: 800, height: 600, sha256: `hash-${n}`, phash: null, originBatchId: null, originJobId: null,
    starred: false, tags: [], createdAt: 0, updatedAt: 0, ...overrides,
  }
}

/**
 * 建一个假接口。
 * @param initial - 初始商品。
 * @param shops - 初始店铺。
 * @param assets - 初始素材。
 * @returns 假接口和它的内存状态。
 */
export function fakeApi(initial: Product[] = [], shops: Shop[] = [], assets: Asset[] = []): FakeApi {
  let next = 100
  const state = {
    products: [...initial], shops: [...shops], assets: [...assets], lastImport: undefined as ImportProductsRequest | undefined,
    rules: RULES,
  }
  const toProduct = (id: string, input: ProductInput, archived = false): Product => ({
    id, name: input.name, code: input.code ?? null, category: input.category ?? null, brand: input.brand ?? null,
    description: input.description ?? null, sellingPoints: [...(input.sellingPoints ?? [])], attributes: [...(input.attributes ?? [])],
    costCents: input.costCents ?? null, currency: input.currency ?? 'CNY', targetMarkets: [...(input.targetMarkets ?? [])],
    archived, createdAt: 0, updatedAt: 0,
  })
  const find = (id: string): Product => {
    const found = state.products.find(p => p.id === id)
    if (found === undefined) throw new CatalogRequestError('商品不存在', 'not_found')
    return found
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
  const patchAsset = (id: string, patch: Partial<Asset>): Asset => {
    const current = state.assets.find(a => a.id === id)
    if (current === undefined) throw new CatalogRequestError('素材不存在', 'not_found')
    const updated = { ...current, ...patch }
    state.assets = state.assets.map(a => a.id === id ? updated : a)
    return updated
  }
  return Object.assign(state, {
    listProducts: vi.fn<CatalogApi['listProducts']>((filter) => {
      const keyword = filter.keyword ?? ''
      const items = state.products.filter(p => p.archived === (filter.archived ?? false) && p.name.includes(keyword))
      const offset = filter.offset ?? 0
      return Promise.resolve({ items: items.slice(offset, offset + (filter.limit ?? 50)), total: items.length })
    }),
    productDetail: vi.fn<CatalogApi['productDetail']>((id): Promise<ProductDetail> => Promise.resolve({
      product: find(id), skus: [],
      summary: { skuCount: 0, imageAssets: 2, textAssets: 0, starredAssets: 0, copies: [], batches: [], lastBatchAt: null },
    })),
    createProduct: vi.fn<CatalogApi['createProduct']>((input) => {
      checkCode(input)
      const created = toProduct(`p${next++}`, input)
      state.products.unshift(created)
      return Promise.resolve(created)
    }),
    updateProduct: vi.fn<CatalogApi['updateProduct']>((id, input) => {
      checkCode(input, id)
      const updated = toProduct(id, input, find(id).archived)
      state.products = state.products.map(p => p.id === id ? updated : p)
      return Promise.resolve(updated)
    }),
    archiveProduct: vi.fn<CatalogApi['archiveProduct']>((id) => {
      state.products = state.products.map(p => p.id === id ? { ...p, archived: true } : p)
      return Promise.resolve()
    }),
    restoreProduct: vi.fn<CatalogApi['restoreProduct']>((id) => {
      state.products = state.products.map(p => p.id === id ? { ...p, archived: false } : p)
      return Promise.resolve()
    }),
    deleteProduct: vi.fn<CatalogApi['deleteProduct']>((id) => {
      state.products = state.products.filter(p => p.id !== id)
      return Promise.resolve()
    }),
    listShops: vi.fn<CatalogApi['listShops']>(() => Promise.resolve([...state.shops])),
    createShop: vi.fn<CatalogApi['createShop']>((input) => {
      const shop = toShop(`s${next++}`, input)
      state.shops.push(shop)
      return Promise.resolve(shop)
    }),
    updateShop: vi.fn<CatalogApi['updateShop']>((id, input) => {
      const shop = toShop(id, input)
      state.shops = state.shops.map(s => s.id === id ? shop : s)
      return Promise.resolve(shop)
    }),
    deleteShop: vi.fn<CatalogApi['deleteShop']>((id) => {
      state.shops = state.shops.filter(s => s.id !== id)
      return Promise.resolve()
    }),
    upload: vi.fn<CatalogApi['upload']>(file => Promise.resolve({ path: `D:/data/inbox/${next++}/${file.name}`, name: file.name, size: file.size })),
    previewProductImport: vi.fn<CatalogApi['previewProductImport']>(request => Promise.resolve({
      ...PREVIEW, headerLine: request.headerLine ?? PREVIEW.headerLine,
    })),
    importProducts: vi.fn<CatalogApi['importProducts']>((request) => {
      state.lastImport = request
      return Promise.resolve(IMPORT_RESULT)
    }),
    listImportMappings: vi.fn<CatalogApi['listImportMappings']>(() => Promise.resolve([])),
    deleteImportMapping: vi.fn<CatalogApi['deleteImportMapping']>(() => Promise.resolve()),
    listAssets: vi.fn<CatalogApi['listAssets']>((filter) => {
      const keyword = filter.keyword ?? ''
      const items = state.assets.filter(a => (a.title ?? '').includes(keyword)
        && (filter.tag == null || a.tags.includes(filter.tag))
        && (filter.starredOnly !== true || a.starred))
      const offset = filter.offset ?? 0
      return Promise.resolve({ items: items.slice(offset, offset + (filter.limit ?? 50)), total: items.length })
    }),
    listAssetTags: vi.fn<CatalogApi['listAssetTags']>(() => {
      const counts = new Map<string, number>()
      for (const a of state.assets) for (const tag of a.tags) counts.set(tag, (counts.get(tag) ?? 0) + 1)
      return Promise.resolve([...counts].map(([tag, count]) => ({ tag, count })))
    }),
    importAssetFiles: vi.fn<CatalogApi['importAssetFiles']>((request) => {
      const imported: { asset: Asset; deduplicated: boolean }[] = []
      const failed: { path: string; message: string }[] = []
      for (const path of request.paths) {
        if (path.endsWith('坏图.png')) {
          failed.push({ path, message: '不支持的图片格式，只支持 PNG、JPEG、WebP' })
          continue
        }
        const title = path.split('/').pop()?.replace(/\.[^.]+$/, '') ?? ''
        const existing = state.assets.find(a => a.title === title)
        if (existing !== undefined) {
          imported.push({ asset: existing, deduplicated: true })
          continue
        }
        const created = asset(next++, { title })
        state.assets.unshift(created)
        imported.push({ asset: created, deduplicated: false })
      }
      return Promise.resolve({ imported, failed })
    }),
    setAssetTags: vi.fn<CatalogApi['setAssetTags']>((id, tags) => Promise.resolve(patchAsset(id, { tags: [...tags].sort() }))),
    starAsset: vi.fn<CatalogApi['starAsset']>((id, starred) => { patchAsset(id, { starred }); return Promise.resolve() }),
    linkAsset: vi.fn<CatalogApi['linkAsset']>((id, productId) => { patchAsset(id, { productId }); return Promise.resolve() }),
    deleteAsset: vi.fn<CatalogApi['deleteAsset']>((id) => {
      state.assets = state.assets.filter(a => a.id !== id)
      return Promise.resolve()
    }),
    listRules: vi.fn<CatalogApi['listRules']>(() => Promise.resolve(state.rules)),
    importRuleFile: vi.fn<CatalogApi['importRuleFile']>(() => Promise.resolve({ kind: 'rule_pack', id: 'amazon', revision: 3 })),
    removeUserRule: vi.fn<CatalogApi['removeUserRule']>((kind, id) => {
      state.rules = { ...state.rules, packs: state.rules.packs.filter(row => !(kind === 'rule_pack' && row.value.id === id)) }
      return Promise.resolve(true)
    }),
    search: vi.fn<CatalogApi['search']>(keyword => Promise.resolve({
      products: state.products.filter(p => p.name.includes(keyword)).slice(0, 5),
      assets: state.assets.filter(a => (a.title ?? '').includes(keyword) || a.tags.includes(keyword)).slice(0, 5),
    })),
  })
}
