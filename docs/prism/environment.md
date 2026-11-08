# 棱镜（基于 dsh）开发环境

记录日期：2026-11-04
底座：deepseek-harness `fcc0ad4621`（dsh 0.2.2-rc.1），远程名 `upstream`

## 本机环境

| 项 | 版本 / 位置 |
|---|---|
| 操作系统 | Windows 11 Pro 10.0.26200，x64，16 核 |
| Node.js | 24.21.0（dsh 要求 ≥ 24.9） |
| pnpm | 11.28.5，安装在 `D:\AI-Cache\pnpm-cli`（没有启用 corepack，避免写 C 盘） |
| Git | 2.47.1 |
| Rust | 1.96.0（棱镜业务服务 `prism-service` 用），`CARGO_HOME=D:/AI-Cache/cargo` |
| Visual Studio | 2022（Windows 安装包需要的 C++ 组件待打包时确认） |

## 缓存都放 D 盘

每个终端先设置这些环境变量（Git Bash 写法）：

```bash
export PATH="/d/AI-Cache/pnpm-cli/node_modules/.bin:$PATH"
export npm_config_store_dir=D:/AI-Cache/pnpm-store
export npm_config_cache=D:/AI-Cache/npm
export ELECTRON_CACHE=D:/AI-Cache/electron
export electron_config_cache=D:/AI-Cache/electron
export TEMP=D:/Claude-Work/tmp TMP=D:/Claude-Work/tmp
```

## 常用命令（在仓库根目录 `D:\Claude-Work\prism-dsh` 执行）

```bash
pnpm install                 # 安装依赖，约 1 分钟
(cd native/prism-service && cargo build -p prism-service)   # 构建 Rust 业务服务，桌面端启动时由 Host 插件拉起
pnpm run build:artifacts     # 构建，约 3 分钟
pnpm run start:desktop       # 用已有产物启动桌面端
pnpm run dev:desktop         # 构建后启动桌面端
```

桌面端开发数据在 `apps/desktop/.desktop-build/development/`（Harness home、Electron 用户数据），不会写进用户的 `~/.dsh`；棱镜的数据库在其中的 `home/prism/`。调试端口：主进程 9229、界面 9222、Host 9230。

Host 插件按 `native/prism-service/target/` 下较新的 release 或 debug 构建启动服务，没构建过时界面照常打开，只是业务功能报"服务不可用"。要用别处的可执行文件，设环境变量 `PRISM_SERVICE_BIN`。

Rust 部分的检查（在 `native/prism-service` 执行）：

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
D:/AI-Cache/cargo/bin/cargo-deny check
```

## 锁文件和国内镜像

本机 npm 源是国内镜像（npmmirror）。`pnpm install` 改动锁文件时会把每个包的镜像下载地址写进 `pnpm-lock.yaml`，产生几千行和机器有关的差异。安装后执行下面这条命令去掉这些地址（校验靠 integrity 哈希，不受影响），再看 `git diff pnpm-lock.yaml` 只剩真正的变化：

```bash
sed -i -E 's#, tarball: https://registry\.npmmirror\.com/[^}]*\}#}#' pnpm-lock.yaml
```

## 第一次跑通的记录（2026-11-04）

| 步骤 | 结果 |
|---|---|
| `pnpm install` | 58 秒，无错误；Linux / macOS 专用的原生包按平台跳过（警告，正常） |
| `pnpm run build:artifacts` | 169 秒，8 项检查全部通过 |
| `pnpm run start:desktop` | 首次下载 Electron 本体、准备内置 Python / Node 运行时并通过 Office 读写检查；Host 监听 `127.0.0.1` 随机端口，中文界面正常显示 |

已知现象：启动时报 `No handler registered for 'dsh-desktop:mandatory-status'`（强制更新状态的处理器在开发模式下没有注册），界面不受影响，待确认是否是开发模式的正常现象。

还没验证：和模型对话（需要 API Key）、Windows 安装包打包。

## 2026-11-05 当前终端实测

以下结果来自本次 Codex 终端；不同终端的 PATH 可能选中不同版本，执行构建前应核对当前命令。

| 项 | 实测结果 |
|---|---|
| 操作系统 | Windows 11 专业版 10.0.26200，64 位 |
| Node.js | 24.9.0，命令位于 `E:\nvm\nodejs\node.exe` |
| npm | 11.6.0 |
| pnpm | 11.25.0，来自 Codex 运行时；根 `package.json` 声明 11.28.5 |
| Java | 17.0.8 LTS |
| Python / pip | Python 3.12.3 / pip 25.2 |
| Git | 2.47.1.windows.1 |
| MySQL 客户端 | 8.0.46；棱镜业务库使用 SQLite，此项仅记录本机工具版本 |
| Rust / Cargo | rustc 1.96.0 / cargo 1.96.0，命令位于 `F:\rust\cargo\bin` |
| 本地环境变量 | `CARGO_HOME` 已设置；`DSH_HOME`、`PRISM_SERVICE_BIN`、`DEEPSEEK_API_KEY`、`DEEPSEEK_BASE_URL` 未设置；根目录没有 `.env` |

本次只检查开发环境，没有启动应用或连接模型。
