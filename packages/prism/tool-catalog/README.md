---
kind: package-reference
description: "注册商品查询、prism_generate_images 和 prism_fission_images。生图工具在具备 jobs 和 approval 的预设中启用。"
---

# 棱镜业务工具

## Summary

注册商品查询、prism_generate_images 和 prism_fission_images。生图工具在具备 jobs 和 approval 的预设中启用。

## Table of Contents

- [模型使用方式](#model-experience)
- [限制和后续工作](#known-limitations-and-deferred-work)
- [维护说明](#dev-note)

接口、运行流程和验证结果见 [服务接口](../../../docs/prism/service-api.md)、[设计](../../../docs/prism/design.md) 和 [测试记录](../../../docs/prism/test.md)。

部署可在插件配置中设置 `imagePollIntervalMs`（100～60000 毫秒，默认 1000），控制后台任务查询批次状态的频率。

## Model Experience

### Prism 业务请求

#### What the model sees

生图默认 1 张，图裂变默认 4 张；最多 200 张，超过 40 张先审批。返回 jobId 和持久化 batchId，完成结果进入会话。`prism_generate_images` 和 `prism_fission_images` 返回 `jobId` 与 `batchId`，供后续查询和取消。

#### Token effect

工具调用和结果按实际内容占用上下文；界面缩略图和系统凭据不会自动加入模型输入。

#### KV Cache effect

本包没有缓存配置；动态工具结果会改变后续模型请求的内容。

## Known Limitations and Deferred Work

- 参考图必须来自素材库，最多 6 张。接口故障时应先检查已有批次，避免重复付费提交。

### Dev Note

检查命令和验收限制见上面的测试记录。
