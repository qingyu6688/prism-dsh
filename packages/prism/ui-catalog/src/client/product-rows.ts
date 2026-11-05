/**
 * 商品列表的分页加载。商品库按 1 万条设计：列表是虚拟滚动的，只按需取可见范围所在的页（每页 200 条），
 * 取过的页留在内存里；换筛选条件、改了商品之后清空重取。
 * @module @prism/dsh-client-ui-catalog/client/product-rows
 */

import type { Product, ProductFilter, ProductPage } from '@prism/dsh-host-catalog/types'
import { messageOf } from './api.ts'

/** 每次取的条数，等于服务端的上限。 */
export const PAGE_SIZE = 200

/** 列表的状态。`version` 每次变化加一，组件据此重新渲染。 */
export interface RowsSnapshot {
  readonly status: 'loading' | 'ready' | 'error'
  readonly total: number
  /** 第一页取失败时的说明；后面的页失败时列表保留，滚回来会重试。 */
  readonly error: string | null
  readonly version: number
}

/** 列表的筛选条件。 */
export interface RowsFilter {
  readonly keyword: string
  readonly archived: boolean
}

/** 按需分页加载商品。 */
export class ProductRows {
  private filter: RowsFilter = { keyword: '', archived: false }
  /** 换了筛选条件后，之前发出的请求回来也不再采用。 */
  private generation = 0
  private readonly pages = new Map<number, 'loading' | 'done'>()
  private items: (Product | undefined)[] = []
  private current: RowsSnapshot = { status: 'loading', total: 0, error: null, version: 0 }
  private readonly listeners = new Set<() => void>()

  /**
   * @param load - 取一页商品。
   */
  constructor(private readonly load: (filter: ProductFilter) => Promise<ProductPage>) {}

  /** 给 `useSyncExternalStore` 用。 */
  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  /** 给 `useSyncExternalStore` 用：同一个版本返回同一个对象。 */
  readonly snapshot = (): RowsSnapshot => this.current

  /**
   * 第几行的商品；还没取到时是 `undefined`。
   * @param index - 行号，从 0 开始。
   * @returns 商品或 `undefined`。
   */
  get(index: number): Product | undefined {
    return this.items[index]
  }

  /**
   * 换筛选条件，从第一页重新取。
   * @param filter - 关键词和在用 / 已归档。
   */
  setFilter(filter: RowsFilter): void {
    this.filter = filter
    this.reload()
  }

  /** 清空缓存重新取（新建、修改、删除商品之后）。 */
  reload(): void {
    this.generation++
    this.pages.clear()
    this.items = []
    this.publish({ status: 'loading', total: this.current.total, error: null })
    void this.fetchPage(0)
  }

  /**
   * 确保这段行号范围所在的页都取了。
   * @param start - 第一行。
   * @param end - 最后一行（含）。
   */
  ensure(start: number, end: number): void {
    if (this.current.status !== 'ready') return
    const last = Math.min(end, this.current.total - 1)
    for (let page = Math.floor(Math.max(start, 0) / PAGE_SIZE); page <= Math.floor(last / PAGE_SIZE); page++) {
      if (!this.pages.has(page)) void this.fetchPage(page)
    }
  }

  private async fetchPage(page: number): Promise<void> {
    const generation = this.generation
    this.pages.set(page, 'loading')
    let result: ProductPage
    try {
      result = await this.load({
        keyword: this.filter.keyword.trim() === '' ? null : this.filter.keyword.trim(),
        archived: this.filter.archived,
        limit: PAGE_SIZE,
        offset: page * PAGE_SIZE,
      })
    } catch (error) {
      if (generation !== this.generation) return
      this.pages.delete(page)
      if (page === 0) this.publish({ status: 'error', total: 0, error: messageOf(error) })
      return
    }
    if (generation !== this.generation) return
    this.pages.set(page, 'done')
    result.items.forEach((product, offset) => { this.items[page * PAGE_SIZE + offset] = product })
    this.publish({ status: 'ready', total: result.total, error: null })
  }

  private publish(next: Omit<RowsSnapshot, 'version'>): void {
    this.current = { ...next, version: this.current.version + 1 }
    for (const listener of [...this.listeners]) listener()
  }
}
