/**
 * 商品库界面内部的跳转：全局搜索框开关、从搜索结果跳到商品详情或按关键词看素材。
 * 状态挂在插件实例上（不是模块级单例），插件卸载时随之消失。
 * @module @prism/dsh-client-ui-catalog/client/navigator
 */

/** 一次跳转请求。`seq` 每次加一，页面据此判断是不是新请求（同一个商品点两次也要响应）。 */
export interface FocusRequest<T> {
  readonly value: T
  readonly seq: number
}

/** 搜索开关及当前请求打开的业务页面。 */
export interface NavigatorState {
  readonly imageBatch: string | undefined
  readonly searchOpen: boolean
  /** 要在商品库里打开详情的商品 id。 */
  readonly product: FocusRequest<string> | null
  /** 要在素材库里搜索的关键词。 */
  readonly assetKeyword: FocusRequest<string> | null
}

/** 商品库界面的跳转状态。 */
export class CatalogNavigator {
  private current: NavigatorState = { imageBatch: undefined, searchOpen: false, product: null, assetKeyword: null }
  private seq = 0
  private readonly listeners = new Set<() => void>()

  /**
   * @param selectPanel - 切换主区域页面（`ctx.layout.selectPanel`）。
   * @param panels - 商品库、素材库页面的 id。
   */
  constructor(
    private readonly selectPanel: (panel: string) => void,
    private readonly panels: { readonly products: string; readonly assets: string },
  ) {}

  /** 给 `useSyncExternalStore` 用。 */
  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  /** 给 `useSyncExternalStore` 用：同一状态返回同一个对象。 */
  readonly snapshot = (): NavigatorState => this.current

  /** 打开全局业务搜索。 */
  openSearch(): void {
    this.set({ searchOpen: true })
  }

  /** 关闭全局业务搜索。 */
  closeSearch(): void {
    this.set({ searchOpen: false })
  }

  /**
   * 切到商品库并打开这个商品的详情。
   * @param id - 商品 id。
   */
  showProduct(id: string): void {
    this.set({ searchOpen: false, product: { value: id, seq: ++this.seq } })
    this.selectPanel(this.panels.products)
  }

  /**
   * 切到素材库并按关键词搜索。
   * @param keyword - 关键词。
   */
  showAssets(keyword: string): void {
    this.set({ searchOpen: false, assetKeyword: { value: keyword, seq: ++this.seq } })
    this.selectPanel(this.panels.assets)
  }

  /**
   * 切到生图结果，可定位已保存的批次。
   * @param id - 目标生图批次；不传时显示全部批次。
   */
  showImageBatch(id?: string): void {
    this.set({ searchOpen: false, imageBatch: id })
    this.selectPanel('prism-images')
  }

  private set(patch: Partial<NavigatorState>): void {
    this.current = { ...this.current, ...patch }
    for (const listener of [...this.listeners]) listener()
  }
}
