/**
 * 一个 prism-service 进程上的 JSON-RPC 连接：按行收发、按 id 对应响应、超时和取消。
 * 协议见 `docs/prism/service-api.md` 第 2 节。进程的启动、重启归 `index.ts`。
 * @module @prism/dsh-host-service/connection
 */

import type { Readable, Writable } from 'node:stream'
import { PrismServiceError, abortError } from './errors.ts'

const METHOD_NOT_FOUND = -32601

interface Pending {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}

interface RpcErrorObject {
  code: number
  message: string
  data?: { code?: unknown; detail?: unknown }
}

/** 连接的回调。 */
export interface ConnectionHooks {
  /** 服务发来的通知（没有 id 的消息）。 */
  onNotification(method: string, params: unknown): void
  /** 连接已经不可用（输出格式错乱、消息过大、写入失败），进程那边应当被结束。 */
  onFatal(error: Error): void
}

/** 绑定在一个进程 stdin / stdout 上的连接。 */
export class RpcConnection {
  private readonly pending = new Map<number, Pending>()
  private nextId = 1
  private parts: Buffer[] = []
  private partBytes = 0
  private closedReason: Error | undefined

  /**
   * @param stdin - 服务进程的 stdin。
   * @param stdout - 服务进程的 stdout。
   * @param maxMessageBytes - 单条消息上限，超过就当作连接出错。
   * @param hooks - 通知和致命错误的回调。
   */
  constructor(
    private readonly stdin: Writable,
    stdout: Readable,
    private readonly maxMessageBytes: number,
    private readonly hooks: ConnectionHooks,
  ) {
    // 进程关了 stdin 时写入会报 EPIPE，不处理的话会变成未捕获异常
    stdin.on('error', (error) => { this.fatal(error) })
    stdout.on('data', (chunk: Buffer) => { this.onData(chunk) })
  }

  /**
   * 调用一个方法。
   * @param method - 方法名，如 `service.ping`。
   * @param params - 参数对象。
   * @param timeoutMs - 超时毫秒数。
   * @param signal - 取消信号；取消时以 `signal.reason`（不是 Error 时包成 `aborted` 错误）拒绝。
   * @returns 服务返回的 `result`。
   */
  call(method: string, params: object, timeoutMs: number, signal?: AbortSignal): Promise<unknown> {
    if (this.closedReason !== undefined) return Promise.reject(this.closedReason)
    if (signal?.aborted === true) return Promise.reject(abortError(signal))
    const id = this.nextId++
    return new Promise((resolve, reject) => {
      const cleanup = (): void => {
        clearTimeout(timer)
        signal?.removeEventListener('abort', onAbort)
        this.pending.delete(id)
      }
      const fail = (error: Error): void => { cleanup(); reject(error) }
      const onAbort = (): void => { if (signal !== undefined) fail(abortError(signal)) }
      const timer = setTimeout(() => {
        fail(new PrismServiceError(`调用 ${method} 超时（${timeoutMs} 毫秒）`, 'timeout'))
      }, timeoutMs)
      signal?.addEventListener('abort', onAbort, { once: true })
      this.pending.set(id, {
        resolve: (value) => { cleanup(); resolve(value) },
        reject: fail,
      })
      const line = `${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`
      this.stdin.write(line, (error) => {
        if (error) this.fatal(error)
      })
    })
  }

  /**
   * 关闭连接：进行中的调用全部以 `reason` 拒绝，之后的调用也立即拒绝。
   * @param reason - 拒绝原因。
   */
  close(reason: Error): void {
    if (this.closedReason !== undefined) return
    this.closedReason = reason
    this.parts = []
    this.partBytes = 0
    for (const pending of [...this.pending.values()]) pending.reject(reason)
  }

  private fatal(error: Error): void {
    if (this.closedReason !== undefined) return
    this.close(new PrismServiceError(`和棱镜业务服务的连接出错：${error.message}`, 'protocol'))
    this.hooks.onFatal(error)
  }

  private onData(chunk: Buffer): void {
    let start = 0
    while (this.closedReason === undefined) {
      const newline = chunk.indexOf(0x0a, start)
      if (newline === -1) {
        this.append(chunk.subarray(start))
        return
      }
      if (!this.append(chunk.subarray(start, newline))) return
      start = newline + 1
      const line = Buffer.concat(this.parts).toString('utf8')
      this.parts = []
      this.partBytes = 0
      this.onLine(line)
    }
  }

  /** 追加一段；超过上限时关闭连接并返回 false。 */
  private append(part: Buffer): boolean {
    if (part.length === 0) return true
    this.partBytes += part.length
    if (this.partBytes > this.maxMessageBytes) {
      this.fatal(new Error(`服务发来的消息超过 ${this.maxMessageBytes} 字节`))
      return false
    }
    this.parts.push(part)
    return true
  }

  private onLine(raw: string): void {
    const line = raw.trim()
    if (line === '') return
    let message: unknown
    try {
      message = JSON.parse(line)
    } catch (error) {
      this.fatal(new Error(`服务输出了无法解析的内容：${(error as Error).message}`))
      return
    }
    if (typeof message !== 'object' || message === null || Array.isArray(message)) {
      this.fatal(new Error('服务输出的消息不是 JSON 对象'))
      return
    }
    const record = message as Record<string, unknown>
    if (!('id' in record)) {
      if (typeof record.method === 'string') this.hooks.onNotification(record.method, record.params)
      return
    }
    // id 为 null 的错误响应说明服务没能解析我们发的某条消息，对不上任何调用
    if (typeof record.id !== 'number') return
    const pending = this.pending.get(record.id)
    if (pending === undefined) return
    if ('error' in record) pending.reject(toError(record.error))
    else pending.resolve(record.result)
  }
}

function toError(raw: unknown): PrismServiceError {
  if (typeof raw !== 'object' || raw === null) {
    return new PrismServiceError('棱镜业务服务返回了格式不对的错误', 'protocol')
  }
  const error = raw as RpcErrorObject
  const message = typeof error.message === 'string' ? error.message : '棱镜业务服务返回了错误'
  const rpcCode = typeof error.code === 'number' ? error.code : undefined
  const dataCode = error.data?.code
  const code = typeof dataCode === 'string'
    ? dataCode
    : rpcCode === METHOD_NOT_FOUND ? 'method_not_found' : 'protocol'
  return new PrismServiceError(message, code, rpcCode, error.data?.detail ?? null)
}
