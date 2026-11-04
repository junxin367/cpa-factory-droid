# 原生协议与调用限制

本版采用 strict native-only：客户端协议、模型配置中的协议和执行器输入/输出协议必须一致，插件不做跨协议转换。客户端使用 CPA 服务地址和 CPA 客户端 Key，Factory API Key 或 OAuth 令牌由插件后端使用。

本机 Mock 已覆盖标准三族原生请求、响应、工具续接及跨协议拒绝，实际范围见 [验证记录](validation.md)。模型由原生凭据自动同步，实际账号权限仍由 Factory 返回，见 [模型目录](models.md)。真实账号的 OAuth 和推理验证不能由模拟测试替代。

## 根据配置选择入口

先查看模型配置的 `protocol`，不要根据模型 ID 中是否含 `claude`、`gpt` 等名称猜协议：

| 模型配置 `protocol` | CPA 原生格式 | 客户端入口 | Factory 上游 |
| --- | --- | --- | --- |
| `anthropic` | `claude` | `POST /v1/messages` | `/api/llm/a/v1/messages` |
| `responses` | `openai-response` | `POST /v1/responses` | `/api/llm/o/v1/responses` |
| `chat_completions` | `openai` | `POST /v1/chat/completions` | `/api/llm/o/v1/chat/completions` |

客户端使用表中的普通 HTTP JSON / SSE 调用方式。共享模型 ID 沿用其配置的同一协议，不会改变允许的入口。宿主内部调用也必须提供完整、同协议的路径上下文并通过相同门禁；缺少路径、WebSocket / Upgrade 或其他操作后缀不作为普通原生调用放行。

协议升级以非空 `Upgrade` 请求头为判断依据。反向代理可能给普通请求附带 `Connection: upgrade`；当 `Upgrade` 缺失或为空时，三种原生入口的 JSON / SSE 请求均正常校验并转发，不把该连接标记误判为 WebSocket。显式的 `Upgrade: websocket`、`Upgrade: h2c` 等请求仍在访问 Factory 前返回 HTTP 400 `unsupported_operation`。这些反代升级标记不透传至 Factory。

对于宿主能向插件提供入口格式证明的标准请求，结果如下；非流式和流式应用相同规则：

| 客户端入口 | `anthropic` 模型 | `responses` 模型 | `chat_completions` 模型 |
| --- | --- | --- | --- |
| `/v1/messages` | 原生转发 | 400，无 Factory 请求 | 400，无 Factory 请求 |
| `/v1/responses` | 400，无 Factory 请求 | 原生转发 | 400，无 Factory 请求 |
| `/v1/chat/completions` | 400，无 Factory 请求 | 400，无 Factory 请求 | 原生转发 |

六种错配返回 HTTP 400，错误 `code` 为 `protocol_mismatch`。更换请求 URL 时必须同时使用对应原生 body；不能把 Chat 的 `messages/tool_calls` 当作 Responses `input`，也不能让插件把一种协议改造成另一种。

## 原生内容与响应

匹配时，插件在同一协议内转发 JSON、原始工具 ID、消息与内容块顺序，保留原生支持字段和不透明内容：

- Anthropic：`thinking.signature`、`redacted_thinking.data`、`tool_use` / `tool_result`、`cache_control`、原生图片和文档块；SSE 保留 `signature_delta` 和块索引。
- Responses：reasoning 的 `encrypted_content`、item ID / call_id、summary / content、原生工具、工具输出、`previous_response_id`、`conversation` 和 `include` 等字段。
- Chat Completions：原生消息、工具、reasoning、采样参数与 usage 结构。

不使用另一协议的参数表过滤或重命名这些字段。字段是否获 Factory 当前模型支持，由实际上游响应决定；保留字段不等于声明该账号或模型一定接受它。

必要的上游模型 ID、Droid identity / system / instructions 及认证/路由请求头按 Factory 网关契约处理，这些调整不改变消息协议。已有 `stream` 值必须与宿主执行方式一致；宿主按流式执行且该字段缺失或为 null 时，会补为 true。客户端 Authorization 和内部校验凭证不发给 Factory。

外层 `metadata`、客户端/代理标记、跟踪字段和下游身份字段会被清理；会话、缓存和安全标识经过独立映射。业务消息、工具参数和原生续接标识保持完整。因此同协议转发不代表不加区分地透传所有外层元数据。规则见 [出站协议与元数据](outbound-protocol.md)。

非流式成功响应通过原生结构检查后，以原始 body 返回给 CPA；SSE 保留上游原生帧，不重建成其他协议事件。错误状态、格式错误与断流使用对应错误返回。

客户端按原协议保存和回放完整工具历史，包括签名、脱敏块、原始工具 ID 与工具结果关系。插件按同一原生协议转发这些内容。

## 共享模型

| 模型 ID | 路由范围 | 用途 |
| --- | --- | --- |
| `factory/<模型后缀>` | 所有已发布该模型的有效 Factory 凭据 | 由 CPA 选择账号 |

不再发布包含账号 UUID 的第二套模型名。以 `GET /v1/models` 或原生凭据详情中的实际发布列表为准；目录出现模型不代表该账号一定有调用权限。

## CPA v8.0.52 的已知边界

插件的原始入口检查发生在选定凭据之后、执行器适配层转换之前；它只能检查宿主此时保留的信息。

CPA 的 [Chat handler](https://github.com/router-for-me/CLIProxyAPI/blob/v8.0.52/sdk/api/handlers/openai/openai_handlers.go#L135) 会在所有插件 hook 之前，把误投到 `/v1/chat/completions`、缺少 `messages` 且含 `input` 或 `instructions` 的 Responses 形状 body 改成 Chat，且不保留这次改写前的 raw body。插件看到的入口与 body 都可能已经是 Chat，因此无法将它与真正的 Chat 请求区分。此能力边界需要改宿主才能消除，本项目不修改 CPA。

宿主还可能调整 model、thinking、payload rules，整理同协议 SSE framing，或补齐 Responses usage details。因此承诺是**插件不做跨协议转换，并保留匹配协议的原生内容**，不能表述为从客户端到 Factory 的端到端字节透明。

### Chat 自定义 event-first SSE

标准 Chat `data: JSON` 帧可以正常处理，但合法的自定义命名事件存在另一项宿主限制。例如 Factory 发出：

```text
event: example.extension
data: {"value":"x"}
```

插件将原帧交给宿主后，CPA 的 OpenAI→OpenAI normalizer 与 [Chat HTTP writer](https://github.com/router-for-me/CLIProxyAPI/blob/v8.0.52/sdk/api/handlers/openai/openai_handlers.go#L578) 仍会处理它。以 `event:` 开头的帧会被再包成 `data: ...`，客户端可能收到默认 message 事件，其 data 也不再是原来的 JSON。

这改变事件语义，不只是空行或分块差异。v8.0.52 原生插件 ABI 没有跳过该 writer 的开关，本项目不改宿主，也不通过删掉测试事件冒充通过。因此**不能承诺所有 Chat SSE 命名事件端到端保真**。Claude / Responses 使用各自宿主路径，不能据此推断 Chat；它们也可能受宿主已有私有事件过滤规则影响。

## SSE、错误与用量

插件向宿主保留合法原生 SSE 帧，同时观察该协议的严格完成条件；观察不合成另一协议的事件。终态非法的帧在发送前拒绝，合法原生 error 帧按原帧发送后以错误结束。单帧最大 16 MiB。最终客户端行为仍受上节宿主边界限制：

| 原生协议 | 完成条件 |
| --- | --- |
| Chat Completions | 所有已出现的 choice 都有非空 `finish_reason`，随后收到 `[DONE]` |
| Anthropic Messages | 有 `message_delta.stop_reason`，随后收到 `message_stop` |
| Responses | 收到 `response.completed` 或 `response.incomplete`，保留其原生事件语义 |

`[DONE]` 不能替代 Anthropic / Responses 的原生终态。客户端断连、超时、上游断流或缺少终态不能按完整成功请求记账；流已开始后不透明重放请求。

原生 usage 交给 CPA 统计及其管理端计价，插件不再重复记账。调用仍要求凭据启用、模型存在于其发布目录且协议匹配；未发布的路由返回 `model_not_allowed`。目录同步不执行付费推理。

401 表示凭据验证失败；403 可能涉及账号、模型或组织权限；413 表示请求过大；429 保留可取得的 `Retry-After`。官方额度或 Analytics 不可读取与模型请求权限分开判断。

HTTP 非成功响应保留原状态。插件最多读取 32 KiB 错误正文：完整 JSON 对象原文交给宿主，非 JSON 正文作为错误消息显示，不脱敏、不加译文前缀、不替换上游错误原因。达到大小上限时保留已读内容并标记截断；没有正文时才使用通用提示。错误响应流最多等待 2 秒并始终关闭，不影响其他请求。已有 HTTP 200 SSE 中的原生 error 事件仍遵循上文的原生事件转发规则。

CPA v8.0.52 的 Claude 错误处理会将执行器错误重组为 `type: error` 外壳，只保留源错误的 `type` 和 `message`；`code`、`param`、`request_id` 等附加 JSON 字段可能被宿主移除。Chat 与非流式 Responses 路径保留完整 JSON 错误对象。Responses 流式错误会在宿主内再次脱敏、限长，并移除部分外层字段；这是 `sanitizeResponsesStreamErrorMessage` 的宿主行为，插件原样返回错误也不能绕过。真实 CPA 验证分别断言各入口的实际行为，不声称所有入口端到端不变；本插件不修改 CPA 内核。

## Messages token 计数

`POST /v1/messages/count_tokens` 支持非流式本地估算，遵循与生成请求相同的调用方鉴权、凭据启用及模型访问校验；一次性路径凭证不能在计数与生成入口之间复用。

参考 CPA 对第三方 Claude 网关的处理，使用内嵌 O200k tokenizer，收集 system、消息文本、思考文本、工具定义、工具调用及嵌套结果。返回标准 `input_tokens`，并附加 `estimated: true`、`estimator: "o200k_base"`、`media_tokens_included: false`。不会调用 Factory 推理或计费接口，也不向 CPA 发布生成用量。单次计数请求上限 16 MiB。

这是文本估算，不是 Factory 官方精确计数：图片、音频、视频、PDF 二进制、加密或脱敏思考内容不计入；JSON 对象采用本插件的序列化顺序。不能用于结算或保证请求一定未超过上游上下文限制。
