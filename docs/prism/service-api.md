# prism-service 接口

版本：0.3（2026-11-05，含 D4 生图接口）。对应设计：[系统设计](design.md)第 4、13 节。

prism-service 是棱镜的 Rust 业务服务（商品库、规则包、表格导入导出、图片处理、生图调度），由 Host 插件 `@prism/dsh-host-service` 作为子进程启动，通过 stdin / stdout 上的 JSON-RPC 调用。

## 1. 进程

| 项 | 规定 |
|---|---|
| 可执行文件 | `prism-service.exe`（Windows），开发时在 `native/prism-service/target/<profile>/`，打包后在平台包 `@prism/service-win32-x64` 里（D5） |
| 启动参数 | `prism-service --data-dir <目录>`；`--log-level <error\|warn\|info\|debug>` 可选，默认 `info` |
| 数据目录 | 存放 SQLite 数据库 `prism.db`、素材文件、用户规则包；不存在时自动创建 |
| 目录锁 | 启动时在数据目录下独占 `prism-service.lock`；拿不到锁说明已有服务在用这个目录，以退出码 3 退出，stderr 写明原因 |
| 退出 | stdin 关闭（Host 退出或主动断开）或收到 `service.shutdown` 时，停止接收新请求，等进行中的请求结束（最多 10 秒），再停止生图引擎（最多 30 秒），然后退出，退出码 0 |
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
{"jsonrpc":"2.0","method":"image.changed","params":{"batchId":"…","succeeded":3,"failed":0,"total":10}}
```

## 3. 方法

### 3.1 `service.*`（D2）

| 方法 | 参数 | 返回 |
|---|---|---|
| `service.ping` | `{}` | `{ pong: true }` |
| `service.version` | `{}` | `{ version, dataDir, schemaVersion }`：服务版本、数据目录、数据库迁移到的版本 |
| `service.shutdown` | `{}` | `{}`；返回后开始退出（见第 1 节） |

`service.*` 不需要参数，传了也忽略。

### 3.2 商品库、规则包、搜索（D3）

沿用原仓库 `docs/api.md` 1.7、1.8、1.10 里已实现、测试过的命令，参数和返回值保持一致（驼峰字段），只是命令名改成 `分组.方法`。分页结果统一是 `{ items, total }`，`limit` 不传或为 0 时 50 条，最多 200 条。

| 方法 | 参数 | 返回 | 原命令 |
|---|---|---|---|
| `catalog.listShops` | `{ platform? }` | `Shop[]` | `list_shops` |
| `catalog.createShop` | `{ input: ShopInput }` | `Shop` | `create_shop` |
| `catalog.updateShop` | `{ id, input: ShopInput }` | `Shop` | `update_shop` |
| `catalog.deleteShop` | `{ id }` | `{}` | `delete_shop` |
| `catalog.listProducts` | `{ filter: { keyword?, archived, limit, offset } }` | `Page<Product>` | `list_products` |
| `catalog.productDetail` | `{ id }` | `ProductDetail` | `get_product_detail` |
| `catalog.createProduct` | `{ input: ProductInput }` | `Product` | `create_product` |
| `catalog.updateProduct` | `{ id, input: ProductInput }` | `Product` | `update_product` |
| `catalog.archiveProduct` / `restoreProduct` / `deleteProduct` | `{ id }` | `{}` | 同名 |
| `catalog.previewProductImport` | `{ path, sheet?, headerLine? }` | `ImportPreview` | `preview_product_import` |
| `catalog.importProducts` | `{ path, sheet?, headerLine?, mapping, mode, saveAs?, matchedMappingId? }` | `ImportResult` | `import_products` |
| `catalog.listImportMappings` | `{}` | `SavedMapping[]` | `list_import_mappings` |
| `catalog.deleteImportMapping` | `{ id }` | `{}` | `delete_import_mapping` |
| `catalog.listAssets` | `{ filter: { productId?, kind?, source?, tag?, keyword?, starredOnly, limit, offset } }` | `Page<Asset>` | `list_assets` |
| `catalog.listAssetTags` | `{}` | `{ tag, count }[]` | `list_asset_tags`（原来是二元组） |
| `catalog.importAssetFiles` | `{ paths, productId?, tags }` | `{ imported: { asset, deduplicated }[], failed: { path, message }[] }`，一个文件失败不影响其他 | `import_asset_files` |
| `catalog.setAssetTags` | `{ id, tags }` | `Asset` | `set_asset_tags` |
| `catalog.starAsset` | `{ id, starred }` | `{}` | `star_asset` |
| `catalog.linkAsset` | `{ id, productId? }` | `{}` | `link_asset` |
| `catalog.deleteAsset` | `{ id }` | `{}` | `delete_asset` |
| `rules.list` | `{}` | `{ packs, wordLists, exportTemplates, problems }` | `list_rules` |
| `rules.importFile` | `{ path }` | `{ kind, id, revision }`；校验不通过时 `detail` 是问题列表 `[{ path, message }]` | `import_rule_file` |
| `rules.removeUser` | `{ kind, id }` | `{ removed }` | `remove_user_rule` |
| `search.global` | `{ keyword }` | `{ products, assets, batches }`；商品和素材各最多 5 条，数字或 `#数字` 精确匹配生图批次号；空关键词返回空组 | `global_search`（会话由 dsh 管理） |

和原命令的差别：

- 素材里的路径：`filePath`、`thumbPath` 是相对数据目录的路径（界面用 Host 的 `/prism/files/` 路由显示）；不再返回 `absoluteFilePath`、`absoluteThumbPath`。
- 规则文件放在数据目录的 `rules/` 下，服务启动时加载，导入、删除后重新加载。
- 导入表格、图片、规则文件时，`path` 是 Host 上的路径（界面先上传到 Host，见 `design.md` 第 12 节）。

### 3.3 生图与系统凭据

| 方法 | 参数 | 返回 |
|---|---|---|
| `image.listProfiles` | `{}` | 配置数组；含 `hasKey`，不含密钥 |
| `image.saveProfile` | `{ id?, name, baseUrl, model, providerType, apiKey?, maxConcurrency, timeoutSecs }` | 保存后的配置；编辑时密钥为空则保留 |
| `image.deleteProfile` | `{ id }` | `{}`；删除配置及系统密钥 |
| `image.testProfile` | `{ id }` | `{ supported, models }`；查询模型列表，不产生生图费用 |
| `image.submitBatch` | `{ owner, args: { prompt, count?, referenceIds?, productId?, size? }, fission? }` | 批次状态；`owner` 是 dsh Session ID |
| `image.batchStatus` / `image.cancelBatch` | `{ id }` | 批次状态 |
| `image.listBatches` | `{}` | 最近 100 个批次中的生图批次，倒序 |
| `credentials.get` / `set` / `delete` | `{ key, value? }` | 值或 `null`；写、删返回 `{}` |
| `credentials.list` | `{}` | 凭据名称索引，不含值 |

生图配置支持 `openai_images` 和 `async_task`。接口地址必须是无凭据、无查询参数的 HTTP(S) 地址；并发 1～64，超时 10～1800 秒。`async_task` 使用 `prism-provider` 的默认提交和轮询协议，任意厂商的自定义协议尚未开放配置。

批次返回 `{ id, seq, owner, status, total, succeeded, failed, reason, assets, errors }`。素材路径相对业务数据目录，原图和缩略图通过 `/api/prism/file?path=…` 读取。每张图对应一个任务，数量 1～200，图裂变必须提供参考素材 ID；不接受模型传任意本地文件路径。用户删除素材后，历史批次只显示仍存在的图片，完成计数不变。取消后保留已生成素材；已经发给上游的请求可能仍产生费用。

`image.submitBatch` 和 `credentials.*` 不暴露为浏览器 Remote。生图工具先通过 dsh 审批和后台任务管理，再提交 Rust 批次；超过 40 张必须得到一次性许可。工具返回 `{ jobId, batchId, count, message }`，结果卡片从已记录的 `batchId` 读取图片，后台任务完成结果进入原会话。引擎恢复持久化任务后可在结果页查看；dsh 内存中的任务观察记录不跨进程恢复。

引擎通知统一为 `image.changed`，`params` 是 `EngineEvent`。通知可能合并或因订阅落后而丢弃，数据库查询才是完整状态。界面每 2 秒刷新，后台任务默认每秒查询，可通过工具插件的 `imagePollIntervalMs` 调整。

凭据由本机系统凭据管理器保存，以业务数据目录的规范路径摘要区分命名空间；SQLite 只存名称索引。文本模型使用 `ref/…` 和 `record/…`，生图使用 `image/<配置 ID>`。这组 RPC 只供可信的本地 Host 使用，不是网络密钥导出接口。

### 3.4 暂未接入的图片处理

`imaging.*` 的缩放、裁切、白底检查和排版仍未注册到 RPC，后续作图模块单独接入。

## 4. Host 侧：`ctx.prismService`

Host 插件 `@prism/dsh-host-service`（`packages/prism/host-service`）负责进程管理，其他插件通过它调用服务：

```ts
import type { Context } from '@deepseek-ai/cordis'
import type {} from '@prism/dsh-host-service'

export async function readServiceVersion(ctx: Context): Promise<string> {
  const result = await ctx.prismService.call<{ version: string }>('service.version', {}, { timeoutMs: 5000 })
  return result.version
}
```

| 项 | 规定 |
|---|---|
| 可执行文件 | 配置 `command` → 环境变量 `PRISM_SERVICE_BIN` → 从插件所在目录往上找 `native/prism-service/target/{release,debug}/`，两个都有时用较新的 |
| 数据目录 | 配置 `dataDir`，默认是 Harness home 下的 `prism/` |
| 其他配置 | `logLevel`（默认 `info`）、`callTimeoutMs`（默认 30000）、`shutdownGraceMs`（默认 45000）；`args` 放在服务参数前面，只给测试用 |
| 就绪 | 每次启动后先调 `service.ping`，15 秒内没响应就结束进程走重启；启动或重启期间发起的调用等服务就绪，等待时间算在超时里 |
| 状态 | `starting` → `running`；意外退出后 `restarting`；30 秒内退出 3 次或找不到可执行文件时 `failed`，不再重启；插件卸载后 `stopped` |
| 卸载 | 关闭服务的 stdin，等服务处理完进行中的请求自己退出，默认 45 秒后还没退出就强制结束（`shutdownGraceMs`）；服务先排空请求，随后等待生图引擎退出 |
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
