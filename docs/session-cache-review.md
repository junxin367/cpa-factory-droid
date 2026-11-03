# CPA 会话与缓存审查

核验日期：2026-11-04。当前本地 CPA 为 `v8.0.52`，源码与运行版本均对应 `6e6afab99a1939525ecb6bb1f0873c8e7a9df877`。对照源码位于 `temp/research/cpa/host`；未以其他 checkout 的 HEAD 代替运行版本。

## RCA 结论

- 状态：部分确认。Factory 请求标识和缓存语义缺陷已复现并修复；未验证真实 Factory 服务端是否曾串用历史，也没有测量真实缓存命中率。
- 现象：旧“账号 ID 充当会话 ID”已在上一批次移除，但新的统一匿名映射仍把显式缓存键、安全标识和对话绑定成同一个 UUID。
- 直接原因：CPA 的 `canonical_session_id` 可以由 `prompt_cache_key` 或普通 `metadata.user_id` 派生；插件优先使用它，覆盖了请求里不同的原生 `conversation`。出站代码又把缓存键、安全标识统一改成会话 UUID。
- 根因：未区分“对话身份”“可跨对话共享的缓存分组”和“用户范围的安全标识”。随机内存映射在重启和淘汰后也会主动改变缓存键。
- 系统性根因：把官方 CLI 默认请求中几个 ID 相同的采样结果推广为所有客户端请求的约束。原验收覆盖显式会话头，但没有覆盖不同 conversation 共用缓存组、同一对话切换缓存组及重启稳定性。
- 关键因果链：共享缓存键 → CPA canonical 别名相同 → 插件优先选该别名 → 两个明确不同的 conversation 使用同一上游会话 ID。反向场景则是一个真实会话 → 统一 UUID 覆盖两个不同缓存键 → 调用方指定的缓存分组区别丢失。

## 证据

| 判断 | 类型 | 证据与位置 |
| --- | --- | --- |
| 上一批次运行的库已不再直接发送本地账号 ID | 事实 | 修复前 DLL SHA256 `300217360363677bc9a17691a495d1a1b82aba97a1ead1693d0c9a46f5ca7b52`，匹配此前部署证据 |
| 两个 conversation 共用缓存键会被合并为同一出站会话 ID | 事实 | 真实 CPA 宿主 + 本机 Mock 的修复前用例失败，修复后通过；见验证 JSON 同名检查 |
| 同一对话的两个显式缓存组被覆盖 | 事实 | 修复前输出缓存键相同；修复后不同，且对话 ID 保持相同 |
| 显式缓存组可用于多个独立对话 | 契约 | OpenAI 缓存文档允许按客户、用户或工作区分组；缓存复用还要求匹配的输入前缀 |
| context 由消息或原生续接字段提供 | 契约及本地事实 | OpenAI conversation-state 文档；本次三协议消息和 `conversation`、`previous_response_id` 保留检查通过 |
| 并发情况下存在标识合并，但没有观察到本地正文串用 | 事实 | 修复前 16 个并发 conversation 产生相同会话 ID，但各请求业务正文均完整；修复后会话 ID 也各自隔离 |
| 重启导致缓存标识变化 | 事实 | 修复前重启检查失败；同一插件状态目录下，修复后会话、缓存、安全标识都稳定 |

## CPA 其他凭证的处理

以下是当前版本源码审查，不等于对每个提供商的真实账号做过运行验收。CPA 没有统一规定“所有凭证都用账号 ID 作为对话 ID”。

| 提供商或路径 | 当前源码中的标识来源与用途 |
| --- | --- |
| Claude OAuth / Claude Code profile | `ClaudeAgentSessionUUIDForRequest` 按真实会话、执行上下文或派生对话生成 UUID；凭据设备身份与会话分开。连续性状态组合凭据身份和会话 ID。普通 API Key 默认保留调用方自身行为。 |
| Claude 兼容兜底 | `CachedSessionIDRequired(apiKey)` 确实存在，按 Key 缓存 UUID；属于旧兼容/工具函数路径，不能仅凭函数名推断所有 Claude 请求都走它。主 CLI profile 会传入已经解析的会话 UUID。 |
| Codex HTTP | 优先使用显式缓存键或会话派生值；部分 Chat 兜底按下游 CPA API Key 生成缓存键。这个缓存分组不等同于 WebSocket 连接状态。 |
| Codex WebSocket | 下游连接建立独立 `execution_session_id`，连接/响应状态按该 ID 管理，并检查当前 auth、URL 等目标是否匹配。不是按账号 ID 合并所有连接。 |
| OpenAI-compatible | 保留用户显式缓存键；否则按提供商、模型、协议和派生会话生成缓存键。需要启用对应缓存键能力。 |
| Devin | 读取 session / conversation / execution 等上下文并规范化 UUID；session 与 cascade 用于会话和缓存，完整历史仍随请求发送。 |
| Antigravity | 优先显式 session，再用派生上下文或首轮用户内容哈希。Interactions 续接缓存还组合调用方、凭据、模型和工具调用标识。无明确会话时的内容推导本身有歧义，不能当作严格的对话边界证明。 |
| Gemini / Vertex | 常规生成请求移除内部 session 字段，使用完整 contents；显式 cachedContent 是资源引用，不是把账号 ID 当作对话。 |
| Kimi | Chat 路径为无状态请求；Messages 路径复用 Claude 执行器及相应 profile。 |
| xAI / Meta | xAI 部分路径复用 execution / prompt_cache_key / derived ID；Meta 生成请求级 UUID。字段名字或相同值本身不足以证明服务端会自动拼接历史。相关私有上游语义未做真实账号验证。 |

源码入口：`internal/runtime/executor/helps/claude_credential_identity.go`、`helps/session_id_cache.go`、`claude_fingerprint_policy.go`、`codex_executor_request.go`、`codex_websockets_session.go`、`openai_compat_executor.go`、`devin_executor.go`、`antigravity_executor_request.go`、`helps/antigravity_interactions_state.go`、`gemini_executor.go`、`kimi_executor.go`、`xai_executor_request.go`、`meta_executor_execute.go`。

本次没有修改 CPA 核心或这些提供商的未证实路径。

## 已排除解释

- “只要两个请求的缓存键相同，模型就一定混入另一个对话”：公开的前缀缓存契约不支持这个推断；本次模拟请求也未出现正文串用。
- “旧账号级会话 ID 仍是当前实现”：部署前哈希与上一批次一致，旧字段已移除。此次修复的是后续匿名映射中的语义耦合。
- “问题只是格式不是 UUID”：修复前已经是合法 UUID，仍能复现不同 conversation 合并；根因是 ID 的来源和用途混淆。

## 异常状态溯源

- 适用约束：当调用方明确提供不同原生 conversation 时，不得仅因共享缓存分组而把它们当作同一对话；同一缓存分组经匿名化后应仍是同一分组，不同分组应保持不同。
- 依据及状态：原生会话/缓存字段的独立契约，以及本任务要求的多会话隔离；请求构造层已确认。
- 场景命中：同一凭据、同一调用方，`conversation.id` 不同而 `prompt_cache_key` 相同。
- 实际来源：请求缓存键 → CPA `session.ExtractSessionInfo` 的 pck 别名 → `canonical_session_id` → 旧 `wire::session_hint` → 旧 `Identity.session` → 会话头、缓存键和安全标识。
- 首次约束违反点：插件将可能代表缓存分组的 canonical 别名无条件提升为 Factory 对话身份；随后进一步覆盖独立缓存字段。
- 调查边界：Factory 私有网关如何使用这些字段、真实缓存收益和历史污染，不能由本地 Mock 推断。

## 修复与防复发

1. 分别生成会话、缓存分组、安全标识，保留用户显式分组的含义，输出仍为不透明 UUID。
2. 对话优先取原生 `conversation`，其次取明确 CLI/会话/线程字段或执行会话；普通 user_id、缓存键及路由/前缀派生值不自动成为对话。
3. 保留 Claude 子代理、Codex 子线程的独立身份。没有明确对话线索时使用独立请求会话；默认缓存仍可按派生前缀或调用方范围稳定分组。
4. 在插件状态中保存随机 `wire_namespace`，通过分域哈希稳定匿名化。不同凭据、不同调用方分别隔离，不保存原始会话/缓存键或无限增长的映射表。
5. 保留消息、工具签名、加密推理、cache_control、原生 continuation 参数。未增加自动历史拼接或隐式上下文恢复。
6. 24 项真实 CPA 隔离对照中，修复前 9 项失败，修复后全部通过；补充旧状态迁移后共 25 项检查通过。另有 10 项专项单测及 6 项既有单测通过。包含 16 会话并发、三协议、重启及命名空间损坏处理。临时 Rust 单测源已移除，验证数据保留在 `session-cache-validation.json`；集成测试脚本及目录的清理状态见下文。

已部署并重启本地 CPA `60892`，安装库与通过测试的库 SHA256 均为 `4fa4338f4bc709e02cc2d74d8e5a3181926a5a0197bc5622d2ce41ae8be35010`。命名空间已持久化，原凭证文件哈希未改变，53 个模型保持一致。

部署验收按当前配置解析数据目录；本机实际目录是 `temp/integration/ui-profile/real-plugin-data`，旧 `plugin-data` 模拟目录不能作为真实状态验收依据。备份附带 `source-paths.json` 标明实际来源。

部署记录见 [验证数据](session-cache-validation.json)。首次升级会替换旧随机映射；之后保留插件数据目录即可避免重启再次改变标识。

## 未闭环项与下一步

- 临时文件清理未完成：本机自动审批策略拒绝删除操作；改用已核验的单文件绝对路径仍返回 `blocked by policy`，未提供更具体原因。残留清单见验证 JSON 的 `temporary_cleanup`；未改用其他工具绕过拒绝，现有配置、凭证和部署备份保留。
- 未使用真实 Factory 账号执行此次推理，因此没有真实缓存命中率或历史串话结论；如需量化收益，应在相同模型、相同前缀和缓存有效期内做受控对照。
- 其他提供商目前只有当前运行版本的源码证据，没有逐账号上游验收，不将本次 Factory 修复推广成“所有提供商完全不会串话”的保证。
- 无明确会话 ID 的独立请求无法仅凭相同内容判定是否同一对话。需要稳定对话边界的调用方应发送 Session-Id、明确 conversation 或等效会话字段。

官方契约来源（2026-11-04 已读取）：

```text
https://developers.openai.com/api/docs/guides/prompt-caching.md
https://developers.openai.com/api/docs/guides/conversation-state.md
https://platform.claude.com/docs/en/build-with-claude/prompt-caching
```
