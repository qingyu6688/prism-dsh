/**
 * 商品查询工具的测试：经过 dsh 的工具注册表和执行流水线调用，业务服务用替身。
 * 检查发给服务的参数、给模型的结果，以及服务出错时工具结果标为错误。
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { Context, Service } from '@deepseek-ai/cordis'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import * as NativeToolExecutor from '@deepseek-ai/dsh-tool-executor-native'
import { executeModelToolFixture } from '@deepseek-ai/dsh-tool-testkit'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import type { ToolExecutionResult } from '@deepseek-ai/dsh-tools'
import type { Product, ProductDetail } from '@prism/dsh-host-catalog/types'
import { PrismServiceError } from '@prism/dsh-host-service'
import * as CatalogTools from '@prism/dsh-tool-catalog'

interface Call {
  method: string
  params: object
}

class StubPrismService extends Service {
  readonly calls: Call[] = []
  readonly replies = new Map<string, unknown>()

  constructor(ctx: Context) {
    super(ctx, 'prismService')
  }

  call<T>(method: string, params: object = {}): Promise<T> {
    this.calls.push({ method, params })
    const reply = this.replies.get(method)
    if (reply instanceof Error) return Promise.reject(reply)
    return Promise.resolve(reply as T)
  }
}

function product(overrides: Partial<Product> = {}): Product {
  return {
    id: 'p1',
    code: 'TS-1',
    name: '纯棉T恤',
    category: '上衣',
    brand: null,
    description: null,
    sellingPoints: ['透气'],
    attributes: [{ name: '材质', value: '棉' }],
    costCents: 2580,
    currency: 'CNY',
    targetMarkets: [],
    archived: false,
    createdAt: 1,
    updatedAt: 2,
    ...overrides,
  }
}

let ctx: Context
let stub: StubPrismService

beforeEach(async () => {
  ctx = new Context()
  await ctx.plugin(SystemPrompt)
  await ctx.plugin(ToolRuntime)
  await ctx.plugin(NativeToolExecutor)
  await ctx.plugin(StubPrismService)
  stub = ctx.get('prismService') as unknown as StubPrismService
  await ctx.plugin(CatalogTools)
})

afterEach(async () => {
  await ctx.fiber.dispose()
})

async function run(name: string, args: object): Promise<ToolExecutionResult> {
  return await executeModelToolFixture(ctx, {
    callId: ToolCallId(`call-${name}`),
    name,
    arguments: args,
    signal: new AbortController().signal,
  })
}

function text(result: ToolExecutionResult): string {
  const block = result.content[0]
  return block?.type === 'text' ? block.text : ''
}

describe('prism_search_products', () => {
  it('按关键词查在用的商品，最多 20 条，只给模型需要的字段', async () => {
    stub.replies.set('catalog.listProducts', { total: 31, items: [product()] })
    const result = await run('prism_search_products', { keyword: ' T恤 ' })
    expect(result.isError).toBe(false)
    expect(stub.calls).toEqual([{
      method: 'catalog.listProducts',
      params: { filter: { keyword: 'T恤', archived: false, limit: 20, offset: 0 } },
    }])
    expect(JSON.parse(text(result))).toEqual({
      total: 31,
      items: [{ id: 'p1', name: '纯棉T恤', code: 'TS-1', category: '上衣', brand: null, archived: false }],
    })
  })

  it('关键词为空查最近的商品；可以查已归档的', async () => {
    stub.replies.set('catalog.listProducts', { total: 0, items: [] })
    await run('prism_search_products', { keyword: '  ', archived: true })
    expect(stub.calls[0]?.params).toEqual({ filter: { keyword: null, archived: true, limit: 20, offset: 0 } })
  })
})

describe('prism_get_product', () => {
  it('返回精简的详情：没有时间戳，素材按类型计数', async () => {
    const detail: ProductDetail = {
      product: product(),
      skus: [{ id: 'k1', code: 'TS-1-M', spec: [{ name: '尺码', value: 'M' }], priceCents: 5900, costCents: null, stock: 10, sortOrder: 0 }],
      summary: {
        skuCount: 1, imageAssets: 3, textAssets: 1, starredAssets: 1,
        copies: [{ language: 'zh-CN', total: 2, approved: 1 }], batches: [], lastBatchAt: null,
      },
    }
    stub.replies.set('catalog.productDetail', detail)
    const result = await run('prism_get_product', { id: 'p1' })
    expect(result.isError).toBe(false)
    expect(stub.calls).toEqual([{ method: 'catalog.productDetail', params: { id: 'p1' } }])
    const value = JSON.parse(text(result)) as Record<string, unknown>
    expect(value).toMatchObject({
      id: 'p1',
      sellingPoints: ['透气'],
      skus: [{ code: 'TS-1-M', priceCents: 5900, stock: 10 }],
      assets: { images: 3, texts: 1, starred: 1 },
      copies: [{ language: 'zh-CN', total: 2, approved: 1 }],
    })
    expect(value).not.toHaveProperty('createdAt')
  })

  it('商品不存在时工具结果是错误，带服务的说明', async () => {
    stub.replies.set('catalog.productDetail', new PrismServiceError('商品不存在', 'not_found', -32001))
    const result = await run('prism_get_product', { id: 'nope' })
    expect(result.isError).toBe(true)
    expect(text(result)).toContain('商品不存在')
  })

  it('缺 id 时在调用服务前就拒绝', async () => {
    const result = await run('prism_get_product', {})
    expect(result.isError).toBe(true)
    expect(stub.calls).toEqual([])
  })
})
