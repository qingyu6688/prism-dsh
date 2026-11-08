/**
 * 棱镜业务服务的 Host 插件（`ctx.prismService`）。
 *
 * 插件加载时通过 `ctx.subprocess` 拉起 `prism-service` 可执行文件，以 stdin / stdout 上的
 * JSON-RPC 调用它（`docs/prism/service-api.md`）。服务意外退出后 1 秒重启；30 秒内退出 3 次就
 * 不再重启，状态变为 `failed`。插件卸载时关闭服务的 stdin，服务处理完进行中的请求后自己退出。
 *
 * 启动失败不会让插件加载失败：dsh 的其他功能照常可用，调用 `call` 时才报"服务不可用"。
 * @module @prism/dsh-host-service
 */

import { Context, Service } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { existsSync, mkdirSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { Readable } from 'node:stream'
import { resolveDshHome } from '@deepseek-ai/dsh-home-paths'
import type { SubprocessHandle, SubprocessOutcome } from '@deepseek-ai/dsh-subprocess'
import { RpcConnection } from './connection.ts'
import { PrismServiceError, abortError, unavailable } from './errors.ts'
import type { PrismCallOptions, PrismServiceState, PrismServiceStatus } from './types.ts'

export { PrismServiceError } from './errors.ts'
export { RpcConnection } from './connection.ts'
export type { PrismCallOptions, PrismErrorCode, PrismServiceState, PrismServiceStatus } from './types.ts'

declare module '@deepseek-ai/cordis' {
  interface Context {
    /** 棱镜业务服务：商品库、规则包、图片处理等。 */
    prismService: PrismService
  }
}

/** 意外退出后等多久重启。 */
const RESTART_DELAY_MS = 1_000
/** 在这个时间窗口内退出 {@link MAX_EXITS} 次就不再重启。 */
const EXIT_WINDOW_MS = 30_000
const MAX_EXITS = 3
/** 启动后等 `service.ping` 响应的时间，超过就结束进程走重启。 */
const STARTUP_TIMEOUT_MS = 15_000
/** 服务端单条消息上限是 16 MB，Host 这边对响应用同样的上限。 */
const MAX_MESSAGE_BYTES = 16 * 1024 * 1024
/** 转发到日志的 stderr 单行长度上限。 */
const MAX_LOG_LINE = 8_192
const MAX_TIMER_MS = 2_147_483_647

const BINARY_NAME = process.platform === 'win32' ? 'prism-service.exe' : 'prism-service'

/** 服务退出码的含义，和 `docs/prism/service-api.md` 第 1 节一致。 */
const EXIT_MEANINGS: Record<number, string> = {
  2: '启动参数错误',
  3: '数据目录被另一个服务进程占用',
  4: '数据目录或数据库打不开',
}

type LogLevel = 'error' | 'warn' | 'info' | 'debug'

/** 插件配置。 */
export interface Config {
  /** prism-service 可执行文件路径；不填时依次找环境变量 `PRISM_SERVICE_BIN`、仓库里 `native/prism-service/target` 下的构建产物。 */
  command?: string
  /** 放在服务参数前面的参数，开发调试用（例如 `command` 是 node、这里是脚本路径）。 */
  args: string[]
  /** 数据目录，默认是 Harness home 下的 `prism`。 */
  dataDir?: string
  /** 服务的日志级别。 */
  logLevel: LogLevel
  /** 默认调用超时（毫秒）。 */
  callTimeoutMs: number
  /** 关闭输入后等待请求排空和生图引擎退出的时间，单位毫秒。 */
  shutdownGraceMs: number
}

interface Deferred<T> {
  readonly promise: Promise<T>
  settled: boolean
  resolve(value: T): void
  reject(error: Error): void
}

function deferred<T>(): Deferred<T> {
  let resolvePromise!: (value: T) => void
  let rejectPromise!: (error: Error) => void
  const promise = new Promise<T>((resolve, reject) => {
    resolvePromise = resolve
    rejectPromise = reject
  })
  // 没有调用在等的时候被拒绝也不算未处理：真正等待的调用方各自会拿到这个拒绝
  promise.then(undefined, () => undefined)
  const result: Deferred<T> = {
    promise,
    settled: false,
    resolve(value) {
      if (result.settled) return
      result.settled = true
      resolvePromise(value)
    },
    reject(error) {
      if (result.settled) return
      result.settled = true
      rejectPromise(error)
    },
  }
  return result
}

/** 一次启动的进程和它的连接。 */
interface Generation {
  readonly handle: SubprocessHandle
  readonly connection: RpcConnection
  /** 最近一条 ERROR 级别的 stderr，进程退出时放进错误说明。 */
  lastErrorLine?: string
}

/**
 * 找 prism-service 可执行文件。
 * @param configured - 配置里写的路径。
 * @param env - 环境变量。
 * @param from - 从哪个目录往上找仓库里的构建产物，默认是本模块所在目录。
 * @returns 可执行文件路径；都找不到时返回 `undefined`。
 */
export function resolveServiceCommand(
  configured: string | undefined,
  env: Record<string, string | undefined> = process.env,
  from: string = dirname(fileURLToPath(import.meta.url)),
): string | undefined {
  if (configured !== undefined && configured !== '') return configured
  const fromEnv = env.PRISM_SERVICE_BIN
  if (fromEnv !== undefined && fromEnv !== '') return fromEnv
  // 开发时用 cargo 构建的产物；release 和 debug 都在时用较新的那个
  for (let dir = from; ; dir = dirname(dir)) {
    const target = join(dir, 'native', 'prism-service', 'target')
    if (existsSync(target)) {
      const builds = ['release', 'debug']
        .map(profile => join(target, profile, BINARY_NAME))
        .filter(path => existsSync(path))
        .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)
      return builds[0]
    }
    if (dirname(dir) === dir) return undefined
  }
}

function describeExit(outcome: SubprocessOutcome | undefined, error: unknown): string {
  if (outcome === undefined) return `启动失败：${error instanceof Error ? error.message : String(error)}`
  if (outcome.exitCode === null) return `被信号 ${String(outcome.signal)} 结束`
  const meaning = EXIT_MEANINGS[outcome.exitCode]
  return meaning === undefined ? `退出码 ${outcome.exitCode}` : `退出码 ${outcome.exitCode}，${meaning}`
}

/** 按行读一个流，单行过长时截断。 */
function forEachLine(stream: Readable, onLine: (line: string) => void): void {
  let rest = ''
  stream.setEncoding('utf8')
  stream.on('data', (chunk: string) => {
    const lines = (rest + chunk).split('\n')
    rest = (lines.pop() ?? '').slice(-MAX_LOG_LINE)
    for (const line of lines) onLine(line.slice(0, MAX_LOG_LINE))
  })
  stream.on('end', () => {
    if (rest !== '') onLine(rest)
  })
}

/** `ctx.prismService`：调用棱镜业务服务。 */
export class PrismService extends Service {
  static inject = ['subprocess']

  static Config: z<Config> = z.object({
    command: z.string(),
    args: z.array(String).default([]),
    dataDir: z.string(),
    logLevel: z.union(['error', 'warn', 'info', 'debug'] as const).default('info'),
    shutdownGraceMs: z.number().min(1).max(MAX_TIMER_MS).default(45_000),
    callTimeoutMs: z.number().min(1).max(MAX_TIMER_MS).default(30_000),
  })

  private state: PrismServiceState = 'starting'
  private exits = 0
  private recentExits: number[] = []
  private lastError: string | undefined
  private current: Generation | undefined
  private ready = deferred<RpcConnection>()
  private restartTimer: ReturnType<typeof setTimeout> | undefined
  private disposed = false
  private resolvedDataDir: string | undefined

  /**
   * @param ctx - 插件上下文，需要 `subprocess`。
   * @param config - 插件配置。
   */
  constructor(ctx: Context, private readonly config: Config) {
    super(ctx, 'prismService')
  }

  protected [Service.init](): void {
    this.ctx.effect(() => () => this.stop(), 'prismService: stop process')
    const command = resolveServiceCommand(this.config.command)
    if (command === undefined || !existsSync(command)) {
      this.fail(command === undefined
        ? `没找到 ${BINARY_NAME}，开发时先在 native/prism-service 下运行 cargo build`
        : `可执行文件不存在：${command}`)
      return
    }
    const dataDir = this.config.dataDir ?? join(resolveDshHome(), 'prism')
    this.resolvedDataDir = dataDir
    try {
      mkdirSync(dataDir, { recursive: true })
    } catch (error) {
      this.fail(`无法创建数据目录 ${dataDir}：${(error as Error).message}`)
      return
    }
    this.launch(command, dataDir)
  }

  /**
   * 服务使用的数据目录（素材文件、上传的文件都在这下面）。
   * @returns 绝对路径；找不到可执行文件、还没确定数据目录时为 `undefined`。
   */
  dataDir(): string | undefined {
    return this.resolvedDataDir
  }

  /**
   * 当前状态。
   * @returns 状态快照。
   */
  status(): PrismServiceStatus {
    return {
      state: this.state,
      exits: this.exits,
      ...this.lastError === undefined ? {} : { lastError: this.lastError },
    }
  }

  /**
   * 调用服务的一个方法。服务正在启动或重启时先等它就绪。
   * @param method - 方法名，如 `service.version`。
   * @param params - 参数对象，默认 `{}`。
   * @param options - 取消信号和超时。
   * @returns 服务返回的 `result`。
   * @throws {PrismServiceError} 调用失败，分类见 `code`。
   */
  async call<T = unknown>(method: string, params: object = {}, options: PrismCallOptions = {}): Promise<T> {
    const timeoutMs = options.timeoutMs ?? this.config.callTimeoutMs
    const startedAt = Date.now()
    const connection = await this.waitReady(timeoutMs, options.signal)
    const remaining = Math.max(1, timeoutMs - (Date.now() - startedAt))
    return await connection.call(method, params, remaining, options.signal) as T
  }

  private waitReady(timeoutMs: number, signal: AbortSignal | undefined): Promise<RpcConnection> {
    if (signal?.aborted === true) return Promise.reject(abortError(signal))
    const ready = this.ready.promise
    return new Promise((resolve, reject) => {
      const finish = (settle: () => void): void => {
        clearTimeout(timer)
        signal?.removeEventListener('abort', onAbort)
        settle()
      }
      const onAbort = (): void => {
        if (signal !== undefined) finish(() => { reject(abortError(signal)) })
      }
      const timer = setTimeout(() => {
        finish(() => { reject(new PrismServiceError('等待棱镜业务服务启动超时', 'timeout')) })
      }, timeoutMs)
      signal?.addEventListener('abort', onAbort, { once: true })
      ready.then(
        (connection) => { finish(() => { resolve(connection) }) },
        (error: unknown) => { finish(() => { reject(error instanceof Error ? error : new Error(String(error))) }) },
      )
    })
  }

  private launch(command: string, dataDir: string): void {
    this.state = this.exits === 0 ? 'starting' : 'restarting'
    let handle: SubprocessHandle
    try {
      handle = this.ctx.subprocess.spawn({
        argv: [command, ...this.config.args, '--data-dir', dataDir, '--log-level', this.config.logLevel],
        cwd: dataDir,
        stdio: { stdin: 'pipe', stdout: 'pipe', stderr: 'pipe' },
        graceMs: 2_000,
      })
    } catch (error) {
      this.onExit(undefined, undefined, error, command, dataDir)
      return
    }
    /* v8 ignore start -- 'pipe' 模式下子进程服务保证三个流都在，这里只是防御 */
    if (handle.stdin === undefined || handle.stdout === undefined || handle.stderr === undefined) {
      handle.terminate()
      this.fail('子进程服务没有提供 stdin / stdout / stderr')
      return
    }
    /* v8 ignore stop */
    const connection = new RpcConnection(handle.stdin, handle.stdout, MAX_MESSAGE_BYTES, {
      onNotification: (method) => { this.ctx.logger.debug('prism-service 通知 %s', method) },
      onFatal: (error) => {
        this.ctx.logger.warn('prism-service 连接出错，结束进程：%s', error.message)
        // 进程还没退出前，新调用不能再拿到这个已经坏掉的连接，改为等下一次启动
        if (this.current === generation && this.ready.settled) {
          this.ready = deferred()
          this.state = 'restarting'
        }
        handle.terminate()
      },
    })
    const generation: Generation = { handle, connection }
    this.current = generation
    forEachLine(handle.stderr, (line) => { this.logServiceLine(generation, line) })

    connection.call('service.ping', {}, STARTUP_TIMEOUT_MS).then(
      () => {
        if (this.current !== generation) return
        this.state = 'running'
        this.ready.resolve(connection)
        this.ctx.logger.info('prism-service 已就绪，数据目录 %s', dataDir)
      },
      (error: unknown) => {
        if (this.current !== generation) return
        this.ctx.logger.warn('prism-service 启动后没有响应，结束进程：%s', (error as Error).message)
        handle.terminate()
      },
    )
    handle.done.then(
      (outcome) => { this.onExit(generation, outcome, undefined, command, dataDir) },
      (error: unknown) => { this.onExit(generation, undefined, error, command, dataDir) },
    )
  }

  private logServiceLine(generation: Generation, line: string): void {
    const text = line.trimEnd()
    if (text === '') return
    // tracing 的默认格式：时间  级别 消息
    const level = /^\S+\s+(ERROR|WARN|INFO|DEBUG|TRACE)\s/.exec(text)?.[1]
    if (level === 'ERROR') {
      generation.lastErrorLine = text.replace(/^\S+\s+ERROR\s+/, '')
      this.ctx.logger.error('[prism-service] %s', text)
    } else if (level === 'WARN') {
      this.ctx.logger.warn('[prism-service] %s', text)
    } else if (level === 'DEBUG' || level === 'TRACE') {
      this.ctx.logger.debug('[prism-service] %s', text)
    } else {
      this.ctx.logger.info('[prism-service] %s', text)
    }
  }

  private onExit(
    generation: Generation | undefined,
    outcome: SubprocessOutcome | undefined,
    error: unknown,
    command: string,
    dataDir: string,
  ): void {
    if (generation !== undefined && generation !== this.current) return
    this.current = undefined
    const reason = describeExit(outcome, error)
    const detail = generation?.lastErrorLine === undefined ? reason : `${reason}（${generation.lastErrorLine}）`
    generation?.connection.close(new PrismServiceError(`棱镜业务服务已退出：${detail}`, 'exited'))
    if (this.disposed) {
      this.state = 'stopped'
      return
    }

    this.exits++
    this.lastError = detail
    const now = Date.now()
    this.recentExits = [...this.recentExits.filter(at => now - at < EXIT_WINDOW_MS), now]
    if (this.ready.settled) this.ready = deferred()
    if (this.recentExits.length >= MAX_EXITS) {
      this.fail(`${EXIT_WINDOW_MS / 1000} 秒内退出了 ${MAX_EXITS} 次，不再重启。最后一次：${detail}`)
      return
    }
    this.ctx.logger.warn('prism-service 意外退出（%s），%d 毫秒后重启', detail, RESTART_DELAY_MS)
    this.state = 'restarting'
    this.restartTimer = setTimeout(() => {
      this.restartTimer = undefined
      if (!this.disposed) this.launch(command, dataDir)
    }, RESTART_DELAY_MS)
  }

  private fail(reason: string): void {
    this.state = 'failed'
    this.lastError = reason
    this.ready.reject(unavailable(reason))
    this.ctx.logger.error('prism-service 不可用：%s', reason)
  }

  private async stop(): Promise<void> {
    this.disposed = true
    if (this.restartTimer !== undefined) clearTimeout(this.restartTimer)
    if (this.ready.settled) this.ready = deferred()
    this.ready.reject(unavailable('插件已停止'))
    const generation = this.current
    if (generation === undefined) {
      this.state = 'stopped'
      return
    }
    // 关闭 stdin 后服务会处理完进行中的请求再退出；超时就强制结束
    generation.handle.stdin?.end()
    const timer = setTimeout(() => { generation.handle.terminate() }, this.config.shutdownGraceMs)
    try {
      await generation.handle.done
    } catch (error) {
      this.ctx.logger.warn('prism-service 停止时出错：%s', (error as Error).message)
    } finally {
      clearTimeout(timer)
    }
    this.state = 'stopped'
  }
}

export default PrismService
