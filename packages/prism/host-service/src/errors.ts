/**
 * 调用 prism-service 失败时的错误类型。
 * @module @prism/dsh-host-service/errors
 */

import type { PrismErrorCode } from './types.ts'

/** 调用失败。`message` 是给用户看的中文说明，分类看 `code`。 */
export class PrismServiceError extends Error {
  override readonly name = 'PrismServiceError'

  /**
   * @param message - 给用户看的说明。
   * @param code - 失败分类，见 {@link PrismErrorCode}。
   * @param rpcCode - 服务返回的 JSON-RPC 错误码；本地产生的错误（超时、进程退出）没有。
   * @param detail - 服务返回的 `error.data.detail`。
   */
  constructor(
    message: string,
    readonly code: PrismErrorCode,
    readonly rpcCode?: number,
    readonly detail?: unknown,
  ) {
    super(message)
  }
}

/**
 * 取消调用时的错误：`signal.reason` 是 Error 就用它，否则包一个 `aborted` 错误。
 * @param signal - 已经触发的取消信号。
 * @returns 拒绝调用用的错误。
 */
export function abortError(signal: AbortSignal): Error {
  const reason: unknown = signal.reason
  return reason instanceof Error ? reason : new PrismServiceError('调用已取消', 'aborted')
}

/**
 * 服务没在运行时的错误。
 * @param reason - 为什么不可用。
 * @returns `unavailable` 错误。
 */
export function unavailable(reason: string): PrismServiceError {
  return new PrismServiceError(`棱镜业务服务不可用：${reason}`, 'unavailable')
}
