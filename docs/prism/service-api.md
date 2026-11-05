# prism-service 接口

版本：0.1（2026-11-05，D2 已实现）
对应设计：`design.md` 第 4 节

prism-service 是棱镜的 Rust 业务服务（商品库、规则包、表格导入导出、图片处理、生图调度），由 Host 插件 `@prism/dsh-host-service` 作为子进程启动，通过 stdin / stdout 上的 JSON-RPC 调用。

## 1. 进程

| 项 | 规定 |
|---|---|
| 可执行文件 | `prism-service.exe`（Windows），开发时在 `native/prism-service/target/<profile>/`，打包后在平台包 `@prism/service-win32-x64` 里（D5） |
| 启动参数 | `prism-service --data-dir <目录>`；`--log-level <error\|warn\|info\|debug>` 可选，默认 `info` |
| 数据目录 | 存放 SQLite 数据库 `prism.db`、素材文件、用户规则包；不存在时自动创建 |
| 目录锁 | 启动时在数据目录下独占 `prism-service.lock`；拿不到锁说明已有服务在用这个目录，以退出码 3 退出，stderr 写明原因 |
| 退出 | stdin 关闭（Host 退出或主动断开）或收到 `service.shutdown` 时，停止接收新请求，等进行中的请求结束（最多 10 秒），然后退出，退出码 0 |
| 日志 | 写 stderr，每行一条；Host 插件转到 dsh 的日志里。不写 API Key、图片内容 |
| 崩溃重启 | Host 插件负责：非正常退出后 1 秒重启；连续 3 次在 30 秒内退出就不再重启，并报告错误 |

退出码：

| 码 | 含义 |
|---|---|
| 0 | 正常退出 |
| 2 | 启动参数错误 |
| 3 | 数据目录被占用（拿不到目录锁） |
| 4 | 数据目录无法创建或数据库打不开（含迁移失败） |
| 1 | 其他错误 |

## 2. 协议

- JSON-RPC 2.0，每条消息一行（`\n` 分隔，UTF-8，消息内不含换行）
- Host → 服务：请求（带 `id`）；服务 → Host：响应和通知（不带 `id`）
- 服务可以并发处理多个请求，响应顺序不保证和请求顺序一致，按 `id` 对应
- 单条消息最大 16 MB；超过的请求返回 `-32600`（`id` 为 `null`），服务继续处理后面的消息
- 不支持批量请求（JSON 数组）和按位置传参（`params` 是数组），分别返回 `-32600`、`-32602`
- 没有 `id` 的请求按通知处理：照常执行，不回响应；空行忽略

请求：

```json
{"jsonrpc":"2.0","id":1,"method":"service.ping","params":{}}
```

成功响应：

```json
{"jsonrpc":"2.0","id":1,"result":{"pong":true}}
```

失败响应：`error.data` 是棱镜的错误对象，`message` 是给用户看的中文说明：

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"商品不存在","data":{"code":"not_found","detail":null}}}
```

错误码：

| `error.code` | 含义 | `error.data.code` |
|---|---|---|
| -32700 | 消息不是合法 JSON | — |
| -32600 | 不是合法的 JSON-RPC 请求，或消息过大 | — |
| -32601 | 方法不存在 | — |
| -32602 | 参数不对（缺字段、类型不对） | `invalid_input` |
| -32001 | 业务错误 | 原仓库 `ApiErrorCode`：`invalid_input`、`not_found`、`storage`、`upstream` 等 |
| -32603 | 服务内部错误 | `internal` |

通知（服务 → Host，没有 `id`）：

```json
{"jsonrpc":"2.0","method":"image.batchProgress","params":{"batchId":"…","succeeded":3,"failed":0,"total":10}}
```

## 3. 方法

### 3.1 `service.*`（D2）

| 方法 | 参数 | 返回 |
|---|---|---|
| `service.ping` | `{}` | `{ pong: true }` |
| `service.version` | `{}` | `{ version, dataDir, schemaVersion }`：服务版本、数据目录、数据库迁移到的版本 |
| `service.shutdown` | `{}` | `{}`；返回后开始退出（见第 1 节） |

`service.*` 不需要参数，传了也忽略。

### 3.2 业务方法（D3、D4 实现时补全）

沿用原仓库 `docs/api.md` 里已实现、测试过的命令，名称改成 `分组.方法` 的驼峰形式，参数和返回值保持一致：

| 分组 | 范围 | 计划 |
|---|---|---|
| `catalog.*` | 店铺、商品、规格、素材、文案、表格导入商品 | D3 |
| `rules.*` | 规则包列表、导入、删除用户规则 | D3 |
| `search.*` | 商品、素材、批次搜索 | D3 |
| `image.*` | 生图配置、提交批次、批次状态、取消、结果 | D4 |
| `imaging.*` | 缩放、裁切、白底检查、排版 | D4 |

## 4. Host 侧：`ctx.prismService`

Host 插件 `@prism/dsh-host-service`（`packages/prism/host-service`）负责进程管理，其他插件通过它调用服务：

```ts
const version = await ctx.prismService.call<{ version: string }>('service.version', {}, { timeoutMs: 5000 })
ctx.prismService.status() // { state: 'running', exits: 0 }
```

| 项 | 规定 |
|---|---|
| 可执行文件 | 配置 `command` → 环境变量 `PRISM_SERVICE_BIN` → 从插件所在目录往上找 `native/prism-service/target/{release,debug}/`，两个都有时用较新的 |
| 数据目录 | 配置 `dataDir`，默认是 Harness home 下的 `prism/` |
| 其他配置 | `logLevel`（默认 `info`）、`callTimeoutMs`（默认 30000）；`args` 放在服务参数前面，只给测试用 |
| 就绪 | 每次启动后先调 `service.ping`，15 秒内没响应就结束进程走重启；启动或重启期间发起的调用等服务就绪，等待时间算在超时里 |
| 状态 | `starting` → `running`；意外退出后 `restarting`；30 秒内退出 3 次或找不到可执行文件时 `failed`，不再重启；插件卸载后 `stopped` |
| 卸载 | 关闭服务的 stdin，等服务处理完进行中的请求自己退出，12 秒后还没退出就强制结束 |
| 日志 | 服务的 stderr 按行转到 Host 日志，按 tracing 的级别分 error / warn / info / debug；最近一条 ERROR 会写进退出时的错误说明 |

调用失败时抛 `PrismServiceError`，`message` 是中文说明，`code` 是分类：

| `code` | 来源 |
|---|---|
| `invalid_input`、`not_found`、`storage`、`upstream` 等 | 服务返回的 `error.data.code`（`rpcCode` 是 -32001、-32602 或 -32603，`detail` 是 `error.data.detail`） |
| `method_not_found` | 服务返回 -32601 |
| `protocol` | 消息格式不对；Host 收到无法解析的输出时还会结束服务进程并重启 |
| `timeout` | 超时（服务那边的处理不会被打断） |
| `exited` | 调用期间服务进程退出了 |
| `unavailable` | 服务没在运行：找不到可执行文件、多次崩溃后放弃、插件已停止 |

取消（`signal`）时 Promise 以 `signal.reason` 拒绝；取消原因不是 Error 时包成 `code` 为 `aborted` 的 `PrismServiceError`。

发布版的 Rust 程序 `panic = "abort"`，处理请求时程序出错会直接退出进程，由 Host 重启；进行中的调用得到 `exited`。
