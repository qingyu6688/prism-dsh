---
kind: package-reference
description: "提供商品、素材、规则、生图配置和结果的 Remote，以及数据目录下的文件读取和上传路由。"
---

# 棱镜业务接口

## Summary

提供商品、素材、规则、生图配置和结果的 Remote，以及数据目录下的文件读取和上传路由。

## Table of Contents

- [模型使用方式](#model-experience)
- [限制和后续工作](#known-limitations-and-deferred-work)
- [维护说明](#dev-note)

接口、运行流程和验证结果见 [服务接口](../../../docs/prism/service-api.md)、[设计](../../../docs/prism/design.md) 和 [测试记录](../../../docs/prism/test.md)。

## Model Experience

### Prism 业务请求

#### What the model sees

Remote 面向界面；模型使用 tool-catalog。浏览器不能提交生图批次或读取原始凭据。`imageBatchStatus` 返回持久化的进度和素材，读取界面结果不会额外调用模型。

#### Token effect

工具调用和结果按实际内容占用上下文；界面缩略图和系统凭据不会自动加入模型输入。

#### KV Cache effect

本包没有缓存配置；动态工具结果会改变后续模型请求的内容。

## Known Limitations and Deferred Work

- 结果页列出最近 100 个批次中的生图批次；批次搜索按序号精确匹配。

### Dev Note

检查命令和验收限制见上面的测试记录。
