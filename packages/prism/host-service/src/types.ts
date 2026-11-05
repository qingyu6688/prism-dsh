/**
 * `ctx.prismService` 对外的类型。
 * @module @prism/dsh-host-service/types
 */

/**
 * 服务进程的状态。
 * - `starting`：第一次启动，还没确认能响应
 * - `running`：已经响应过 `service.ping`
 * - `restarting`：意外退出，等待重启
 * - `failed`：30 秒内退出 3 次，或找不到可执行文件，不再重启
 * - `stopped`：插件已卸载
 */
export type PrismServiceState = 'starting' | 'running' | 'restarting' | 'failed' | 'stopped'

/** 当前状态快照。 */
export interface PrismServiceStatus {
  state: PrismServiceState
  /** 启动以来意外退出的次数。 */
  exits: number
  /** 最近一次出错的说明，给界面和日志看。 */
  lastError?: string
}

/** 单次调用的选项。 */
export interface PrismCallOptions {
  /** 取消调用；取消后 Promise 以 `signal.reason` 拒绝（不是 Error 时包成 `aborted` 错误），服务端的处理不会被打断。 */
  signal?: AbortSignal
  /** 超时（毫秒），默认用插件配置的 `callTimeoutMs`。等服务启动的时间也算在内。 */
  timeoutMs?: number
}

/**
 * 调用失败的分类。
 * - 服务返回的业务错误：`invalid_input`、`not_found`、`storage`、`upstream` 等（`docs/prism/service-api.md`）
 * - `method_not_found`：服务没有这个方法
 * - `protocol`：消息格式不对
 * - `timeout`：超时
 * - `exited`：调用期间服务进程退出了
 * - `unavailable`：服务没有在运行（启动失败、已停止）
 * - `aborted`：调用方取消，且取消原因不是 Error
 */
export type PrismErrorCode = string
