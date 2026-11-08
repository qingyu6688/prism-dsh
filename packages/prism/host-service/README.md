---
kind: package-reference
description: "提供 ctx.prismService，管理 Rust 服务的启动、重启、JSON-RPC 和退出。默认退出等待 45 秒，可配置 shutdownGraceMs。"
---

# 棱镜业务进程

## Summary

提供 ctx.prismService，管理 Rust 服务的启动、重启、JSON-RPC 和退出。默认退出等待 45 秒，可配置 shutdownGraceMs。

## Table of Contents

- [模型使用方式](#model-experience)
- [限制和后续工作](#known-limitations-and-deferred-work)
- [维护说明](#dev-note)

接口、运行流程和验证结果见 [服务接口](../../../docs/prism/service-api.md)、[设计](../../../docs/prism/design.md) 和 [测试记录](../../../docs/prism/test.md)。

## Model Experience

### Prism 业务请求

#### What the model sees

模型通过业务工具间接使用服务，不能直接调用任意 RPC。模型通过 `prism_generate_images` 等消费工具间接使用服务，不接触原始 RPC。

#### Token effect

工具调用和结果按实际内容占用上下文；界面缩略图和系统凭据不会自动加入模型输入。

#### KV Cache effect

本包没有缓存配置；动态工具结果会改变后续模型请求的内容。

## Known Limitations and Deferred Work

- 开发版需要先构建 Rust 可执行文件；平台包和安装包归 D5。

### Dev Note

检查命令和验收限制见上面的测试记录。
