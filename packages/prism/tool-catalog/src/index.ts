/**
 * 给 Agent 用的商品库查询工具：`prism_search_products`（按关键词查商品）、`prism_get_product`（商品详情）。
 * 只读，不需要审批。写操作（新建、修改商品）在界面上做，D3 不给工具。
 *
 * 工具通过 `ctx.prismService` 调棱镜业务服务，挂在棱镜的 Agent 预设里（`presets/prism.patch.yml`）。
 * @module @prism/dsh-tool-catalog
 */

import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import type { JsonValue } from '@deepseek-ai/dsh-util-values'
import type { Product, ProductDetail, ProductPage } from '@prism/dsh-host-catalog/types'
import type {} from '@prism/dsh-host-service'

/** Cordis 插件名。 */
export const name = 'prism-tool-catalog'
/** 注册和执行工具需要的服务。 */
export const inject = ['tools', 'prismService']

/** 搜索结果最多给模型几条，再多也看不过来。 */
export const SEARCH_LIMIT = 20

/** 搜索结果里每个商品给模型看的字段。用 type 而不是 interface：工具结果要能赋给 JsonValue。 */
export type ProductHit = {
  id: string
  name: string
  code: string | null
  category: string | null
  brand: string | null
  archived: boolean
}

function hit(product: Product): ProductHit {
  return {
    id: product.id,
    name: product.name,
    code: product.code,
    category: product.category,
    brand: product.brand,
    archived: product.archived,
  }
}

/**
 * 商品详情里给模型看的部分：去掉时间戳、排序号这些和任务无关的字段，成本、价格按"分"给。
 * @param detail - 服务返回的商品详情。
 * @returns 精简后的详情。
 */
export function productForModel(detail: ProductDetail): JsonValue {
  const { product, skus, summary } = detail
  // 嵌套的数组、对象都重新建一份普通对象：interface 类型不能直接当 JsonValue
  const pairs = (list: ProductDetail['product']['attributes']): JsonValue => list.map(({ name, value }) => ({ name, value }))
  return {
    id: product.id,
    name: product.name,
    code: product.code,
    category: product.category,
    brand: product.brand,
    description: product.description,
    sellingPoints: [...product.sellingPoints],
    attributes: pairs(product.attributes),
    costCents: product.costCents,
    currency: product.currency,
    targetMarkets: [...product.targetMarkets],
    archived: product.archived,
    skus: skus.map(sku => ({
      code: sku.code,
      spec: pairs(sku.spec),
      priceCents: sku.priceCents,
      costCents: sku.costCents,
      stock: sku.stock,
    })),
    assets: {
      images: summary.imageAssets,
      texts: summary.textAssets,
      starred: summary.starredAssets,
    },
    copies: summary.copies.map(({ language, total, approved }) => ({ language, total, approved })),
  }
}

/**
 * 注册两个查询工具。
 * @param ctx - 带 `tools` 和 `prismService` 的上下文。
 */
export function apply(ctx: Context): void {
  ctx.tools.register(defineTool({
    name: 'prism_search_products',
    description: `在棱镜商品库里按关键词查商品，在名称、货号、类目、品牌里搜索，按最近更新排，最多返回 ${SEARCH_LIMIT} 条。`
      + '用户提到某个商品时，先用它找到商品 id，再用 prism_get_product 看详情。关键词为空时返回最近更新的商品。',
    parameters: {
      keyword: { type: 'string', description: '关键词，如商品名的一部分、货号、类目、品牌；可以为空' },
      archived: { type: 'boolean', description: 'true 只查已归档的商品；默认 false，只查在用的' },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          total: { type: 'integer', required: true, description: '符合条件的商品总数，可能多于返回的条数' },
          items: { type: 'array', required: true, items: { type: 'json' } },
        },
      },
      render: (_args, value) => [{ type: 'text', text: JSON.stringify(value) }],
    },
    async execute(args, exec) {
      exec.signal.throwIfAborted()
      const keyword = args.keyword?.trim()
      const page = await ctx.prismService.call<ProductPage>('catalog.listProducts', {
        filter: {
          keyword: keyword === undefined || keyword === '' ? null : keyword,
          archived: args.archived ?? false,
          limit: SEARCH_LIMIT,
          offset: 0,
        },
      }, { signal: exec.signal })
      return { total: page.total, items: page.items.map(hit) }
    },
    presentCall: args => ({
      card: 'generic',
      kind: 'search',
      title: args.keyword === undefined || args.keyword.trim() === '' ? '查看最近的商品' : `查商品：${args.keyword}`,
    }),
  }))

  ctx.tools.register(defineTool({
    name: 'prism_get_product',
    description: '按 id 取棱镜商品库里一个商品的详情：名称、货号、类目、品牌、说明、卖点、参数、成本、目标市场、规格（SKU）、'
      + '素材和文案数量。成本和价格的单位是分。id 用 prism_search_products 查到。',
    parameters: {
      id: { type: 'string', required: true, description: '商品 id' },
    },
    output: {
      schema: { type: 'json' },
      render: (_args, value) => [{ type: 'text', text: JSON.stringify(value) }],
    },
    async execute(args, exec) {
      exec.signal.throwIfAborted()
      const detail = await ctx.prismService.call<ProductDetail>('catalog.productDetail', { id: args.id }, { signal: exec.signal })
      return productForModel(detail)
    },
    presentCall: args => ({ card: 'generic', kind: 'read', title: `查看商品 ${args.id}` }),
  }))
}
