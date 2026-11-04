# 出站协议与元数据

核验日期：2026-11-04（Asia/Shanghai）。基线为官方 Windows x64 Droid CLI `1.2.0`，文件 SHA256 为 `b075e952982d0b3c731e875dfd4e95c835c4d1a8d9e46c791636ebd8116fd754`。

本次实际运行该官方可执行文件，在独立 HOME、空工作目录和模拟 API Key 下，将 Factory API 指向本机模拟服务。分别使用 `claude-opus-5-5`、`gpt-5.4`、`glm-5.3`，三个进程均正常退出并收到模拟回复。依据为真实捕获的 HTTP 报文，不以二进制字符串出现某字段作为发送证据。脱敏摘要见 [官方采样](factory-cli-wire-baseline.json)。

## 请求包装

| 项目 | 当前行为 |
| --- | --- |
| LLM 客户端身份 | 默认 `User-Agent: factory-cli/1.2.0`、`X-Factory-Client: cli`、`X-Client-Version: 1.2.0` |
| 认证 | 插件读取当前 Factory API Key / OAuth access token，重建 Bearer；不转发下游 CPA Key |
| Anthropic | 固定 `anthropic-version: 2023-06-01` 和 `anthropic-beta: fine-grained-tool-streaming-2025-05-14`；模型目录要求 fast 模式时补充对应 beta；不转发调用者的版本、beta 或占位 `X-Api-Key` |
| 模型路由 | 原生路径、上游模型名、`x-api-provider`、`x-provider-routing-source` 和 OpenAI 组织头与采样对齐 |
| 会话 | `x-session-id` 使用不透明 UUID；有明确对话标识时，在相同凭据和调用方范围内稳定，重启后也保持一致 |
| 单次消息及跟踪 | 每次出站独立生成消息 UUID 和 W3C `traceparent`，不转发本地 request ID、traceparent、tracestate 或 baggage |
| Responses 缓存与安全标识 | 显式 `prompt_cache_key` 和 `safety_identifier` 分别映射，保留缓存分组和用户范围；未显式指定且有明确对话时沿用会话 UUID |
| API 查询 | whoami 使用采样中的 `Bun/1.4.4` fetch 身份；billing 查询补充 CLI 版本及独立追踪头 |
| OAuth | 使用明确的 fetch 客户端头，不依赖 Go 默认 User-Agent；账号授权流程保持原样 |

发往上游的头部从空集合构造，只使用 Factory CLI 默认值、当前 Factory 凭据、受信任模型目录及插件生成的会话/跟踪标识。下游 `User-Agent`、`Via`、`Forwarded`、`X-Forwarded-*`、`Origin`、`Referer`、Cookie、代理鉴权及插件内部门禁头均不转发。调用者的 `anthropic-version`、`anthropic-beta` 和 `OpenAI-Beta` 也不透传，不再因 Claude Code 的 `claude-code-20250219` 等标志不符合日期格式而拒绝请求。

固定协议头按上述 Factory CLI 采样基线生成；`server-side-fallback` / `fallback-credit` 与本插件未启用的 fallback 请求流程配套，因此不单独照搬。客户端通过自定义 beta 头开启的其他实验功能不会自动启用；只有插件实现并与模型目录匹配的能力才会生成对应上游标志。

会话优先采用原生 `conversation`，其次读取明确的 CLI/会话/线程字段和执行会话；Claude 子代理及 Codex 子线程分别隔离。CPA 的 canonical/derived ID 可能来自共享缓存键、用户或路由前缀，不能直接认作对话。缺少明确线索时，单次请求使用独立会话 UUID；派生前缀仍可用于默认缓存分组。

从 v0.1.1 起，Codex 同时提供线程 ID 和子代理名称时优先使用明确线程 ID，避免同一根会话中的不同线程因为复用了 `agent_name` 而合并。未提供线程 ID 时仍可按根会话和子代理名称保持稳定。

插件状态只持久化随机 `wire_namespace`，用分域哈希生成会话、缓存组和安全标识，不存储下游原文或无限增长的映射表。同一命名空间、凭据、调用方和原始标识产生相同 UUID，避免重启或映射表淘汰造成缓存键变化。备份和迁移插件数据目录时应保留该字段；新安装或恢复不含该字段的旧备份会生成新命名空间。

## 请求体边界

清理顶层 `metadata`、`user`、客户端/SDK/运行时标记、鉴权/转发信息、请求/会话/跟踪编号，以及明确的 CPA、CLIProxyAPI、LiteLLM、OpenRouter 等代理字段。客户端依赖上游保存这些自定义元数据的行为会改变。

消息、system/instructions、工具定义和参数、工具结果、签名、加密推理、图片、`previous_response_id`、原生 `conversation` 等仍按原协议保留。业务文本中本身出现“CPA”或“proxy”不会被改写；这些内容不是插件自动附加的反代信息。

本次不自动启用官方 CLI 的 server-side fallback / fallback-credit 功能，也不复制其整套工具定义和提示词。对应 beta、工具、推理参数与实际功能绑定，不能为了请求外观相似而改变客户端指定的调用行为。HTTP 压缩由宿主实际支持的解码能力决定，不宣称支持尚未实现的 br/zstd 解压。

## 验证及限制

- 官方 CLI 三个原生入口均完成本机采样。
- 6 项既有单元测试、8 项本次专项测试通过；专项覆盖会话范围、无会话情况、TTL/容量、外层清理、业务内容保留和协议头注入。
- 56 项真实 CPA 宿主隔离检查通过，包括三协议 JSON/SSE、不同调用方/凭据隔离、仅 X-Api-Key 鉴权、合法 beta、恶意协议头拒绝、OAuth、官方额度接口和 CPA 原生统计。明细见 [验证记录](outbound-protocol-validation.json)。
- 已部署到本地 CPA `127.0.0.1:60892` 并重启。安装库与通过隔离检查的库 SHA256 相同：`300217360363677bc9a17691a495d1a1b82aba97a1ead1693d0c9a46f5ca7b52`。现有 53 个模型保持一致，账号 UUID 模型为 0；已有真实 API Key 凭证的额度接口返回 HTTP 200。
- 本次模拟测试未使用真实账号执行推理，仍不代表真实账号拥有所有模型权限。

上述 56 项及 `300217...` 库哈希属于首次出站包装对齐批次。后续会话/缓存拆分修复的复现、当前部署信息和限制，以 [会话与缓存审查](session-cache-review.md) 和 [对应验证数据](session-cache-validation.json) 为准。

已清理的是应用层可控的包装字段。TLS/HTTP 实现、出口网络、调用节奏及用户实际内容仍可能与官方 CLI 不同；不能据此承诺上游无法识别客户端实现。
