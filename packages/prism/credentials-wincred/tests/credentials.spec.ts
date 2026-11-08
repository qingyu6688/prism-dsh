/** 旧文件迁移以系统写入成功为前提，失败时保留可恢复的原文件。 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { Context, Service } from '@deepseek-ai/cordis'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { credentialKey, credentialRef } from '@deepseek-ai/dsh-credentials'
import WinCredentials, { decodeRecord } from '../src/index.ts'

let ctx: Context
let dir: string
const values = new Map<string, string>()
const rpc = vi.fn()
beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'prism-credentials-'))
  vi.stubEnv('DSH_HOME', dir)
  values.clear()
  rpc.mockReset().mockImplementation(async (method: string, args?: { key: string; value?: string }) => {
    if (method === 'credentials.list') return [...values.keys()]
    if (args === undefined) throw new Error('缺少参数')
    if (method === 'credentials.get') return values.get(args.key) ?? null
    if (method === 'credentials.set') { values.set(args.key, args.value!); return {} }
    if (method === 'credentials.delete') { values.delete(args.key); return {} }
    throw new Error('未知方法')
  })
  ctx = new Context()
  await ctx.plugin(class extends Service {
    call = rpc
    constructor(c: Context) { super(c, 'prismService') }
  })
})
afterEach(async () => {
  await ctx.fiber.dispose()
  vi.unstubAllEnvs()
  await rm(dir, { recursive: true, force: true })
})
it('迁移文本 Key 和授权记录后移除原秘密字段，保留版本', async () => {
  const file = join(dir, '.credentials.yaml')
  await writeFile(file, 'version: 1\nrefs:\n  TEST_KEY: secret-one\nrecords:\n  llm/provider:\n    kind: api-key\n    key: secret-two\n')
  await ctx.plugin(WinCredentials)
  expect(await ctx.credentials.resolve(credentialRef('TEST_KEY'))).toMatchObject({ value: 'secret-one', source: 'wincred' })
  expect(await ctx.credentials.readRecord(credentialKey('llm', 'provider'))).toEqual({ kind: 'api-key', key: 'secret-two' })
  const text = await readFile(file, 'utf8')
  expect(text).toContain('version: 1')
  expect(text).not.toMatch(/secret|refs:|records:/)
})
it('系统存储失败时原文件逐字保留', async () => {
  const source = 'version: 1\nrefs:\n  TEST_KEY: keep-me\n'
  const file = join(dir, '.credentials.yaml')
  await writeFile(file, source)
  rpc.mockImplementation(async (method: string) => {
    if (method === 'credentials.get') return null
    throw new Error('凭据管理器不可用')
  })
  await expect(ctx.plugin(WinCredentials)).rejects.toThrow()
  expect(await readFile(file, 'utf8')).toBe(source)
})
it('未知文件版本不迁移也不覆盖', async () => {
  const file = join(dir, '.credentials.yaml')
  const source = 'version: 99\nrefs:\n  TEST_KEY: keep-me\n'
  await writeFile(file, source)
  await expect(ctx.plugin(WinCredentials)).rejects.toThrow()
  expect(rpc).not.toHaveBeenCalled()
  expect(await readFile(file, 'utf8')).toBe(source)
})
it('并发修改按前一次保存结果执行，删除后列表消失', async () => {
  await ctx.plugin(WinCredentials)
  const key = credentialKey('llm', 'test')
  await Promise.all([
    ctx.credentials.modifyRecord(key, async () => ({ kind: 'api-key', key: 'first' })),
    ctx.credentials.modifyRecord(key, async (current) => { expect(current).toEqual({ kind: 'api-key', key: 'first' }); return { kind: 'api-key', key: 'second' } }),
  ])
  expect(await ctx.credentials.listRecords()).toEqual([{ key, kind: 'api-key' }])
  await ctx.credentials.deleteRecord(key)
  expect(await ctx.credentials.listRecords()).toEqual([])
})
it('损坏记录的错误不回显秘密', () => {
  expect(() => decodeRecord('{"kind":"secret-value"}')).toThrow('系统凭据记录损坏，请重新保存')
  expect(() => decodeRecord('{"kind":"api-key","key":12}')).toThrow('系统凭据记录损坏')
})
