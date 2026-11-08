---
kind: package-reference
description: "替换棱镜组合中的 credentials-local。通过 prism-service 读写 Windows 凭据管理器；接口错误会阻止对应操作，迁移失败保留旧文件。"
---

# 系统凭据存储

## Summary

替换棱镜组合中的 credentials-local。通过 prism-service 读写 Windows 凭据管理器；接口错误会阻止对应操作，迁移失败保留旧文件。

## Table of Contents

- [模型使用方式](#model-experience)
- [限制和后续工作](#known-limitations-and-deferred-work)
- [维护说明](#dev-note)

接口、运行流程和验证结果见 [服务接口](../../../docs/prism/service-api.md)、[设计](../../../docs/prism/design.md) 和 [测试记录](../../../docs/prism/test.md)。

## Model Experience

### Prism 业务请求

#### What the model sees

模型不能读取或导出密钥；提供器只供 Host 模型路由及授权插件调用。`credentials.get` 只供本机 Host 读取，模型工具表不包含这个接口。

#### Token effect

工具调用和结果按实际内容占用上下文；界面缩略图和系统凭据不会自动加入模型输入。

#### KV Cache effect

本包没有缓存配置；动态工具结果会改变后续模型请求的内容。

## Known Limitations and Deferred Work

- 凭据命名空间随业务数据目录变化，复制数据库不能复制系统凭据。Windows 为已验收平台。

### Dev Note

检查命令和验收限制见上面的测试记录。
