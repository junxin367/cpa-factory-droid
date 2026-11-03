# 本机集成验证记录

验证日期：2026-11-03（Asia/Shanghai）。

本次收尾结论：**最新 ZIP 实装、插件注册、健康检查与资源指纹通过；新界面的获取候选、测试和创建流程未完整验收。** 最新包已实际安装到 `127.0.0.1:60892` 和 `127.0.0.1:53159`，两处均记录 `registered=true`、`effective_enabled=true`、`health=ready`。直接页已检查桌面约 640px 抽屉、移动端全宽、移除三步面包屑后的上下连续表单、固定底部操作、紧凑额度行，以及删除“用量与预算”“模型配置”“使用配置”三个 Tab 后的账号单页布局；这些布局检查不代表完整业务流程已通过。

最终安装包 `release/factory-droid_0.1.0_windows_amd64.zip` 的 SHA256 为 `8d18fd1425da899fe4304bb280cad054c2e3434f6aa75102b2d4f1c8fef890de`；包内与两处实装 DLL 的 SHA256 同为 `f7aa9f8714842b324ac7697771a24b0cdfb78c854f6359467450349f5e70d66b`。两处 HTTP 返回的 HTML、JS、CSS 指纹均与冻结源码一致，官方管理面板指纹也匹配。证据分别为 `temp/integration/ui-profile/evidence/final-package-install.json` 与 `temp/integration/live/evidence/final-package-install.json`；UI 安装证据中的待验收状态不作为交互已通过的证明。

60892 的 `api-base-url` 保持 `https://api.factory.ai`，填写 Mock Key 后点击获取候选收到官方上游 `401`，因此获取候选、测试和创建流程未完成；已按要求停止排查，未修改该配置。账号目录、草稿、别名/价格覆盖及官方周期统计的 **42 个受控场景已通过**，详见 `temp/integration/pipeline/evidence/current-stage-summary.json`；该矩阵基于中间 DLL，不作为最终 ZIP 重跑的证明。53159 已接入本机官方 Droid CLI `1.2.0`，已有证据记录 `cli_discovery_configured=true`、账号数与活动请求均为 0，供用户后续自行填写真实 Key；依赖证据为 `temp/integration/live/evidence/official-cli-dependency.json`。真实 Factory 推理和真实账号权限仍未验证；未操作 `cpahd-manager.faibak.com`。

> 以下保留各阶段过程记录。旧包指纹、1040px 抽屉以及“待验收”“待同步”等描述仅代表当时状态；当前结论以上述收尾记录为准，旧版宿主截图与流程结果不替代新版完整验收。

严格原生核心已通过三族原生非流/流和真正两轮保真、12 项跨协议拒绝、用量独立 oracle、门禁、热发布、取消/背压、断流和非法终态检查。账号目录、按账号允许、草稿新建和周期统计的本机 Mock 流程已通过；上一版代码验收 ZIP 已实际安装，1040px 宽抽屉曾通过宿主验收。本次 640px 连续表单的最新进度以当前状态表为准；真实 Factory 账号仍未验证。严格原生证据独立存于 `temp/integration/runtime/evidence/strict-native/`。

## 本轮新增功能的实际状态

下表记录当前独立验收状态，旧转换架构和旧核心矩阵均不能替代这些项目：

| 项目 | 状态 | 已有证据与未完成项 |
| --- | --- | --- |
| 账号目录与按账号验证允许 | 本轮本机核心与边界验证通过 | 21 组核心宿主断言及后述边界补强通过，包括两账号候选隔离、发现零推理/零发布、confirm 前置门禁、固定基础验证恰一次 validation、候选禁用项、手工来源、固定账号 Key、撤销/路由变更同步、429/500 保留证明、迟到验证不复活、重启、删除重加及 CLI 隔离清理 |
| AI 提供商草稿新建 | 核心 API 通过；640px 连续表单 UI 待验收 | 上一版宿主抽屉流程通过；用户追加的窄抽屉、连续表单及固定底部操作须从新 ZIP 实装后重做宿主 UI 验收 |
| 每官方周期 M/$ 统计 | 后台受控账本验收通过；新紧凑额度行待 UI 验收 | 保留 6 组管理 API 与独立 oracle 通过结果；当前新增已用/预测总量的额度行展示须分别核对实际计量、未知预测、多周期与窄屏，不将后台结果直接当作新 UI 已验收 |
| 新代码 ZIP 实装 | 最新 UI 代码包已安装；宿主交互待验收 | 16:51 的 640px 连续表单代码包已从 ZIP 实际安装到 60892，DLL/资源、原生注册和 health 通过；交互验收后同步 53159 并核最终文档包 |
| 真实 Factory 账号 | 未验证 | 没有发真实推理；保留回环 Live 预备实例，未操作生产管理站点 |

详细矩阵维护于 `temp/integration/model-quality-plan.md`。本轮测试仅访问回环 CPA、Mock Factory 和受控 CLI 替身，不将源码存在或编译成功写为运行验收通过。

本轮实际使用的中间 DLL SHA256 为 `b01b7a49b93cea6d47095b391ebb983149544da773c37c076a1f0687c39576ee`，CPA/Mock 分别是 `127.0.0.1:58299` 和独立分配的回环端口。原 21 组中畸形目录组曾因旧断言期待统一 `invalid_catalog` 而失败；按确认后的精确契约单独重测通过：重复 ID=`discovery_duplicate_model`、空 ID=`discovery_invalid_model`、空目录=`discovery_invalid_catalog`，三者均 HTTP 502、保留上次快照并标 stale、零推理且发布集合不变。原失败证据保留，没有覆盖改写。

对应证据在 `temp/integration/pipeline/evidence/`：

- `account-model-summary.json`：首轮 20 通过、1 项错误码断言差异；`extension-summary.json` 包含该组的精确重测及 8 组草稿流程，9 组通过。
- `period-summary.json`：6 组周期统计通过；`period-oracle-seed.json` 保存受控账本输入。标准池独立结果为 170 token、0.000170 M、USD 0.25，validation 子合计 50 token、USD 0.05；core 为 1200 token，另一账号为 1000 token。
- v2 预测夹具在同账号/池/周期的两次快照之间增加 200 token、USD 0.4、使用率 1 个百分点，结果总量 20000 token、剩余 11300 token、总费用 USD 40、剩余 USD 22.6；这些是 Mock 估算夹具，不是官方余额。

前端使用独立实例 `127.0.0.1:60892` / Mock `127.0.0.1:60893` 和 `temp/integration/ui-profile/`，独立 CLI 副本及控制文件。最终安装版本的实际宿主浏览器验收已通过，明细与三份截图见 [前端宿主验收](frontend-ui-acceptance.md)。

边界补强另外 7 组闭环，当前共有 42 个唯一场景通过：401 使账号身份及模型授权失效，403 使对应模型证明失效；CLI 候选移除后撤销、恢复候选不自动允许；全局启停保存有效证明并同步注册；CLI 10 秒超时回收父进程、子进程和隔离目录；旧刷新迟到不能覆盖新成功快照；失败模型可保存但只发布有效子集，空 overrides 清除别名/价格覆盖且不重测。`edge-summary.json` 保存首轮 6 通过及一项敏感输出预期差异，`sensitive-final-summary.json` 保存单组复核：含完整 Key 的 stdout 目录被 `discovery_sensitive_output` 拒绝、旧快照保留并标 stale；stderr 被消费后丢弃，不阻止有效 stdout；持久状态、宿主日志和 HTTP 证据均未包含完整 Key，安全未知元数据保留。此前错误断言的证据没有覆盖或删除。

### 本次本机环境证据

用户验收使用实际安装后的本机 CPA 宿主管理界面，经插件 iframe 点击“新建”检查右侧抽屉。独立 UI 环境已安装官方管理前端 `router-for-me/Cli-Proxy-API-Management-Center v1.27.2`；`60892/management.html` 实际 HTTP 内容与官方发布资产 SHA256 `2f6cf57a8ca4ace1fadde0280cbd41084f56999738d0e692a30b135420f7fc30` 一致，自动更新关闭，证据为 `ui-profile/evidence/management-panel-install.json`。

当前实际安装的是 16:12 第三版代码验收包：ZIP SHA256 `5de8c18f657b0b366e9cfdb7e7127d86e4fd3d206be8191b12cdeb980a64cdf1`，包内、60892 和 53159 已安装 `factory-droid.dll` SHA256 同为 `91473c6a75af919f05c8d1c253006a1e2379ca0b556f8265ff02986ffcdb1e24`；原生 `registered=true/effective_enabled=true`。两处 HTTP 资源逐字等于冻结源码：HTML `48576027707390c23a37d04dc9cd8e6a8532931829c905e1310d600ac80ab4a4`、JS `0e10eb99bc3c1a138a480a7cf67d1228ea03b4cc6e440647f618c891b683b0af`、CSS `ef885c1b4cba62d4556c295d8323aedb591c9f4afc5c2275c47b0e282910ca3f`。第三版包含桌面滚动条右缝、宿主工具栏关闭按钮避让及 rememberPassword 显式门禁修正；首次启动即 `health=ready`，没有额外调用同步。实际宿主桌面视口 `1920×855` 下抽屉右边缘为 1920、高 855；移动视口 `390×844` 下抽屉覆盖 0~390、高 844，关闭按钮避开宿主工具栏。鼠标关闭、Esc、焦点返回、Key 清空、候选多选、消耗确认门禁及取消后草稿 404 均通过，console error 为 0。截图文档重封后再核最终 ZIP 与运行库一致性，代码验收 ZIP 指纹不冒充重封后的最终 ZIP 指纹。

历史安装观察单独保留：15:57 第一包升级旧 WIP 账号时，首次状态显示目录未同步、`health=degraded`，显式 `/models/reconcile` 后变为 ready；该记录保留为 `ui-profile/evidence/final-package-install-initial.json`，不写成启动即成功。第二包与第三包首次状态均为 ready；旧 WIP 账号的证明已因凭据指纹变化而失效，因此 UI 空发布集合不作为“有效模型恢复”的证据。另在独立空 `startup-profile/` 使用同冻结后端代码新建 Mock 账号并明确验证 GLM，重启后首次及 0/2/4/6/8/10 秒始终 ready，共享与专属两模型始终存在，重启新增 LLM 为 0，无手工 reconcile；证据为 `startup-profile/evidence/final-dll-valid-proof-startup.json`。

用户自行填写真实 Key 的 `53159/management.html` 入口已恢复并更新到第三版：每次更新前只读确认账号数 0、auth 文件数 0、活动请求 0，保留原管理密钥及配置，启用同指纹的官方管理前端。启动后 `registered/effective=true`、`health=ready`、账号数和活动请求均为 0，三份资源与上述冻结源码逐字相同。只查询本机状态和静态资源，没有向真实 Factory 发送 whoami 或推理。证据为 `live/evidence/final-package-install.json`；真实凭据和模型权限仍未验证。

**本次环境的 CLI 依赖，不是通用默认路径：**53159 的 `droid-cli-path` 为 `E:/code/junes/github/cpa-factory-droid/temp/research/factory-model-discovery/cli/package/bin/droid.exe`，版本 `1.2.0`。官方研究包 SHA1 `6b8baddbc6a35c833e2f526eeb535696231c8928` 重新核对通过，现有 EXE 与包内成员 SHA256 均为 `b075e952982d0b3c731e875dfd4e95c835c4d1a8d9e46c791636ebd8116fd754`。仅在隔离 HOME/USERPROFILE/Factory 目录且无 Key 的环境执行 `--version`，退出码 0、输出 1.2.0；未调用 Factory 身份、目录或推理。新增配置后按宿主要求重启，最终 HTTP `/status.capabilities.cli_discovery_configured=true`、health ready、账号及活动请求仍为 0。证据为 `live/evidence/official-cli-dependency.json`。CLI 只保留在上述本地位置，不随插件 ZIP 分发；其他安装环境须指向其自身已安装的官方 CLI。

## 旧转换架构历史记录

> 以下内容仅记录旧转换架构截至 13:03 的历史验收，已经被严格原生协议政策替代。旧协议转换矩阵、fdctx 和 1 小时缓存均不是当前版本的验收或限制。

旧阶段结论：**当时本机 Mock 验收通过，真实账号未验证**。以下数字、DLL 指纹和签名缓存约束仅对应旧阶段，不代表当前目标。

## 环境与安装

| 项目 | 实际值 |
| --- | --- |
| CPA 宿主 | 官方 Windows amd64 发布包 `v8.0.52`，commit `6e6afab9` |
| 宿主包 SHA256 | `395324dd124bd657bccd5fdf06119587cd7c87bddacff191d5f45431e8f08d23`，已与官方 `checksums.txt` 核对 |
| 插件 | `0.1.0`，Rust `cdylib`，开发构建 `cpa_factory_droid.dll` |
| 实际安装 | 复制并命名为独立插件目录中的 `factory-droid.dll` |
| 当前验证 DLL SHA256 | `2e945fdb3b0051b0f7893cca96fb95feccc07deb05d1f869865d26e1e10544c7` |
| Mock CPA | `http://127.0.0.1:62595`，独立配置、auth、plugins、data |
| Mock Factory | `http://127.0.0.1:62596`，不联系真实 Factory |
| 正式连接预备实例 | `http://127.0.0.1:53159`，独立空 auth/data/plugins，使用默认官方 Factory 地址 |

宿主管理 API 实际返回 `X-CPA-Support-Plugin: 1`；不是使用不支持插件的宿主推断 DLL 失败。常用路径未发现 Go/Docker，本次直接使用官方支持插件的发布包，没有修改已有服务。

启动命令使用独立配置：

```powershell
& "<隔离目录>\bin\cli-proxy-api.exe" -config "<隔离目录>\config.yaml" -local-model
```

本机运行目录位于 `temp/integration/runtime/` 与 `temp/integration/live/`。二者仅监听回环地址，管理密钥、客户端密钥分别随机生成，不写入源码、验证报告或 URL。正式连接实例的管理密钥文件为 `temp/integration/live/management-key.txt`；该文件仅供本地使用，不应发布。

## 已通过的 HTTP 验证

全部请求经过实际 CPA HTTP 接口和已加载 DLL，没有用插件 RPC 自测替代宿主集成。

| 范围 | 观察结果 |
| --- | --- |
| 元信息与菜单 | `registered=true`、`effective_enabled=true`；metadata、Factory Droid 菜单及 quota capability 可查询 |
| 静态资源 | `index.html`、`app.js`、`style.css` 匿名请求为 200，正文非空 |
| 管理认证 | 匿名请求插件 `/status` 为 401；正确管理认证可访问受保护 API |
| 授权 | 无效 Mock Key 为 401；合法 Key 保存成功；相同 Key 重复添加为 409 |
| 模型注册 | 未授权时无 `factory/` 模型；共享目录 13 个，当前版本另为每个账号注册 13 个固定账号模型；停用及删除后相应目录撤下 |
| 账号启停 | 停用显示 `disabled`；启用恢复 `ready`，`enabled` 与模型可用状态一致 |
| 删除 | 删除前停用凭据，旧凭据中的 Key 被清除；历史用量保留；相同 Key 可重新添加 |
| 请求约束 | Mock 只接受调研确认的 `/api/llm/*` 路径；extended whoami、客户端头通过；API Key 请求未携带 `X-Factory-Org-Id` |
| 系统指令 | 原始业务 system/instructions 在实际上游请求中保留 |
| 错误 | 上游 401、429、500 分别返回相应状态；429 保留 `Retry-After`；响应为脱敏错误 |
| 断流 | 已开始的流缺少完成事件时返回明确流错误，不计作成功 token 用量 |
| 取消 | 客户端读取首帧后断开，Mock 观察到上游连接取消，插件活动请求恢复为 0 |
| 重启 | 账号启用状态、标签、优先级、设置、请求数及用量恢复；浮点持久化按 `1e-12` 容差比较 |
| 脱敏 | 对 Mock 日志、HTTP 证据及插件 data 的完整 Key 扫描未命中；凭据文件与本地运行密钥文件除外 |

## 协议矩阵与补强回归

基础矩阵为 **36 个用例**：

- 入站：`/v1/chat/completions`、`/v1/responses`、`/v1/messages`。
- 上游：Anthropic Messages、OpenAI Responses、OpenAI Chat Completions。
- 每个组合分别验证非流式、SSE，以及普通文本、单工具调用。

使用模型映射 `factory/claude-opus-5-5`、`factory/gpt-6.1-sol`、`factory/glm-5.3`。模型 ID 来自插件配置，Mock 成功不证明真实账号已开通这些模型。

补强后的 Mock Responses 成功对象始终包含 `error:null` 和真实形状的 output/usage；修复版 DLL 已重新通过 36 个用例。SSE 检查了各入站协议的终结事件、有效 usage 及无重复 `data:` framing。

已另外验证：

1. 三类上游的 CRLF 首帧与 LF 次帧合并在一次写入中，仍能正确解析。
2. Chat 流仅有 partial + `[DONE]`、缺少 `finish_reason` 时，返回错误且成功数不增加。
3. 插件网络配置变更被拒绝并恢复旧配置后，仍可继续执行请求。
4. Anthropic 流 `raw input=11`、cache read=2、cache write=3 转为 OpenAI prompt=16；终帧显式零值能够覆盖早期计数，缺失字段能够保留。
5. 三类上游的基本工具结果续接均通过：工具调用回包后发送 `tool_result`/工具结果，得到正常文本。

含签名思考的续接已追加 **6 个两轮用例**：Chat、Responses、Messages 分别测试非流式和流式；第一轮包含 thinking/signature、redacted thinking 和工具调用，流式包含 `signature_delta`。探针将第一轮返回的 assistant（Responses 为 output items）原样加入第二轮；Mock 严格检查第二轮上游 thinking 原文、签名、redacted data、工具原始 ID 和块顺序，6 例全部通过。

## 多账号与签名上下文

同时启用两个不同 Mock Key 时，验证了两套 `factory/<account-id>/claude-opus-5-5`：

- 每套入口连续 3 次只命中对应的 Mock 凭据，没有被轮询到另一个账号。
- 每个账号固定入口的非流式和流式签名续接另外通过 2 例，共 4 例。合计 10 个第二轮上游请求都经过上述严格块校验。
- 多账号共享 `factory/<model>` 入口携带 Anthropic tools 时，在发送上游前返回 409，提示选择固定账号模型；Mock 推理请求数前后不变。
- 变更已签名回合的工具参数、改用其他模型、重启后重放旧回合均返回 409。不能修改或丢弃返回的 `fdctx_...` 工具 ID。
- 固定账号入口的用量按原模型统一统计，账号归属正确；没有 `unknown` 账号账本。

签名原文只存在插件进程的有界缓存中，实现期限为 1 小时、最多 256 回合与 32 MiB。宿主重启后需重新开始该工具回合；该限制已通过重启重放测试确认。普通持久化用量和账号不受此缓存失效影响。

## 用量、官方额度与估算

- 第一轮基础矩阵产生 36 次成功请求，账本恰为 36 次成功、36 次有 usage，没有重复计数。
- 缺少 usage 的非流式与流式请求只增加请求数，不增加 token 或估算值；失败、取消不形成成功 token 账本。
- 截至 13:03 汇总，账本有 184 个唯一 RequestID，172 次成功、12 次失败；其中 4 次为发送上游前主动拒绝的 409。后续操作会继续增加，不应把这个时点总数作为固定验收值。
- 未配置价格时美元估算保持 `null`。配置 uncached input、output、cache read 分价后，受控请求计算为 USD `0.0000232`；已有缺价历史使聚合美元值保持未知。
- 官方 Analytics、billing limits 和本地估算分开显示：无权限、格式未知或服务错误时为不可用/未知，不替换为零余额。
- Mock 合法样本验证 FSC `123498`、官方额外用量 USD `12.34`，以及官方滚动窗口使用率。这些都是夹具数据，**不是真实账号余额**。
- 已检查 reset 的 epoch 数值字符串、`secondsRemaining`、已过期窗口及 `null` 百分比；过期或未知窗口不伪造可用额度。
- 预测样本：同一账号和窗口使用率由 42.5% 增至 43.5%，期间 6 次请求共 120 原始 token，推算总容量 12000、剩余约 6780；结果明确标注估算，USD 缺价仍未知，FSC 不参与该公式。

## 证据与当前限制

本机原始证据位于 `temp/integration/runtime/evidence/`：`results.jsonl` 保存响应与断言，`mock-events.jsonl` 保存脱敏上游请求，`final-summary.json` 保存汇总。临时探针位于 `temp/integration/`，不作为永久测试文件发布。

早期错误用例曾被 CPA 模型冷却期阻挡，已改用独立模型重测；界面验收与探针并行启停账号时出现的 `model_not_found`，已在协调独占时段后重测。两者未归因于插件缺陷。

界面验收另有独立记录，本文件仅对 HTTP 与本机宿主验证负责。真实 Factory 的 whoami、区域主机路由、模型权限、私有 HTTP 契约和官方额度权限均待用户本地提供 Key 后验证。正式连接实例在启动时已核验账号数为 0，没有发送真实推理请求；其最后一版 DLL 更新需与正在进行的本地登录流程协调。
