/**
 * 输入框里 `@` 引用商品。
 *
 * - 输入 `@` 后的菜单里多一组"商品"，按名称、货号、类目、品牌搜索（和商品库的搜索一样）。
 * - 选中后插入一个原子引用，发给模型的文字是 `@[商品名称](prism-product:<商品 id>)`，模型可以直接拿 id
 *   调 `prism_get_product`（工具说明里写了这个写法）。
 * - 聊天记录里的这种写法显示成带包装盒图标的标签。
 * @module @prism/dsh-client-ui-catalog/client/product-reference
 */
import type { IconProps } from '@deepseek-ai/dsh-client-ui-primitives'
import type { UserTextNodeProps, UserTextRule } from '@deepseek-ai/dsh-client-ui-conversation/client'
import type { InputTriggerCandidate, InputTriggerSource } from '@deepseek-ai/dsh-client-ui-input-trigger/client'
import type { Product } from '@prism/dsh-host-catalog/types'
import type { CatalogApi } from './api.ts'
import type { Translate } from './ProductForm.tsx'
import css from './Catalog.module.css'

declare module '@deepseek-ai/dsh-client-ui-conversation/client' {
  interface UserTextDataMap {
    'prism-product': { readonly label: string; readonly id: string }
  }
}

/** 引用来源的名字，也是插入项的 `source`。 */
export const PRODUCT_SOURCE = 'prism-products'
/** 菜单里最多列几个商品。 */
const CANDIDATE_LIMIT = 8
const MENTION = /@\[([^\]\n]+)\]\(prism-product:([^)\s]+)\)/uy

/**
 * 商品的引用写法。名称里的方括号和换行换成空格，不然会截断引用。
 * @param product - 商品。
 * @returns `@[名称](prism-product:<id>)`。
 */
export function productMention(product: Pick<Product, 'id' | 'name'>): string {
  const label = product.name.replace(/[[\]\r\n]+/g, ' ').trim()
  return `@[${label}](prism-product:${encodeURIComponent(product.id)})`
}

/**
 * 从引用写法里取回名称和 id。
 * @param mention - 引用写法。
 * @returns 名称和 id；不是商品引用时返回 `undefined`。
 */
export function parseProductMention(mention: string): { label: string; id: string } | undefined {
  MENTION.lastIndex = 0
  const match = MENTION.exec(mention)
  if (match === null || match[0].length !== mention.length) return undefined
  const [, label, id] = match
  return label === undefined || id === undefined ? undefined : { label, id: decodeURIComponent(id) }
}

/** 菜单行和标签里的包装盒图标，和左侧导航"商品库"的图标一致。 */
export function ProductGlyph({ size = 16, className }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5}
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" className={className}>
      <path d="M3.5 7.5 12 3l8.5 4.5v9L12 21l-8.5-4.5z" />
      <path d="M3.5 7.5 12 12l8.5-4.5M12 12v9" />
    </svg>
  )
}

/**
 * `@` 菜单里的"商品"一组。
 * @param api - 商品库接口。
 * @param t - 翻译函数。
 * @returns 引用来源。
 */
export function createProductSource(api: CatalogApi, t: Translate): InputTriggerSource {
  return {
    trigger: '@',
    name: PRODUCT_SOURCE,
    // 电商场景里商品比文件常用，排在文件、会话前面
    order: -1,
    showGroupTitle: false,
    async candidates(_session, { query, quoted, signal }): Promise<readonly InputTriggerCandidate[]> {
      // `@"…` 是带引号的文件路径，不查商品
      if (quoted === true) return []
      const keyword = query.trim()
      const page = await api.listProducts({ keyword: keyword === '' ? null : keyword, limit: CANDIDATE_LIMIT })
      if (signal.aborted) return []
      return page.items.map(product => ({
        name: product.name,
        label: product.name,
        description: [product.code, product.category].filter(value => value !== null && value !== '').join(' · '),
        icon: ProductGlyph,
        section: t('reference.section'),
        value: productMention(product),
      }))
    },
    onPick({ candidate }) {
      const parsed = candidate.value === undefined ? undefined : parseProductMention(candidate.value)
      if (parsed === undefined || candidate.value === undefined) return undefined
      return {
        insert: { source: PRODUCT_SOURCE, ref: candidate.value, label: parsed.label, clipboardText: candidate.value },
      }
    },
    codec: {
      clipboardText: ref => ref,
      serialize: ref => Promise.resolve(ref),
    },
  }
}

/** 在用户消息里找 `@[名称](prism-product:<id>)`。 */
export const productReferenceRule: UserTextRule<'prism-product'> = {
  id: 'prism-product',
  // 排在会话引用（10）后面；两者写法不同，不会抢同一段文字
  order: 20,
  match(text, from) {
    for (let start = text.indexOf('@[', from); start >= 0; start = text.indexOf('@[', start + 1)) {
      MENTION.lastIndex = start
      const match = MENTION.exec(text)
      if (match === null) continue
      const [, label, id] = match
      if (label === undefined || id === undefined) continue
      return { id: 'prism-product', start, end: start + match[0].length, data: { label, id: decodeURIComponent(id) } }
    }
    return undefined
  },
}

/**
 * 聊天记录里的商品引用标签。
 * @param props - 识别出的引用和原文。
 * @returns 行内标签。
 */
export function ProductReference({ node, raw }: Pick<UserTextNodeProps<'prism-product'>, 'node' | 'raw'>) {
  return (
    <span className={css.productChip} title={raw} data-ref-chip="prism-product">
      <ProductGlyph size={14} className={css.productChipIcon} />
      {node.data.label}
    </span>
  )
}
