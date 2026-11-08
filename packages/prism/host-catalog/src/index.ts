/**
 * 商品库、规则包、搜索的 Remote 接口（`ctx.remote.prism.*`）和读写数据目录文件的路由。
 *
 * 每个方法都只是转调 `ctx.prismService`（prism-service 的 `catalog.*`、`rules.*`、`search.global`），
 * 业务规则都在 Rust 那边。服务返回的错误统一转成 `RemoteError('prism/failed')`，`details.code`
 * 是服务给的分类，`message` 是给用户看的中文说明。
 * @module @prism/dsh-host-catalog
 */

import { Context, Service } from '@deepseek-ai/cordis'
import type {} from '@deepseek-ai/dsh-client-connection'
import { Remote, RemoteError, TypertRemoteService } from '@deepseek-ai/dsh-typert-protocol'
import type { JsonValue } from '@deepseek-ai/dsh-util-values'
import { PrismServiceError } from '@prism/dsh-host-service'
import { FILE_PATH, UPLOAD_PATH, cleanInbox, receiveUpload, serveFile } from './files.ts'
import type { ImageProfileInput, ImageProfile, ImageBatch } from './types.ts'
import type {
  Asset,
  AssetFilter,
  AssetPage,
  IdRequest,
  ImportAssetFilesRequest,
  ImportAssetFilesResult,
  ImportedRule,
  ImportPreview,
  ImportPreviewRequest,
  ImportProductsRequest,
  ImportResult,
  Product,
  ProductDetail,
  ProductFilter,
  ProductInput,
  ProductPage,
  RuleKind,
  RulesOverview,
  SavedMapping,
  SearchResults,
  Shop,
  ShopInput,
  TagCount,
} from './types.ts'

export type * from './types.ts'
export { FILE_PATH, UPLOAD_PATH } from './files.ts'

declare module '@deepseek-ai/cordis' {
  interface Context {
    /** 商品库、规则包、搜索的 Remote 接口。 */
    prismCatalog: PrismCatalog
  }
}

/** `ctx.remote.prism.*`：界面调用棱镜业务服务的入口。 */
export class PrismCatalog extends TypertRemoteService {
  static inject = ['prismService', 'typert', 'connection']

  /**
   * @param ctx - 插件上下文，需要 `prismService`、`typert`、`connection`。
   */
  constructor(ctx: Context) {
    super(ctx, 'prismCatalog', { namespace: 'prism' })
  }

  protected [Service.init](): void {
    const ctx = this.ctx
    const dataDir = (): string | undefined => ctx.prismService.dataDir()
    ctx.effect(() => ctx.connection.fetch.register({
      path: FILE_PATH,
      methods: ['GET', 'HEAD'],
      requestBody: 'buffered',
      fetch: request => serveFile(dataDir(), request),
    }), 'prism-catalog: file route')
    ctx.effect(() => ctx.connection.fetch.register({
      path: UPLOAD_PATH,
      methods: ['POST'],
      requestBody: 'streaming',
      fetch: request => receiveUpload(dataDir(), request),
    }), 'prism-catalog: upload route')
    const dir = dataDir()
    if (dir !== undefined) {
      cleanInbox(dir, Date.now()).then(
        (removed) => { if (removed > 0) ctx.logger.info('清理了 %d 个过期的上传目录', removed) },
        (error: unknown) => { ctx.logger.warn('清理上传目录失败：%s', (error as Error).message) },
      )
    }
  }

  private async call<T>(method: string, params: object = {}): Promise<T> {
    try {
      return await this.ctx.prismService.call<T>(method, params)
    } catch (error) {
      if (error instanceof PrismServiceError) {
        const detail = (error.detail ?? null) as JsonValue
        throw new RemoteError('prism/failed', error.message, { code: error.code, detail }, { cause: error })
      }
      throw error
    }
  }

  /**
   * 列出不含密钥的生图配置。
   * @returns 配置及密钥是否存在。
   */
  @Remote
  async listImageProfiles(): Promise<readonly ImageProfile[]> {
    return await this.call('image.listProfiles', {})
  }

  /**
   * 保存配置并启用；编辑时省略密钥会保留旧值。
   * @param input - 配置字段及可选的新密钥。
   * @returns 保存后的配置，不含密钥。
   */
  @Remote
  async saveImageProfile(input: ImageProfileInput): Promise<ImageProfile> {
    return await this.call('image.saveProfile', input)
  }

  /**
   * 删除生图配置及系统凭据。
   * @param request - 生图配置 ID。
   */
  @Remote
  async deleteImageProfile(request: IdRequest): Promise<void> {
    await this.call('image.deleteProfile', request)
  }

  /**
   * 读取接口支持的模型，验证已保存的连接。
   * @param request - 生图配置 ID。
   * @returns 接口是否支持模型列表，以及图像模型名称。
   */
  @Remote
  async testImageProfile(request: IdRequest): Promise<{ readonly supported: boolean; readonly models: readonly string[] }> {
    return await this.call('image.testProfile', request)
  }

  /**
   * 列出最近 100 个批次中的生图任务。
   * @returns 批次状态和仍保留的素材。
   */
  @Remote
  async listImageBatches(): Promise<readonly ImageBatch[]> {
    return await this.call('image.listBatches', {})
  }

  /**
   * 查询批次进度和生成结果。
   * @param request - 生图批次 ID。
   * @returns 持久化状态及仍保留的素材。
   */
  @Remote
  async imageBatchStatus(request: IdRequest): Promise<ImageBatch> {
    return await this.call('image.batchStatus', request)
  }

  /**
   * 取消尚未完成的任务，保留已生成图片。
   * @param request - 生图批次 ID。
   * @returns 取消后的批次状态。
   */
  @Remote
  async cancelImageBatch(request: IdRequest): Promise<ImageBatch> {
    return await this.call('image.cancelBatch', request)
  }

  // ---------- 店铺 ----------

  /**
   * 店铺列表。
   * @param request - 按平台筛选，不填是全部。
   * @returns 店铺，按排序号排。
   */
  @Remote
  async listShops(request: { readonly platform?: string | null }): Promise<readonly Shop[]> {
    return await this.call('catalog.listShops', request)
  }

  /**
   * 新建店铺。
   * @param request - 店铺字段。
   * @returns 新店铺。
   */
  @Remote
  async createShop(request: { readonly input: ShopInput }): Promise<Shop> {
    return await this.call('catalog.createShop', request)
  }

  /**
   * 修改店铺。
   * @param request - 店铺 id 和全部字段。
   * @returns 改后的店铺。
   */
  @Remote
  async updateShop(request: { readonly id: string; readonly input: ShopInput }): Promise<Shop> {
    return await this.call('catalog.updateShop', request)
  }

  /**
   * 删除店铺。
   * @param request - 店铺 id。
   */
  @Remote
  async deleteShop(request: IdRequest): Promise<void> {
    await this.call('catalog.deleteShop', request)
  }

  // ---------- 商品 ----------

  /**
   * 商品列表，按最近更新排。
   * @param request - 关键词、在用 / 已归档、分页。
   * @returns 一页商品和总数。
   */
  @Remote
  async listProducts(request: { readonly filter: ProductFilter }): Promise<ProductPage> {
    return await this.call('catalog.listProducts', request)
  }

  /**
   * 商品详情。
   * @param request - 商品 id。
   * @returns 商品、规格、素材和文案汇总。
   */
  @Remote
  async productDetail(request: IdRequest): Promise<ProductDetail> {
    return await this.call('catalog.productDetail', request)
  }

  /**
   * 新建商品。
   * @param request - 商品字段。
   * @returns 新商品。
   */
  @Remote
  async createProduct(request: { readonly input: ProductInput }): Promise<Product> {
    return await this.call('catalog.createProduct', request)
  }

  /**
   * 修改商品。
   * @param request - 商品 id 和全部字段。
   * @returns 改后的商品。
   */
  @Remote
  async updateProduct(request: { readonly id: string; readonly input: ProductInput }): Promise<Product> {
    return await this.call('catalog.updateProduct', request)
  }

  /**
   * 归档商品：不在列表里显示，数据保留。
   * @param request - 商品 id。
   */
  @Remote
  async archiveProduct(request: IdRequest): Promise<void> {
    await this.call('catalog.archiveProduct', request)
  }

  /**
   * 把归档的商品恢复成在用。
   * @param request - 商品 id。
   */
  @Remote
  async restoreProduct(request: IdRequest): Promise<void> {
    await this.call('catalog.restoreProduct', request)
  }

  /**
   * 删除商品（连同规格和文案；关联的素材保留，解除关联）。
   * @param request - 商品 id。
   */
  @Remote
  async deleteProduct(request: IdRequest): Promise<void> {
    await this.call('catalog.deleteProduct', request)
  }

  // ---------- 从表格导入商品 ----------

  /**
   * 读表格，给出建议的映射。
   * @param request - 上传后 Host 上的路径、工作表、表头所在行。
   * @returns 表头、前 5 行、可导入的字段、建议的映射。
   */
  @Remote
  async previewProductImport(request: ImportPreviewRequest): Promise<ImportPreview> {
    return await this.call('catalog.previewProductImport', request)
  }

  /**
   * 按映射导入商品。
   * @param request - 路径、映射、导入方式、是否保存映射。
   * @returns 新建、更新、跳过的数量和问题行。
   */
  @Remote
  async importProducts(request: ImportProductsRequest): Promise<ImportResult> {
    return await this.call('catalog.importProducts', request)
  }

  /**
   * 保存过的导入映射。
   * @returns 映射，常用的在前。
   */
  @Remote
  async listImportMappings(): Promise<readonly SavedMapping[]> {
    return await this.call('catalog.listImportMappings')
  }

  /**
   * 删除一条保存的映射。
   * @param request - 映射 id。
   */
  @Remote
  async deleteImportMapping(request: IdRequest): Promise<void> {
    await this.call('catalog.deleteImportMapping', request)
  }

  // ---------- 素材 ----------

  /**
   * 素材列表，按最近更新排。
   * @param request - 商品、类型、来源、标签、关键词、只看收藏、分页。
   * @returns 一页素材和总数。
   */
  @Remote
  async listAssets(request: { readonly filter: AssetFilter }): Promise<AssetPage> {
    return await this.call('catalog.listAssets', request)
  }

  /**
   * 用过的标签和各自的素材数。
   * @returns 标签，用得多的在前。
   */
  @Remote
  async listAssetTags(): Promise<readonly TagCount[]> {
    return await this.call('catalog.listAssetTags')
  }

  /**
   * 导入图片文件，一个文件失败不影响其他文件。
   * @param request - 上传后 Host 上的路径、关联的商品、标签。
   * @returns 导入成功的素材和失败的文件。
   */
  @Remote
  async importAssetFiles(request: ImportAssetFilesRequest): Promise<ImportAssetFilesResult> {
    return await this.call('catalog.importAssetFiles', request)
  }

  /**
   * 设置素材的标签（整体替换）。
   * @param request - 素材 id 和新的标签。
   * @returns 改后的素材。
   */
  @Remote
  async setAssetTags(request: { readonly id: string; readonly tags: readonly string[] }): Promise<Asset> {
    return await this.call('catalog.setAssetTags', request)
  }

  /**
   * 收藏或取消收藏素材。
   * @param request - 素材 id 和是否收藏。
   */
  @Remote
  async starAsset(request: { readonly id: string; readonly starred: boolean }): Promise<void> {
    await this.call('catalog.starAsset', request)
  }

  /**
   * 把素材关联到商品，`productId` 为空时解除关联。
   * @param request - 素材 id 和商品 id。
   */
  @Remote
  async linkAsset(request: { readonly id: string; readonly productId: string | null }): Promise<void> {
    await this.call('catalog.linkAsset', request)
  }

  /**
   * 删除素材（连同文件）。
   * @param request - 素材 id。
   */
  @Remote
  async deleteAsset(request: IdRequest): Promise<void> {
    await this.call('catalog.deleteAsset', request)
  }

  // ---------- 规则包 ----------

  /**
   * 当前生效的规则包、词表、导出模板，以及加载失败的文件。
   * @returns 规则总览。
   */
  @Remote
  async listRules(): Promise<RulesOverview> {
    return await this.call('rules.list')
  }

  /**
   * 导入规则文件：校验通过、修订号不比当前的旧才写入。
   * @param request - 上传后 Host 上的路径。
   * @returns 导入的文件类型、id、修订号。
   */
  @Remote
  async importRuleFile(request: { readonly path: string }): Promise<ImportedRule> {
    return await this.call('rules.importFile', request)
  }

  /**
   * 删除用户导入的版本，回到内置版本。
   * @param request - 规则类型和 id。
   * @returns 有没有删掉文件。
   */
  @Remote
  async removeUserRule(request: { readonly kind: RuleKind; readonly id: string }): Promise<{ readonly removed: boolean }> {
    return await this.call('rules.removeUser', request)
  }

  // ---------- 搜索 ----------

  /**
   * 全局搜索：商品、素材各取前 5 条，数字或 #序号匹配生图批次。
   * @param request - 关键词；去掉空格后为空时返回空结果。
   * @returns 商品、素材和匹配的生图批次。
   */
  @Remote
  async search(request: { readonly keyword: string }): Promise<SearchResults> {
    return await this.call('search.global', request)
  }
}

export default PrismCatalog
