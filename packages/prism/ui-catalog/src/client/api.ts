/**
 * 界面调用商品库接口的封装：把 `RemoteResult` 拆开，失败时抛 {@link CatalogRequestError}，
 * 组件只处理"成功的值"和"一句给用户看的说明"。
 * @module @prism/dsh-client-ui-catalog/client/api
 */

import type {
  Asset, AssetFilter, AssetPage, IdRequest, ImportAssetFilesRequest, ImportAssetFilesResult, ImportPreview,
  ImportPreviewRequest, ImportProductsRequest, ImportResult, Product, ProductDetail, ProductFilter, ProductInput,
  ProductPage, RuleKind, RulesOverview, SavedMapping, SearchResults, Shop, ShopInput, TagCount, UploadedFile, ImportedRule,
} from '@prism/dsh-host-catalog/types'
import type { RemoteResult } from '@deepseek-ai/dsh-typert-protocol'

/** 请求失败。`message` 可以直接显示，`code` 是业务服务给的分类（如 `not_found`）。 */
export class CatalogRequestError extends Error {
  override readonly name = 'CatalogRequestError'

  /**
   * @param message - 给用户看的说明。
   * @param code - 失败分类。
   * @param detail - 服务给的细节，如规则文件的问题列表；没有时为 `null`。
   */
  constructor(message: string, readonly code: string, readonly detail: unknown = null) {
    super(message)
  }
}

/** 生成的 `ctx.remote.prism` 里本包用到的方法。测试用替身实现它。 */
export interface PrismRemote {
  listProducts(request: { readonly filter: ProductFilter }): Promise<RemoteResult<ProductPage>>
  productDetail(request: IdRequest): Promise<RemoteResult<ProductDetail>>
  createProduct(request: { readonly input: ProductInput }): Promise<RemoteResult<Product>>
  updateProduct(request: { readonly id: string; readonly input: ProductInput }): Promise<RemoteResult<Product>>
  archiveProduct(request: IdRequest): Promise<RemoteResult<void>>
  restoreProduct(request: IdRequest): Promise<RemoteResult<void>>
  deleteProduct(request: IdRequest): Promise<RemoteResult<void>>
  listShops(request: { readonly platform?: string | null }): Promise<RemoteResult<readonly Shop[]>>
  createShop(request: { readonly input: ShopInput }): Promise<RemoteResult<Shop>>
  updateShop(request: { readonly id: string; readonly input: ShopInput }): Promise<RemoteResult<Shop>>
  deleteShop(request: IdRequest): Promise<RemoteResult<void>>
  previewProductImport(request: ImportPreviewRequest): Promise<RemoteResult<ImportPreview>>
  importProducts(request: ImportProductsRequest): Promise<RemoteResult<ImportResult>>
  listImportMappings(): Promise<RemoteResult<readonly SavedMapping[]>>
  deleteImportMapping(request: IdRequest): Promise<RemoteResult<void>>
  listAssets(request: { readonly filter: AssetFilter }): Promise<RemoteResult<AssetPage>>
  listAssetTags(): Promise<RemoteResult<readonly TagCount[]>>
  importAssetFiles(request: ImportAssetFilesRequest): Promise<RemoteResult<ImportAssetFilesResult>>
  setAssetTags(request: { readonly id: string; readonly tags: readonly string[] }): Promise<RemoteResult<Asset>>
  starAsset(request: { readonly id: string; readonly starred: boolean }): Promise<RemoteResult<void>>
  linkAsset(request: { readonly id: string; readonly productId: string | null }): Promise<RemoteResult<void>>
  deleteAsset(request: IdRequest): Promise<RemoteResult<void>>
  listRules(): Promise<RemoteResult<RulesOverview>>
  importRuleFile(request: { readonly path: string }): Promise<RemoteResult<ImportedRule>>
  removeUserRule(request: { readonly kind: RuleKind; readonly id: string }): Promise<RemoteResult<{ readonly removed: boolean }>>
  search(request: { readonly keyword: string }): Promise<RemoteResult<SearchResults>>
}

/** 组件用的接口：成功返回值，失败抛 {@link CatalogRequestError}。 */
export interface CatalogApi {
  listProducts(filter: ProductFilter): Promise<ProductPage>
  productDetail(id: string): Promise<ProductDetail>
  createProduct(input: ProductInput): Promise<Product>
  updateProduct(id: string, input: ProductInput): Promise<Product>
  archiveProduct(id: string): Promise<void>
  restoreProduct(id: string): Promise<void>
  deleteProduct(id: string): Promise<void>
  listShops(): Promise<readonly Shop[]>
  createShop(input: ShopInput): Promise<Shop>
  updateShop(id: string, input: ShopInput): Promise<Shop>
  deleteShop(id: string): Promise<void>
  /** 把浏览器里选的文件传到 Host，返回 Host 上的路径（导入前先上传）。 */
  upload(file: File, signal?: AbortSignal): Promise<UploadedFile>
  previewProductImport(request: ImportPreviewRequest): Promise<ImportPreview>
  importProducts(request: ImportProductsRequest): Promise<ImportResult>
  listImportMappings(): Promise<readonly SavedMapping[]>
  deleteImportMapping(id: string): Promise<void>
  listAssets(filter: AssetFilter): Promise<AssetPage>
  listAssetTags(): Promise<readonly TagCount[]>
  importAssetFiles(request: ImportAssetFilesRequest): Promise<ImportAssetFilesResult>
  setAssetTags(id: string, tags: readonly string[]): Promise<Asset>
  starAsset(id: string, starred: boolean): Promise<void>
  linkAsset(id: string, productId: string | null): Promise<void>
  deleteAsset(id: string): Promise<void>
  listRules(): Promise<RulesOverview>
  /** 导入上传到 Host 的规则文件。校验不通过时错误的 `detail` 是问题列表 `[{ path, message }]`。 */
  importRuleFile(path: string): Promise<ImportedRule>
  /** 删除用户导入的版本，回到内置版本。 */
  removeUserRule(kind: RuleKind, id: string): Promise<boolean>
  /** 全局搜索：商品、素材各取前 5 条。 */
  search(keyword: string): Promise<SearchResults>
}

/** 上传路由，和 `@prism/dsh-host-catalog` 的 `UPLOAD_PATH` 一致（那个包是 Host 端，浏览器里不能引它的值）。 */
export const UPLOAD_PATH = '/api/prism/upload'
/** 读数据目录图片的路由，和 `@prism/dsh-host-catalog` 的 `FILE_PATH` 一致。 */
export const FILE_PATH = '/api/prism/file'

/**
 * 数据目录里一个文件的地址，给 `<img src>` 用。
 * @param relativePath - 素材的 `filePath` 或 `thumbPath`。
 * @returns 地址。
 */
export function fileUrl(relativePath: string): string {
  return `${FILE_PATH}?path=${encodeURIComponent(relativePath)}`
}

/**
 * 上传一个文件到 Host。失败时用路由返回的中文说明。
 * @param fetchImpl - 发请求的函数（测试时替换）。
 * @param file - 浏览器里选的文件。
 * @param unreachable - 连不上 Host 时的说明。
 * @param signal - 取消上传。
 * @returns Host 上的路径、文件名、大小。
 */
async function uploadFile(fetchImpl: typeof fetch, file: File, unreachable: string, signal?: AbortSignal): Promise<UploadedFile> {
  let response: Response
  try {
    response = await fetchImpl(`${UPLOAD_PATH}?name=${encodeURIComponent(file.name)}`, {
      method: 'POST',
      body: file,
      credentials: 'same-origin',
      ...signal === undefined ? {} : { signal },
    })
  } catch (error) {
    if (signal?.aborted === true) throw error
    throw new CatalogRequestError(unreachable, 'unreachable')
  }
  const body: unknown = await response.json().catch(() => null)
  if (response.ok) return body as UploadedFile
  const failure = (body ?? {}) as { code?: unknown; message?: unknown }
  throw new CatalogRequestError(
    typeof failure.message === 'string' ? failure.message : `${unreachable}（HTTP ${response.status}）`,
    typeof failure.code === 'string' ? failure.code : `http_${response.status}`,
  )
}

/**
 * 拆开一次 Remote 调用的结果。
 * @param pending - Remote 调用。
 * @param unreachable - 连不上 Host（不是业务服务返回的错误）时显示的说明。
 * @returns 成功的值。
 * @throws {CatalogRequestError} 调用失败。
 */
export async function unwrap<T>(pending: Promise<RemoteResult<T>>, unreachable: string): Promise<T> {
  const result = await pending
  if (result.ok) return result.value
  const { error } = result
  if (error.code === 'prism/failed') throw new CatalogRequestError(error.message, error.details.code, error.details.detail)
  // 网关、连接层的错误信息是英文的技术描述，换成一句能看懂的话，原始码留着排查
  throw new CatalogRequestError(`${unreachable}（${error.code}）`, error.code)
}

/**
 * 包一层：每个方法都拆开结果。
 * @param remote - 生成的 `ctx.remote.prism`。
 * @param unreachable - 连不上 Host 时的说明（已按界面语言翻译）。
 * @returns 组件用的接口。
 */
export function createCatalogApi(
  remote: PrismRemote,
  unreachable: string,
  fetchImpl: typeof fetch = (input, init) => globalThis.fetch(input, init),
): CatalogApi {
  const run = <T>(pending: Promise<RemoteResult<T>>): Promise<T> => unwrap(pending, unreachable)
  return {
    listProducts: filter => run(remote.listProducts({ filter })),
    productDetail: id => run(remote.productDetail({ id })),
    createProduct: input => run(remote.createProduct({ input })),
    updateProduct: (id, input) => run(remote.updateProduct({ id, input })),
    archiveProduct: id => run(remote.archiveProduct({ id })),
    restoreProduct: id => run(remote.restoreProduct({ id })),
    deleteProduct: id => run(remote.deleteProduct({ id })),
    listShops: () => run(remote.listShops({})),
    createShop: input => run(remote.createShop({ input })),
    updateShop: (id, input) => run(remote.updateShop({ id, input })),
    deleteShop: id => run(remote.deleteShop({ id })),
    upload: (file, signal) => uploadFile(fetchImpl, file, unreachable, signal),
    previewProductImport: request => run(remote.previewProductImport(request)),
    importProducts: request => run(remote.importProducts(request)),
    listImportMappings: () => run(remote.listImportMappings()),
    deleteImportMapping: id => run(remote.deleteImportMapping({ id })),
    listAssets: filter => run(remote.listAssets({ filter })),
    listAssetTags: () => run(remote.listAssetTags()),
    importAssetFiles: request => run(remote.importAssetFiles(request)),
    setAssetTags: (id, tags) => run(remote.setAssetTags({ id, tags })),
    starAsset: (id, starred) => run(remote.starAsset({ id, starred })),
    linkAsset: (id, productId) => run(remote.linkAsset({ id, productId })),
    deleteAsset: id => run(remote.deleteAsset({ id })),
    listRules: () => run(remote.listRules()),
    importRuleFile: path => run(remote.importRuleFile({ path })),
    removeUserRule: async (kind, id) => (await run(remote.removeUserRule({ kind, id }))).removed,
    search: keyword => run(remote.search({ keyword })),
  }
}

/**
 * 取出错误里给用户看的说明。
 * @param error - 捕获到的错误。
 * @returns 说明文字。
 */
export function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
