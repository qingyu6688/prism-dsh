/**
 * 界面调用商品库接口的封装：把 `RemoteResult` 拆开，失败时抛 {@link CatalogRequestError}，
 * 组件只处理"成功的值"和"一句给用户看的说明"。
 * @module @prism/dsh-client-ui-catalog/client/api
 */

import type {
  IdRequest, Product, ProductDetail, ProductFilter, ProductInput, ProductPage, Shop, ShopInput,
} from '@prism/dsh-host-catalog/types'
import type { RemoteResult } from '@deepseek-ai/dsh-typert-protocol'

/** 请求失败。`message` 可以直接显示，`code` 是业务服务给的分类（如 `not_found`）。 */
export class CatalogRequestError extends Error {
  override readonly name = 'CatalogRequestError'

  /**
   * @param message - 给用户看的说明。
   * @param code - 失败分类。
   */
  constructor(message: string, readonly code: string) {
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
  if (error.code === 'prism/failed') throw new CatalogRequestError(error.message, error.details.code)
  // 网关、连接层的错误信息是英文的技术描述，换成一句能看懂的话，原始码留着排查
  throw new CatalogRequestError(`${unreachable}（${error.code}）`, error.code)
}

/**
 * 包一层：每个方法都拆开结果。
 * @param remote - 生成的 `ctx.remote.prism`。
 * @param unreachable - 连不上 Host 时的说明（已按界面语言翻译）。
 * @returns 组件用的接口。
 */
export function createCatalogApi(remote: PrismRemote, unreachable: string): CatalogApi {
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
