/**
 * `ctx.prismService` 的测试：用真实的子进程服务（subprocess-local）拉起假服务
 * （tests/fake-service.ts），检查调用、错误分类、超时、取消、崩溃重启和卸载。
 * 仓库里已经用 cargo 构建过 prism-service 时，再对真服务跑一遍 ping 和 version。
 */

import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { existsSync } from 'node:fs'
import { mkdtemp, realpath, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { Context } from '@deepseek-ai/cordis'
import LocalSubprocessRuntime from '@deepseek-ai/dsh-subprocess-local'
import PrismService, { PrismServiceError, resolveServiceCommand } from '@prism/dsh-host-service'
import type { Config } from '@prism/dsh-host-service'

const fakeService = fileURLToPath(new URL('./fake-service.ts', import.meta.url))

let root: string
let ctx: Context

beforeEach(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), 'prism-host-service-')))
  ctx = new Context()
  await ctx.plugin(LocalSubprocessRuntime)
})

afterEach(async () => {
  await ctx.fiber.dispose()
  await rm(root, { recursive: true, force: true })
})

async function mount(extraArgs: string[] = [], overrides: Partial<Config> = {}): Promise<PrismService> {
  await ctx.plugin(PrismService, {
    command: process.execPath,
    args: [fakeService, ...extraArgs],
    dataDir: join(root, 'data'),
    ...overrides,
  })
  return ctx.prismService
}

async function rejection(promise: Promise<unknown>): Promise<PrismServiceError> {
  const error = await promise.then(() => undefined, (e: unknown) => e)
  expect(error).toBeInstanceOf(PrismServiceError)
  return error as PrismServiceError
}

describe('调用', () => {
  it('等服务就绪后返回结果', async () => {
    const service = await mount()
    await expect(service.call('service.ping')).resolves.toEqual({ pong: true })
    expect(service.status()).toEqual({ state: 'running', exits: 0 })
    expect(service.dataDir()).toBe(join(root, 'data'))
    await expect(service.call('test.echo', { text: '第一行\n第二行' })).resolves.toEqual({ text: '第一行\n第二行' })
  })

  it('并发调用按 id 对应', async () => {
    const service = await mount()
    const results = await Promise.all(Array.from({ length: 20 }, (_, n) => service.call('test.echo', { n })))
    expect(results).toEqual(Array.from({ length: 20 }, (_, n) => ({ n })))
  })

  it('业务错误带服务给的分类和说明', async () => {
    const service = await mount()
    const error = await rejection(service.call('test.notFound'))
    expect(error.message).toBe('商品不存在')
    expect(error.code).toBe('not_found')
    expect(error.rpcCode).toBe(-32001)
    expect(error.detail).toEqual({ id: 7 })
  })

  it('不存在的方法', async () => {
    const service = await mount()
    const error = await rejection(service.call('catalog.nope'))
    expect(error.code).toBe('method_not_found')
    expect(error.rpcCode).toBe(-32601)
  })

  it('超时', async () => {
    const service = await mount()
    await service.call('service.ping')
    const error = await rejection(service.call('test.slow', {}, { timeoutMs: 100 }))
    expect(error.code).toBe('timeout')
    // 超时的调用不影响后面的调用
    await expect(service.call('service.ping')).resolves.toEqual({ pong: true })
  })

  it('取消时以 signal.reason 拒绝', async () => {
    const service = await mount()
    await service.call('service.ping')
    const controller = new AbortController()
    const pending = service.call('test.slow', {}, { signal: controller.signal })
    const reason = new Error('用户取消')
    controller.abort(reason)
    await expect(pending).rejects.toBe(reason)
    await expect(service.call('test.slow', {}, { signal: controller.signal })).rejects.toBe(reason)

    // 取消原因不是 Error 时包成 aborted
    const other = new AbortController()
    const pendingOther = service.call('test.slow', {}, { signal: other.signal })
    other.abort('不是 Error')
    expect((await rejection(pendingOther)).code).toBe('aborted')
  })

  it('通知不影响调用', async () => {
    const service = await mount()
    await expect(service.call('test.notify')).resolves.toEqual({})
  })
})

describe('崩溃和重启', () => {
  it('调用期间进程退出：进行中的调用报 exited，重启后继续可用', async () => {
    const service = await mount()
    await service.call('service.ping')
    const error = await rejection(service.call('test.crash'))
    expect(error.code).toBe('exited')
    expect(error.message).toContain('退出码 1')
    expect(service.status().state).toBe('restarting')
    // 重启期间发起的调用等新进程就绪
    await expect(service.call('service.ping')).resolves.toEqual({ pong: true })
    expect(service.status()).toMatchObject({ state: 'running', exits: 1 })
  })

  it('输出格式错乱时结束进程并重启', async () => {
    const service = await mount()
    await service.call('service.ping')
    const error = await rejection(service.call('test.garbage'))
    expect(error.code).toBe('protocol')
    await expect(service.call('service.ping')).resolves.toEqual({ pong: true })
    expect(service.status().exits).toBe(1)
  })

  it('启动时退出一次后重启成功', async () => {
    const service = await mount(['--mode', 'crash-once', '--marker', join(root, 'crashed')])
    await expect(service.call('service.ping')).resolves.toEqual({ pong: true })
    expect(service.status()).toMatchObject({ state: 'running', exits: 1 })
  })

  it('30 秒内退出 3 次后不再重启', async () => {
    const service = await mount(['--mode', 'crash-on-start'])
    const error = await rejection(service.call('service.ping'))
    expect(error.code).toBe('unavailable')
    expect(error.message).toContain('不再重启')
    // stderr 里最后一条 ERROR 放进说明，方便排查
    expect(error.message).toContain('打开数据库失败')
    expect(service.status()).toMatchObject({ state: 'failed', exits: 3 })
  }, 20_000)
})

describe('启动失败和卸载', () => {
  it('找不到可执行文件时插件照常加载，调用报 unavailable', async () => {
    const service = await mount([], { command: join(root, 'missing.exe') })
    expect(service.status().state).toBe('failed')
    expect(service.dataDir()).toBeUndefined()
    const error = await rejection(service.call('service.ping'))
    expect(error.code).toBe('unavailable')
    expect(error.message).toContain('missing.exe')
  })

  it('卸载时关闭 stdin，服务正常退出', async () => {
    const service = await mount()
    await service.call('service.ping')
    await ctx.fiber.dispose()
    expect(service.status().state).toBe('stopped')
    expect(existsSync(join(root, 'data', 'clean-exit'))).toBe(true)
    ctx = new Context()
  })
})

describe('resolveServiceCommand', () => {
  it('配置优先，其次环境变量', () => {
    expect(resolveServiceCommand('a.exe', { PRISM_SERVICE_BIN: 'b.exe' })).toBe('a.exe')
    expect(resolveServiceCommand(undefined, { PRISM_SERVICE_BIN: 'b.exe' })).toBe('b.exe')
  })

  it('往上找不到构建目录时返回 undefined', () => {
    expect(resolveServiceCommand(undefined, {}, root)).toBeUndefined()
  })
})

const realBinary = resolveServiceCommand(undefined, {})

describe.skipIf(realBinary === undefined)('真 prism-service', () => {
  it('ping 和 version', async () => {
    const dataDir = join(root, '真服务数据')
    await ctx.plugin(PrismService, { command: realBinary, dataDir, logLevel: 'warn' })
    const service = ctx.prismService
    await expect(service.call('service.ping')).resolves.toEqual({ pong: true })
    const version = await service.call<{ version: string; dataDir: string; schemaVersion: number }>('service.version')
    expect(version.dataDir).toBe(dataDir)
    expect(version.schemaVersion).toBeGreaterThan(0)
    await ctx.fiber.dispose()
    expect(service.status().state).toBe('stopped')
    ctx = new Context()
  })
})
