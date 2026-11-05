# 出站协议与元数据

核验日期：2026-11-04（Asia/Shanghai）。基线为官方 Windows x64 Droid CLI `1.2.0`，文件 SHA256 为 `b075e952982d0b3c731e875dfd4e95c835c4d1a8d9e46c791636ebd8116fd754`。

本次实际运行该官方可执行文件，在独立 HOME、空工作目录和模拟 API Key 下，将 Factory API 指向本机模拟服务。分别使用 `claude-opus-5-5`、`gpt-5.4`、`glm-5.3`，三个进程均正常退出并收到模拟回复。依据为真实捕获的 HTTP 报文，不以二进制字符串出现某字段作为发送证据。脱敏摘要见 [官方采样](factory-cli-wire-baseline.json)。

## 请求包装

| 项目 | 当前行为 |
| --- | --- |
| LLM 客户端身份 | 默认 `User-Agent: factory-cli/1.2.0`、`X-Factory-Client: cli`、`X-Client-Version: 1.2.0` |
| 认证 | 插件读取当前 Factory API Key / OAuth access token，重建 Bearer；不转发下游 CPA Key |
| Anthropic | 固定 `anthropic-version: 2023-06-01`；以 `fine-grained-tool-streaming-2025-05-14` 为基础，按请求中的受支持功能追加 beta，并合并调用者 Header / SDK `betas` 中的能力标志；不转发调用者版本或占位 `X-Api-Key` |
| 模型路由 | 原生路径、上游模型名、`x-api-provider`、`x-provider-routing-source` 和 OpenAI 组织头与采样对齐 |
| 会话 | `x-session-id` 使用不透明 UUID；有明确对话标识时，在相同凭据和调用方范围内稳定，重启后也保持一致 |
| 单次消息及跟踪 | 每次出站独立生成消息 UUID 和 W3C `traceparent`，不转发本地 request ID、traceparent、tracestate 或 baggage |
| Responses 缓存与安全标识 | 显式 `prompt_cache_key` 和 `safety_identifier` 分别映射，保留缓存分组和用户范围；未显式指定且有明确对话时沿用会话 UUID |
| API 查询 | whoami 使用采样中的 `Bun/1.4.4` fetch 身份；billing 查询补充 CLI 版本及独立追踪头 |
| OAuth | 使用明确的 fetch 客户端头，不依赖 Go 默认 User-Agent；账号授权流程保持原样 |

发往上游的头部从空集合构造，使用 Factory CLI 默认值、当前 Factory 凭据、受信任模型目录及插件生成的会话/跟踪标识。下游 `User-Agent`、`Via`、`Forwarded`、`X-Forwarded-*`、`Origin`、`Referer`、Cookie、代理鉴权及插件内部门禁头均不转发。调用者的 `anthropic-version` 不覆盖 Factory 协议版本；匹配协议的 `anthropic-beta` / `OpenAI-Beta` 可合并到上游，未知能力标志交由上游判断。标志必须为合法可见 ASCII，重复项只保留一次。

普通请求的默认协议头按上述 Factory CLI 采样基线生成。功能标志根据请求体独立生成，不能把一次普通请求的采样头视为所有请求的完整头集合。`thinking.display=updates`、`thinking.block_binding`、中途工具变化及逐消息推理档位参考官方 CLI 1.2.0 的参数组装分支；上下文管理、工具搜索、中途系统消息等原生功能参考 CPA Claude 执行器的明确协议规则。调用者能力标志与默认标志合并，正文 `betas` 作为 SDK 传输选项消费到 Header 后移除。工具参数与 JSON Schema 内的同名业务字段不会触发功能标志。

不启用尚未实现的 `server-side-fallback` / `fallback-credit`。对应 beta、非空 `fallbacks` 或 `fallback_credit_token` 会在访问上游前返回 `unsupported_native_feature`。其他仅由 beta 开启的实验功能可以转发其标志，实际可用性由 Factory 上游判断，不承诺每项 beta 均受其支持。

会话优先采用原生 `conversation`，其次读取明确的 CLI/会话/线程字段和执行会话；Claude 子代理及 Codex 子线程分别隔离。CPA 的 canonical/derived ID 可能来自共享缓存键、用户或路由前缀，不能直接认作对话。缺少明确线索时，单次请求使用独立会话 UUID；派生前缀仍可用于默认缓存分组。

从 v0.1.1 起，Codex 同时提供线程 ID 和子代理名称时优先使用明确线程 ID，避免同一根会话中的不同线程因为复用了 `agent_name` 而合并。未提供线程 ID 时仍可按根会话和子代理名称保持稳定。

插件状态只持久化随机 `wire_namespace`，用分域哈希生成会话、缓存组和安全标识，不存储下游原文或无限增长的映射表。同一命名空间、凭据、调用方和原始标识产生相同 UUID，避免重启或映射表淘汰造成缓存键变化。备份和迁移插件数据目录时应保留该字段；新安装或恢复不含该字段的旧备份会生成新命名空间。

## 请求体边界

清理顶层 `metadata`、`user`、客户端/SDK/运行时标记、鉴权/转发信息、请求/会话/跟踪编号，以及明确的 CPA、CLIProxyAPI、LiteLLM、OpenRouter 等代理字段。客户端依赖上游保存这些自定义元数据的行为会改变。

消息、system/instructions、工具定义和参数、工具结果、签名、加密推理、图片、`previous_response_id`、原生 `conversation` 等仍按原协议保留。业务文本中本身出现“CPA”或“proxy”不会被改写；这些内容不是插件自动附加的反代信息。默认封装在鉴权后拦截阶段完成，CPA 的 payload override/filter 随后生效；最终执行阶段不会重新添加过滤掉的缓存或安全标识，也不会再次清理管理员显式配置的业务字段。

本次不自动启用官方 CLI 的 server-side fallback / fallback-credit 功能，也不复制其整套工具定义和提示词。对应 beta、工具、推理参数与实际功能绑定，不能为了请求外观相似而改变客户端指定的调用行为。HTTP 压缩由宿主实际支持的解码能力决定，不宣称支持尚未实现的 br/zstd 解压。

## 验证及限制

2026-11-04 的后续兼容修复对照了 SHA256 一致的官方 CLI 1.2.0，并离线执行其纯参数组装函数：普通 `xhigh` 保持基础 beta，`display=updates` 增加专用 beta。此证据证明组装分支，不替代真实账号上游验收。宿主验证脚本已增加完整工具历史与功能标志的严格校验，覆盖错误正文分段、超大正文、不结束的错误流、本地 token 计数及调用者鉴权。

- 官方 CLI 三个原生入口均完成本机采样。
- 6 项既有单元测试、8 项本次专项测试通过；专项覆盖会话范围、无会话情况、TTL/容量、外层清理、业务内容保留和协议头注入。
- 56 项真实 CPA 宿主隔离检查通过，包括三协议 JSON/SSE、不同调用方/凭据隔离、仅 X-Api-Key 鉴权、合法 beta、恶意协议头拒绝、OAuth、官方额度接口和 CPA 原生统计。明细见 [验证记录](outbound-protocol-validation.json)。
- 已部署到本地 CPA `127.0.0.1:60892` 并重启。安装库与通过隔离检查的库 SHA256 相同：`300217360363677bc9a17691a495d1a1b82aba97a1ead1693d0c9a46f5ca7b52`。现有 53 个模型保持一致，账号 UUID 模型为 0；已有真实 API Key 凭证的额度接口返回 HTTP 200。
- 本次模拟测试未使用真实账号执行推理，仍不代表真实账号拥有所有模型权限。

上述 56 项及 `300217...` 库哈希属于首次出站包装对齐批次。后续会话/缓存拆分修复的复现、当前部署信息和限制，以 [会话与缓存审查](session-cache-review.md) 和 [对应验证数据](session-cache-validation.json) 为准。

已清理的是应用层可控的包装字段。TLS/HTTP 实现、出口网络、调用节奏及用户实际内容仍可能与官方 CLI 不同；不能据此承诺上游无法识别客户端实现。
