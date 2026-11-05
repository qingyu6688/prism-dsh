/**
 * `PrismCatalog` 的测试：方法名和参数原样转给 `ctx.prismService`，服务的错误转成 `prism/failed`，
 * 两个文件路由注册在 Connection 上并随插件卸载移除。业务服务和 Connection 用替身。
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context, Service } from '@deepseek-ai/cordis'
import type { ConnectionFetchRoute } from '@deepseek-ai/dsh-client-connection'
import { remoteErrorOf } from '@deepseek-ai/dsh-typert-protocol'
import TypertRegistry from '@deepseek-ai/dsh-typert-registry'
import PrismCatalog, { FILE_PATH, UPLOAD_PATH } from '@prism/dsh-host-catalog'
import { PrismServiceError } from '@prism/dsh-host-service'

interface Call {
  method: string
  params: object
}

/** 记下每次调用，按方法名返回预设结果或抛错。 */
class StubPrismService extends Service {
  readonly calls: Call[] = []
  readonly replies = new Map<string, unknown>()
  dir: string | undefined

  constructor(ctx: Context) {
    super(ctx, 'prismService')
  }

  call<T>(method: string, params: object = {}): Promise<T> {
    this.calls.push({ method, params })
    const reply = this.replies.get(method)
    if (reply instanceof Error) return Promise.reject(reply)
    return Promise.resolve(reply as T)
  }

  dataDir(): string | undefined {
    return this.dir
  }
}

let ctx: Context
let stub: StubPrismService
let routes: Map<string, ConnectionFetchRoute>
let dataDir: string

beforeEach(async () => {
  dataDir = await mkdtemp(join(tmpdir(), 'prism-catalog-'))
  ctx = new Context()
  routes = new Map()
  ctx.reflect.provide('connection', {
    fetch: {
      register: (route: ConnectionFetchRoute) => {
        routes.set(route.path, route)
        return () => {
          routes.delete(route.path)
          return Promise.resolve()
        }
      },
    },
  })
  await ctx.plugin(TypertRegistry)
  await ctx.plugin(StubPrismService)
  stub = ctx.get('prismService') as unknown as StubPrismService
  stub.dir = dataDir
  await ctx.plugin(PrismCatalog)
})

afterEach(async () => {
  await ctx.fiber.dispose()
  await rm(dataDir, { recursive: true, force: true })
})

describe('转调业务服务', () => {
  it('方法名和参数原样转过去，结果原样返回', async () => {
    stub.replies.set('catalog.listShops', [{ id: 's1' }])
    await expect(ctx.prismCatalog.listShops({ platform: 'amazon' })).resolves.toEqual([{ id: 's1' }])
    stub.replies.set('catalog.listProducts', { items: [], total: 0 })
    await ctx.prismCatalog.listProducts({ filter: { keyword: 'T恤' } })
    await ctx.prismCatalog.archiveProduct({ id: 'p1' })
    await ctx.prismCatalog.listImportMappings()
    await ctx.prismCatalog.removeUserRule({ kind: 'rule_pack', id: 'amazon' })
    await ctx.prismCatalog.search({ keyword: '杯' })
    expect(stub.calls).toEqual([
      { method: 'catalog.listShops', params: { platform: 'amazon' } },
      { method: 'catalog.listProducts', params: { filter: { keyword: 'T恤' } } },
      { method: 'catalog.archiveProduct', params: { id: 'p1' } },
      { method: 'catalog.listImportMappings', params: {} },
      { method: 'rules.removeUser', params: { kind: 'rule_pack', id: 'amazon' } },
      { method: 'search.global', params: { keyword: '杯' } },
    ])
  })

  it('服务的错误转成 prism/failed，带分类和细节', async () => {
    stub.replies.set('catalog.productDetail', new PrismServiceError('商品不存在', 'not_found', -32001, { id: 'p1' }))
    const failure = await ctx.prismCatalog.productDetail({ id: 'p1' }).catch((error: unknown) => error)
    expect(remoteErrorOf(failure)).toMatchObject({
      code: 'prism/failed',
      message: '商品不存在',
      details: { code: 'not_found', detail: { id: 'p1' } },
    })

    stub.replies.set('rules.list', new PrismServiceError('棱镜业务服务不可用：插件已停止', 'unavailable'))
    const unavailable = await ctx.prismCatalog.listRules().catch((error: unknown) => error)
    expect(remoteErrorOf(unavailable)).toMatchObject({ details: { code: 'unavailable', detail: null } })
  })

  it('不是服务错误的异常原样抛出', async () => {
    const bug = new TypeError('程序出错')
    stub.replies.set('catalog.listShops', bug)
    await expect(ctx.prismCatalog.listShops({})).rejects.toBe(bug)
  })
})

describe('文件路由', () => {
  it('注册读文件和上传两个路由，读的是业务服务的数据目录', async () => {
    expect([...routes.keys()].sort()).toEqual([FILE_PATH, UPLOAD_PATH].sort())
    await writeFile(join(dataDir, 'a.png'), 'png')
    const response = await routes.get(FILE_PATH)!.fetch(new Request(`http://localhost${FILE_PATH}?path=a.png`))
    expect(response.status).toBe(200)
    expect(await response.text()).toBe('png')
  })

  it('插件卸载后路由移除', async () => {
    await ctx.fiber.dispose()
    expect(routes.size).toBe(0)
    ctx = new Context()
  })
})
