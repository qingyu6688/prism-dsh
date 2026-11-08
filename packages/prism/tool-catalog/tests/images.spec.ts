/** 生图工具经过工具流水线，核对审批、后台任务交接和取消的副作用。 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { Context, Service } from '@deepseek-ai/cordis'
import type { Agent } from '@deepseek-ai/dsh-agent'
import { SessionId } from '@deepseek-ai/dsh-session'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { executeToolFixture } from '@deepseek-ai/dsh-tool-testkit'
import { JobId, type JobHooks, type JobSpec } from '@deepseek-ai/dsh-jobs'
import { registerImageTools } from '../src/image.ts'
import { Config } from '../src/index.ts'

let ctx: Context
let hooks: JobHooks | undefined
const call = vi.fn()
const approve = vi.fn()
const start = vi.fn()
const batch = { id: 'batch-1', owner: 'session-1', status: 'completed', total: 1, succeeded: 1, failed: 0, assets: [], errors: [], reason: null }
beforeEach(async () => {
  call.mockReset().mockResolvedValue(batch)
  approve.mockReset().mockResolvedValue('allowed-once')
  start.mockReset().mockImplementation((spec: JobSpec) => {
    hooks = spec.run({ id: JobId('prism-image-1'), append: vi.fn(), updateProgress: vi.fn() })
    return 'prism-image-1'
  })
  hooks = undefined
  ctx = new Context()
  await ctx.plugin(SystemPrompt)
  await ctx.plugin(ToolRuntime)
  await ctx.plugin(class extends Service {
    call = call
    constructor(c: Context) { super(c, 'prismService') }
  })
  await ctx.plugin(class extends Service {
    request = approve
    constructor(c: Context) { super(c, 'approval') }
  })
  await ctx.plugin(class extends Service {
    start = start
    constructor(c: Context) { super(c, 'jobs') }
  })
  registerImageTools(ctx, 1000)
})
afterEach(async () => {
  hooks?.cancel?.()
  await hooks?.done
  await ctx.fiber.dispose()
})
function run(count: number, fission = false, signal = new AbortController().signal) {
  const agent = { ctx, session: { id: SessionId('session-1') } } as Agent
  return executeToolFixture(ctx, { name: fission ? 'prism_fission_images' : 'prism_generate_images',
    callId: ToolCallId('call-image'), arguments: { prompt: '蓝色水杯', count, ...(fission ? { referenceIds: ['asset-1'] } : {}) }, agent, signal })
}
it.each([1, 40])('%s 张不询问，返回可重开的批次 ID', async (count) => {
  const result = await run(count)
  expect(result.isError).toBe(false)
  expect(result.content[0]?.type === 'text' ? result.content[0].text : '').toContain('"batchId":"batch-1"')
  expect(approve).not.toHaveBeenCalled()
  expect(call.mock.calls[0]).toMatchObject(['image.submitBatch', { owner: 'session-1', args: { count } }])
})
it.each(['rejected', 'cancelled', 'unavailable'])('41 张审批 %s 时不建任务或批次', async (outcome) => {
  approve.mockResolvedValue(outcome)
  expect((await run(41)).isError).toBe(true)
  expect(approve).toHaveBeenCalledOnce()
  expect(start).not.toHaveBeenCalled()
  expect(call).not.toHaveBeenCalled()
})
it('审批通过才交接图裂变，保留参考素材', async () => {
  expect((await run(41, true)).isError).toBe(false)
  expect(call.mock.calls[0]).toMatchObject(['image.submitBatch', { fission: true, args: { referenceIds: ['asset-1'] } }])
})
it('没有任务控制器时不会提交生图', async () => {
  start.mockImplementation(() => { throw new Error('没有任务控制器') })
  expect((await run(1)).isError).toBe(true)
  expect(call).not.toHaveBeenCalled()
})
it('持久化提交期间取消会补发批次取消', async () => {
  const submitted = Promise.withResolvers<typeof batch>()
  call.mockImplementation((method: string) => method === 'image.submitBatch' ? submitted.promise : Promise.resolve(batch))
  const pending = run(1)
  await vi.waitFor(() => { expect(hooks).toBeDefined() })
  hooks?.cancel?.()
  submitted.resolve(batch)
  await pending
  expect(await hooks?.done).toMatchObject({ status: 'killed' })
  expect(call).toHaveBeenCalledWith('image.cancelBatch', { id: 'batch-1' })
})
it('提交失败不伪报成功，保留可检查的错误', async () => {
  call.mockRejectedValue(new Error('没有生图配置'))
  expect((await run(1)).isError).toBe(true)
  expect(await hooks?.done).toMatchObject({ status: 'failed' })
})

it('查询间隔使用配置默认值，并拒绝无效频率', () => {
  expect(Config({}).imagePollIntervalMs).toBe(1000)
  expect(Config({ imagePollIntervalMs: 2500 }).imagePollIntervalMs).toBe(2500)
  for (const value of [0, 99, 100.5, 60001]) {
    expect(() => Config({ imagePollIntervalMs: value })).toThrow()
  }
})
