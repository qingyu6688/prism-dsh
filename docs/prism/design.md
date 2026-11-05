# 棱镜建在 DeepSeek Harness 上的设计

版本：0.1（2026-11-04，已确认）
底座：deepseek-harness `fcc0ad4621`（dsh 0.2.2-rc.1，MIT）

> 2026-11-04 的决定：
>
> - fork dsh 作为棱镜的底座，前端、后端、桌面壳都用 dsh；
> - 新代码放在 `D:\Claude-Work\prism-dsh`，原来的 Tauri 版仓库 `D:\Claude-Work\prism` 保留不动；
> - 现有 Rust 业务代码按 dsh 处理原生程序的方式，编成独立服务程序 `prism-service`，由插件通过 JSON-RPC 调用（方案 A）。
>
> 调研见原仓库 `docs/dsh-desktop-build-research.md`。

## 1. 原则

1. **加东西，少改 dsh**：棱镜的功能做成新包（插件、组合包、品牌），尽量不改 dsh 原有文件，方便以后合并上游更新。必须改的地方在第 9 节逐条登记。
2. **dsh 有的就用 dsh 的**：会话、对话界面、流式输出、附件、排队与中途补充、停止、上下文压缩、文本模型接入、审批、后台任务，都不再自己写。
3. **业务逻辑留在 Rust**：商品库、规则包、表格导入导出、图片处理、生图调度引擎原样搬进 `prism-service`，约 320 个测试一起搬过去。
4. **不对外上报**：关掉向 DeepSeek 上报的遥测、产品数据采集和反馈。shell、文件系统、终端、技能、目标、子智能体和工作流都保留（2026-11-04 决定），由棱镜的 Agent 预设统一挂载。

## 2. 整体结构

```text
棱镜.exe（Electron 壳，棱镜品牌）
  ├─ 界面：dsh Web 应用 + 棱镜界面插件（商品库、素材库、结果区、作图卡片、生图配置）
  └─ Host（Electron 的 Node 模式子进程）
       ├─ dsh 内核：会话、Agent 循环、工具流水线、审批、后台任务、文本模型（pi-ai 适配器）
       ├─ 棱镜 Host 插件：业务工具、给界面的 API 路由、prism-service 进程管理
       └─ prism-service.exe（Rust，stdio JSON-RPC）
            ├─ SQLite：商品、素材、文案、店铺、规则包、批次、任务
            ├─ 素材文件和缩略图
            └─ 生图调度引擎 → 生图上游接口
```

## 3. 仓库里放哪

| 位置 | 内容 |
|---|---|
| `packages/prism/` | 棱镜的 TypeScript 包（pnpm workspace 已覆盖 `packages/*/*`）。Host 插件与界面插件分开，按 dsh 的包检查清单写 README 和测试 |
| `packages/bundle/prism-desktop/` | 棱镜的组合包：在 dsh 基础组合上去掉不要的、加上棱镜的（第 6 节） |
| `native/prism-service/` | Rust workspace：从原仓库搬来的 `prism-core`、`prism-store`、`prism-catalog`、`prism-rules`、`prism-io`、`prism-imaging`、`prism-provider`、`prism-engine`，加一个新的 `prism-service` 可执行文件 crate |
| `native/prism-service/npm/win32-x64/` | 平台包 `@prism/service-win32-x64`，放编译好的 `prism-service.exe`，打包时解压到 asar 外（和 ripgrep 一样） |
| `docs/prism/` | 棱镜自己的文档：环境、本设计、接口、测试、更新日志 |

原仓库的 `src-tauri` 和 React 界面不搬。界面按 dsh 的插件方式重写，交互规范沿用原来的 `ui-design.md`。

## 4. prism-service（Rust 服务）

### 4.1 进程和通信

- 由 Host 插件 `@prism/host-service` 在 Host 启动时拉起，参数传数据目录；Host 退出时关闭它，崩溃后自动重启（最多连续 3 次）
- 通信：stdin / stdout 上按行分帧的 JSON-RPC 2.0；stderr 是日志
- 图片只传文件路径，不传字节
- 一个数据目录同时只允许一个服务进程（沿用 0.3.0 的目录锁）

### 4.2 接口

方法名按模块分组，参数和返回值沿用原仓库 `api.md` 1.5～1.11 里已经实现、测试过的命令，只改调用方式：

| 组 | 方法（例） | 来源 |
|---|---|---|
| `catalog.*` | `listProducts`、`productDetail`、`importProducts`、`listAssets`、`importAssetFiles`、`listShops` …… | `prism-catalog` |
| `rules.*` | `list`、`importFile`、`removeUser` | `prism-rules` |
| `io.*` | 表格预览、导出模板 | `prism-io` |
| `imaging.*` | 缩放、裁切、白底检查、SVG 排版 | `prism-imaging` |
| `image.*` | `submitBatch`、`batchStatus`、`cancelBatch`、`listProfiles`、`saveProfile`、`testProfile` | `prism-engine`、`prism-provider` |
| `search.global` | 商品、素材、批次 | 原全局搜索 |
| `service.*` | `ping`、`version`、`shutdown` | 新增 |

推送通知（服务 → Host）：`image.batchProgress`、`image.batchFinished`、`catalog.changed`。

确切的方法表单独写进 `docs/prism/service-api.md`，写服务时先定接口、后写代码。

### 4.3 数据库

沿用原仓库的迁移 0001～0006 和迁移程序。下面这些表交给 dsh，不再使用：
- 会话、消息、通知（0001、0005）；
- Agent 计划（0004）；
- 文本模型配置（0001、0006 的文本部分）。

表本身保留不删，以后再出一个迁移清理。

## 5. 功能怎么落到 dsh 上

| 原来的功能（P0～P3） | 新做法 |
|---|---|
| 会话、消息、流式输出、附件（图片和文本文件）、排队与中途补充、停止、修改消息 | dsh 自带 |
| Agent 循环、工具调用、失败恢复、上下文压缩 | dsh 自带 |
| 文本模型配置（官方、中转站，拉模型列表，测试连接） | dsh 的模型设置 + `llm-pi-ai` 适配器（OpenAI 兼容网关） |
| 全局搜索 | 会话搜索用 dsh 的；商品、素材、批次的搜索做成棱镜插件，接进 dsh 的搜索入口（细节待查） |
| 通知中心 | dsh 的会话状态和后台任务通知；不够再补 |
| 商品库、导入商品、素材库、店铺、规则包 | 棱镜界面插件：左侧导航的全局页面（和"自动化任务"页同样的挂法）+ prism-service |
| 生图配置（Images / 异步任务接口，Key） | 棱镜设置卡片 + prism-service |
| `@` 引用商品、素材 | 棱镜插件接入 dsh 的引用输入（`ui-reference`、`ui-input-trigger`） |
| Agent 能调用的商品查询工具 | 棱镜 Host 插件注册的工具 |
| 生图、图裂变 | 棱镜工具：提交批次到 prism-service，批次作为 dsh 后台任务（`jobs`）运行，可取消，完成时通知会话 |
| 计划和确认（超过 40 个任务先确认） | 去掉计划执行器（第 8 节第 1 条）；生图工具执行前按预估张数走 dsh 的审批（`user-approval` 的"询问"） |
| 结果区 | 右侧栏的一种 tab（`ctx.sidebarRightTabs`）；对话里的结果卡片用会话节点渲染器 |
| 数据目录设置 | 棱镜设置卡片，改的是 prism-service 的数据目录 |

## 6. 组合包 `prism-desktop`

在 dsh 的 `base` + `web-app` 上做 patch：

**去掉**（`packages/bundle/prism-desktop/cordis.patch.yml`，共 17 个条目）：
- 数据上报：OpenTelemetry 遥测、会话遥测、桌面产品遥测、产品数据采集、消息反馈和反馈命令、DeepSeek 插件清单上报；
- DeepSeek 账号：账号模型路由、账号设置页。账号后端暂时保留，因为 dsh 的欢迎流程启动时依赖它，做棱镜欢迎页时一起去掉；
- Cordis 调试工具：面向 dsh 开发者；
- MCP：暂不开放，需要时再打开。

**保留**：dsh 的其余全部功能，包括 shell / PowerShell 和终端、文件系统、技能、目标、子智能体、团队、工作流、计划模式、压缩、自动化任务、附件、模型设置、插件管理。

**Agent 预设**：新增 `prism` 预设（`presets/prism.patch.yml`），是 dsh standard 预设的完整副本，只改人设和计划模式提示词（中文、电商场景），设为默认；dsh 自带的预设保留，可在设置里切换。同步上游时要对照 standard 预设的变化合并过来。

**加上**（后续步骤）：棱镜的 Host 插件、界面插件、品牌插件；`llm-pi-ai` 预置 OpenAI 和常见中转站的路由模板。

桌面端的 profile 从 dsh 默认组合换成 `prism-desktop`。实测记录：产品数据采集依赖账号和遥测导出器；欢迎流程依赖账号后端；Agent 预设引用的工具依赖顶层的 shell、技能、目标、子智能体等服务，所以这些服务不能在顶层关掉。

## 7. 品牌和用户数据

| 项 | 做法 |
|---|---|
| 产品名 | 棱镜 Prism；关于页注明"基于 DeepSeek Harness 构建"（品牌规范允许）。不使用 "DeepSeek Harness" 作名称，不使用鲸鱼标志 |
| 图标、欢迎页、安装界面 | 换成棱镜的标志（原仓库 `docs/ui-design.md` 第 0 章）；新增 `ui-brand-prism` 替代 `ui-brand-official` |
| 应用标识 | electron-builder 的 appId、安装目录、开始菜单名、卸载项全部改成棱镜的，不和 dsh 官方桌面端冲突 |
| 用户数据目录 | Harness home 默认改为棱镜自己的目录（如 `%APPDATA%\Prism`），不用 `~/.dsh`，避免和用户装的 dsh 混用；业务数据默认在其下 `data/`，可在设置里改 |
| 自动更新 | 先关掉（没有自己的更新服务器之前不检查更新，避免连到 dsh 的更新源） |
| 许可证 | 保留 dsh 的 MIT 许可证和 `THIRD_PARTY_NOTICES.md`；棱镜自己的代码也用 MIT |

## 8. 已定的事（2026-11-04）

1. **跨模块计划：改用 dsh 的方式**。原来的 `plan_steps` 是"先列出多步计划、依赖关系、合计任务数，确认后按依赖自动执行"。
   - 做法：去掉专门的计划执行器，改用 dsh 的方式：模型自己按顺序调用工具，生图这类大任务走后台任务，执行前按张数审批。计划模式（先规划再执行）dsh 也有。
   - 代价：依赖顺序由模型保证，不再由程序强制；好处是少维护一整套执行器。
2. **API Key：存 Windows 凭据管理器**。需求 FR-41 要求存在 Windows 凭据管理器，dsh 默认存在 Harness home 下的私有文件里。
   - 做法：写一个凭据存储插件 `credentials-wincred`，替代 `credentials-local`，文本和生图的 Key 都存进凭据管理器；prism-service 用同样的名字读生图 Key。
3. **原 Tauri 版的数据：不迁移**。0.3.0 只是开发预览，没有真实用户。
   - 新版从空库开始；以后需要再写"从旧数据目录导入商品和素材"的功能。

## 9. 需要改动 dsh 原有文件的地方（预计）

| 文件 | 改动 |
|---|---|
| `apps/desktop` 的打包配置、图标、安装界面文案、欢迎页 | 品牌 |
| `apps/desktop/src/welcome-api.ts` | `needsWelcome` 恒为 false：不显示 dsh 的欢迎窗口（只能配 DeepSeek 官方模型或登录账号），模型在"设置 · 模型"里配 |
| `apps/desktop/src/locale.ts` | 菜单、对话框、托盘里的产品名：中文"棱镜"，其他语言 "Prism" |
| `packages/client/ui-conversation/src/client/locales.ts` | 首屏标语 `hero.headline` 改为"今天要做什么？"（8 种语言）。语言服务不允许别的插件覆盖同一命名空间，只能改这里 |
| `packages/client/locale/src/locales/*.ts` | `brand.localBuild` 改为"棱镜" / "Prism"，窗口标题随界面语言 |
| `apps/web/vite.config.ts` | 脚本加载前的默认窗口标题改为"棱镜 Prism" |
| 以上改动对应的测试和预期文件 | 产品名、标语、快照随之更新；dsh 欢迎引导的测试标记跳过并注明原因 |
| `apps/desktop/src/project-manager.ts` | 桌面端组合列表改为 dsh Web 组合 + `@prism/dsh-bundle-desktop`（已改，4 处引用加 1 个常量） |
| `apps/cli/package.json` | 依赖里加 `@prism/dsh-bundle-desktop`，让开发和打包都能解析到组合包（已改，1 行） |
| `tsconfig.base.json`、`tsconfig.host.json`、`tsconfig.client.json` | 登记棱镜的包（路径别名、项目引用）；以后每个棱镜包各加一行（已改） |
| `pnpm-lock.yaml` | 随新增的包变化；注意去掉镜像源写入的下载地址（见 `environment.md`） |
| `apps/desktop` 的 Harness home 默认值 | 指向棱镜目录 |
| `apps/desktop` 的打包文件策略 | 让 `@prism/service-*` 平台包解压到 asar 外 |
| 根 `package.json` / CI | 加 Rust 构建步骤（`cargo build -p prism-service --release`） |

以上之外的改动都放在新增的包里。合并上游时，冲突主要会集中在这几处。

## 10. 分步实施

每步都能单独提交和验收：

| 步 | 内容 | 验收 | 预计 |
|---|---|---|---|
| D1 | 组合与品牌：`prism-desktop` 组合包、关遥测和上报、棱镜预设、界面品牌（`packages/prism/ui-brand`）、不显示 dsh 欢迎窗口、产品名和标语。安装包的产品名和应用 ID、Harness home 默认目录挪到 D5（它们只影响打包后的版本，且牵动 dsh 官方发布流程的大量测试）；未签名构建本身不带更新源，自动更新已是关闭的 | 桌面端以"棱镜"启动，没有终端和 shell，抓包确认不向外上报；能用中转站的 Key 和文本模型对话 | 2 天 |
| D2 | prism-service 骨架：搬 Rust crate、JSON-RPC 服务、平台包、Host 进程管理插件 | 原有 Rust 测试在新位置全部通过；Host 能拉起服务并 `ping` 通；服务崩溃会重启 | 2 天 |
| D3 | 商品与素材：商品库、导入商品、素材库、店铺、规则包页面；商品查询工具；`@` 引用 | 1 万商品滚动流畅；对话里能查商品、引用商品 | 3 天 |
| D4 | 作图：生图配置卡片、Windows 凭据管理器存储插件 `credentials-wincred`、生图和图裂变工具（后台任务 + 审批）、结果区和结果卡片 | 模拟服务上批量生图、取消、超过阈值先确认；结果区显示图片 | 3 天 |
| D5 | Windows 安装包：产品名 Prism、快捷方式和卸载显示"棱镜 Prism"、安装包文件名、应用 ID（打包环境变量 `DSH_DESKTOP_APP_ID`）、Harness home 默认改到棱镜目录、关掉或替换 dsh 官方发布流程；未签名安装包、安装 / 卸载 / 覆盖安装验收 | 干净的 Windows 上安装后能用 | 1.5 天 |

合计约 11.5 个工作日，完成后功能大致回到原 P3 的水平，再接着做 P4（作图模块）。原计划的日期在确认后重排。

## 11. 风险

| 风险 | 应对 |
|---|---|
| dsh 是开发者预览版，会有破坏性更新 | 固定在已验证的提交上；每次合并上游前先看 changelog，合并后跑完整验收 |
| 合并上游时冲突 | 遵守原则 1，改动集中在第 9 节列的几处 |
| 安装包体积变大（Electron + 内置 Python / Node） | 内置 Python 是否需要在 D1 评估（棱镜目前用不到 Office 技能）；能去掉就去掉 |
| 界面要按 dsh 的插件方式重写（React 18、dsh 的 slot 体系） | 交互和视觉沿用原 `ui-design.md`，组件改用 dsh 的 `ui-primitives` |
| 插件依赖关系复杂，去掉某个插件导致启动失败 | D1 逐个去掉、逐个启动验证 |
