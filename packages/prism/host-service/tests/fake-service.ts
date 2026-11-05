/**
 * 测试用的假 prism-service：协议和真服务一样（一行一条 JSON-RPC），行为由参数控制。
 *
 * 用法：node fake-service.ts [--mode ok|crash-on-start|crash-once] [--marker <文件>] --data-dir <目录> ...
 * - `crash-on-start`：一启动就以退出码 1 退出
 * - `crash-once`：marker 文件不存在时创建它并退出，之后正常工作
 * - stdin 关闭时在数据目录写 `clean-exit` 再退出，测试据此判断是正常退出
 */

import { existsSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { createInterface } from 'node:readline'

function arg(name: string): string | undefined {
  const index = process.argv.indexOf(name)
  return index === -1 ? undefined : process.argv[index + 1]
}

const mode = arg('--mode') ?? 'ok'
const marker = arg('--marker')
const dataDir = arg('--data-dir') ?? '.'

if (mode === 'crash-on-start') {
  process.stderr.write('2026-11-05T00:00:00Z ERROR 打开数据库失败\n')
  process.exit(1)
}
if (mode === 'crash-once' && marker !== undefined && !existsSync(marker)) {
  writeFileSync(marker, 'x')
  process.exit(1)
}

process.stderr.write('2026-11-05T00:00:00Z  INFO prism-service 已启动\n')

function reply(message: object): void {
  process.stdout.write(`${JSON.stringify({ jsonrpc: '2.0', ...message })}\n`)
}

const lines = createInterface({ input: process.stdin })
lines.on('line', (line) => {
  const request = JSON.parse(line) as { id: number; method: string; params: Record<string, unknown> }
  const { id, method, params } = request
  switch (method) {
    case 'service.ping':
      reply({ id, result: { pong: true } })
      break
    case 'test.echo':
      reply({ id, result: params })
      break
    case 'test.notFound':
      reply({ id, error: { code: -32001, message: '商品不存在', data: { code: 'not_found', detail: { id: 7 } } } })
      break
    case 'test.slow':
      // 不回响应，用来测超时和取消
      break
    case 'test.crash':
      process.exit(1)
      break
    case 'test.garbage':
      process.stdout.write('这不是 JSON\n')
      break
    case 'test.notify':
      reply({ method: 'test.progress', params: { done: 1 } })
      reply({ id, result: {} })
      break
    default:
      reply({ id, error: { code: -32601, message: `没有这个方法：${method}` } })
  }
})
lines.on('close', () => {
  writeFileSync(join(dataDir, 'clean-exit'), 'x')
  process.exit(0)
})
