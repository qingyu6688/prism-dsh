---
kind: package-reference
description: "注册商品库、素材库、生图结果页、店铺、规则和生图设置、商品引用及全局搜索。生图结果也可作为会话右侧栏和对话卡片查看。"
---

# 棱镜商品与生图界面

## Summary

注册商品库、素材库、生图结果页、店铺、规则和生图设置、商品引用及全局搜索。生图结果也可作为会话右侧栏和对话卡片查看。

## Table of Contents

- [模型使用方式](#model-experience)
- [限制和后续工作](#known-limitations-and-deferred-work)
- [维护说明](#dev-note)

接口、运行流程和验证结果见 [服务接口](../../../docs/prism/service-api.md)、[设计](../../../docs/prism/design.md) 和 [测试记录](../../../docs/prism/test.md)。

## Model Experience

### Prism 业务请求

#### What the model sees

界面不直接生成图片；用户在对话中描述需求，由工具执行审批与后台任务流程。`prism_generate_images` 的已记录输出决定结果卡片显示的批次，重新打开会话不会重复提交。

#### Token effect

工具调用和结果按实际内容占用上下文；界面缩略图和系统凭据不会自动加入模型输入。

#### KV Cache effect

本包没有缓存配置；动态工具结果会改变后续模型请求的内容。

## Known Limitations and Deferred Work

- 结果每 2 秒查询，卸载时停止；系统重启后通过持久化批次恢复显示。付费网关兼容性需单独验收。

### Dev Note

检查命令和验收限制见上面的测试记录。
