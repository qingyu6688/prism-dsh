/** Agent 生图工具：审批后启动会话所属后台任务，完成结果经 jobs 回到会话。 */
import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import type { JobHandle, JobOutcome } from '@deepseek-ai/dsh-jobs'
import type {} from '@deepseek-ai/dsh-user-approval'
import type { ImageBatch } from '@prism/dsh-host-catalog/types'
import { setTimeout as delay } from 'node:timers/promises'

declare module '@deepseek-ai/dsh-jobs' {
  interface JobKindMap { 'prism-image': 'prism-image' }
}

/**
 * 查询批次直到落入终态。取消等待时仍向服务提交取消，错误会保留未知结果说明。
 * @param ctx - 拥有业务服务的上下文。
 * @param id - 持久化批次 ID。
 * @param job - 后台任务输出。
 * @param signal - 后台任务取消信号。
 * @param pollIntervalMs - 批次状态查询间隔，单位毫秒。
 * @returns 后台任务最终结果。
 */
export async function followImageBatch(
  ctx: Context, id: string, job: JobHandle, signal: AbortSignal, pollIntervalMs: number,
): Promise<JobOutcome> {
  try {
    for (;;) {
      if (signal.aborted) {
        await ctx.prismService.call('image.cancelBatch', { id })
        return { status: 'killed', detail: '已取消生图批次，已完成的图片保留在素材库' }
      }
      const batch = await ctx.prismService.call<ImageBatch>('image.batchStatus', { id })
      job.updateProgress(`${batch.succeeded + batch.failed}/${batch.total}`)
      if (['completed', 'failed', 'cancelled', 'paused'].includes(batch.status)) {
        const result = JSON.stringify(batch)
        job.append(result)
        return { status: batch.status === 'completed' ? 'completed' : batch.status === 'cancelled' ? 'killed' : 'failed',
          detail: batch.reason ?? batch.errors[0] ?? `成功 ${batch.succeeded} 张，失败 ${batch.failed} 张`, result }
      }
      try { await delay(pollIntervalMs, undefined, { signal }) } catch (error) {
        if (!(error instanceof Error && error.name === 'AbortError')) throw error
      }
    }
  } catch (error) {
    return { status: 'failed', detail: `无法确认批次 ${id} 的状态，请在生图结果中检查：${error instanceof Error ? error.message : String(error)}` }
  }
}

/**
 * 在具备后台任务和审批服务的 Agent 预设中注册生图工具。
 * @param ctx - Agent 作用域上下文。
 * @param pollIntervalMs - 批次状态查询间隔，单位毫秒。
 */
export function registerImageTools(ctx: Context, pollIntervalMs: number): void {
  for (const fission of [false, true]) {
    const name = fission ? 'prism_fission_images' : 'prism_generate_images'
    ctx.tools.register(defineTool({
      name,
      description: fission ? '基于素材库参考图生成不同构图或场景的变体。默认 4 张，必须提供 referenceIds。返回后台任务 ID，完成后结果自动进入素材库。'
        : '生成图片或按参考素材编辑图片，默认 1 张。参考图使用素材 ID。超过 40 张需要审批；返回后台任务 ID，使用 job 工具查看进度或取消。',
      parameters: {
        prompt: { type: 'string', required: true, description: '完整画面需求，1～4000 字' },
        count: { type: 'integer', description: '生成数量，1～200' },
        referenceIds: { type: 'array', items: { type: 'string' }, description: '素材库图片 ID，最多 6 张' },
        productId: { type: 'string', description: '关联商品 ID，可省略' },
        size: { type: 'string', description: '图片尺寸，例如 1024x1024，可省略' },
      },
      output: { schema: { type: 'json' }, render: (_args, value) => [{ type: 'text', text: JSON.stringify(value) }] },
      async execute(args, exec) {
        const agent = exec.agent
        if (agent === undefined) throw new Error('生图需要当前会话')
        const count = args.count ?? (fission ? 4 : 1)
        if (!Number.isInteger(count) || count < 1 || count > 200) throw new Error('生成数量需要在 1～200 之间')
        if (count > 40) {
          const outcome = await ctx.approval.request({ agent, toolName: name, callId: exec.callId,
            reason: `本次将生成 ${count} 张图片，可能产生接口费用`, signal: exec.signal })
          if (outcome !== 'allowed-once') throw new Error('尚未获准生成这一批图片')
        }
        exec.signal.throwIfAborted()
        const controller = new AbortController()
        const submitted = Promise.withResolvers<ImageBatch>()
        // 后台任务仍拥有整批执行；工具只等批次持久化，便于会话记录准确引用结果。
        void submitted.promise.catch(() => undefined)
        const jobId = ctx.jobs.start({ kind: 'prism-image', owner: agent.session.id, label: args.prompt,
          toolCall: { callId: exec.callId, rootCallId: exec.rootCallId },
          run(job) {
            const done = (async (): Promise<JobOutcome> => {
              try {
                // 注册成功后才提交，避免审批拒绝或后台任务不可用时留下生图批次。
                if (controller.signal.aborted) { submitted.reject(new Error('生图已取消')); return { status: 'killed' } }
                const batch = await ctx.prismService.call<ImageBatch>('image.submitBatch', {
                  owner: agent.session.id, fission,
                  args: { prompt: args.prompt, count, referenceIds: args.referenceIds ?? [],
                    ...(args.productId === undefined ? {} : { productId: args.productId }),
                    ...(args.size === undefined ? {} : { size: args.size }) },
                })
                submitted.resolve(batch)
                job.append(`生图批次：${batch.id}\n`)
                return await followImageBatch(ctx, batch.id, job, controller.signal, pollIntervalMs)
              } catch (error) {
                submitted.reject(error)
                return { status: 'failed', detail: `生图提交失败或结果未知，请检查生图结果后再试：${error instanceof Error ? error.message : String(error)}` }
              }
            })()
            return { cancel: () => { controller.abort() }, done }
          },
        })
        const batch = await submitted.promise
        return { jobId, batchId: batch.id, count, message: '已创建生图后台任务，完成后自动通知；可在生图结果和素材库查看图片' }
      },
      presentCall: args => ({ card: 'generic', kind: 'other', title: `${fission ? '图裂变' : '生图'}：${args.prompt}` }),
    }))
  }
}
