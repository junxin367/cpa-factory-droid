# 调研记录

核验日期：2026-11-03（Asia/Shanghai）。以下版本、规模和维护状态都是该时点观察，不代表持续保证；本次源码调研没有使用真实 Factory 账号完成上游调用。

## 结论与选择

采用 CPA 原生 Rust 插件，通过 CPA OAuth 登录或用户填写 Factory API Key 接入，并通过 CPA 的 HTTP callback 直接访问 Droid 网关。这样可以复用 CPA 的凭据、代理、取消和 usage 集成，也避免引入常驻 CLI 子进程。

本版采用 strict native-only：模型配置显式决定 Messages、Responses 或 Chat 入口，仅允许同协议请求与响应，六种标准跨协议组合在 Factory 请求前拒绝。原生字段、工具 ID、签名和 SSE 保留，不使用协议转换器。CPA v8.0.52 更早的 HTTP handler 兼容修正及其他宿主改写仍是已知边界，详见 [原生协议与调用限制](protocol.md)。

官方 CLI 只在管理员配置后用于短时读取 `droid.list_models` 候选，不用于实际推理，也不随插件包分发。本次对官方 CLI 的独立检查确认：故意无效的 Key 仍能得到 60 个候选。因此 CLI 目录不能作为模型权限证明；最终采用 whoami、显式固定基础调用、账号允许意图与 CPA 实际发布的分离流程，见 [账号与模型管理](models.md)。

直接 HTTP 是 Droid 私有客户端契约，不是 Factory 公开承诺稳定的推理 API。模型请求先经身份接口解析区域及专属主机，再根据模型映射选择 Anthropic、Responses 或 Chat Completions 路径。账号实际能用哪些模型、Key 能否读取官方限额，都需要当前账号响应验证。

已根据 WorkOS 官方规范接入设备码 OAuth、组织范围恢复和 refresh token 轮换；公开客户端 ID 与本机官方 Droid 原生二进制相符。真实设备授权接口已返回 Factory 官方登录地址，真实账号登录验收状态见 [OAuth 登录](oauth.md)。SDK/CLI 推理桥只作为研究对照，不提供直接 HTTP 失败后的执行备用通道。

## CPA 宿主与参考实现

| 项目 | 本次锁定版本 / 提交 | 用途 |
| --- | --- | --- |
| [router-for-me/CLIProxyAPI](https://github.com/router-for-me/CLIProxyAPI) | `v8.0.52`，`6e6afab99a1939525ecb6bb1f0873c8e7a9df877` | 原生 C ABI、RPC schema 10、auth / HTTP / stream callback、quota、管理资源及 pluginstore |
| [router-for-me/Cli-Proxy-API-Management-Center](https://github.com/router-for-me/Cli-Proxy-API-Management-Center) | `f154fa9380825c4fe94bc577a9abaa7e505f8875` | 官方插件菜单、iframe 和通用插件 quota 渲染 |
| [junxin367/cpa-apikey-manager](https://github.com/junxin367/cpa-apikey-manager) | `64cbf8d38963574e457bd726c0c18687fe9169a6`，Cargo `0.6.14` | Rust ABI 分配/释放、内嵌管理页、schema 1 注册表、安装包与 GitHub 维护方式 |
| [seakee/CPA-Manager-Plus](https://github.com/seakee/CPA-Manager-Plus) | `v1.14.10`，`a67ed8834a914ab24c36d2490fa6bb6d4166724c` | 用户现有管理端的通用账号历史、固定 provider 额度、独立价格表和同源认证 |

CPA `v8.0.52` 是本项目验收基线。不能仅因较低 RPC schema 可以注册，就宣称旧 CPA 版本支持当前 HTTP 取消、流转发和额度接口。原生动态库加载需要带 CGO 的宿主构建。

商店契约核自宿主 `internal/pluginstore/registry.go`、`github.go`、`install.go` 和参考注册表：`schema_version: 1`、`plugins[]`、一致的插件 ID / repository / version；版本号不带 `v`，Release 标签带 `v`。对应 ZIP 根目录必须包含匹配插件名的动态库，Release 提供 `checksums.txt`。本项目注册表没有编造尚不存在的 Release 资产 URL。

CPAMP v1.14.10 的通用账号历史支持 `factory-droid` provider 和宿主 `auth_index`，但 `resolveAccountQuota` 与刷新流程只接固定 provider。插件的标准 quota 不能自动出现在其原生账号卡片中；详情见 [Manager Plus 兼容说明](manager-plus.md)。Manager Server / 管理面板 v1.14.10 不是 CPA 内核版本，不能代替内核兼容核验。

## Factory 的独立交叉证据

下表包含直接网关、SDK/CLI 桥和独立额度客户端等路线。`magpie-community/plugins` 与 magpie 同源，不重复计作独立实现。

| 项目 | 本次锁定提交 | 规模与维护观察 | 采用与未采用的部分 |
| --- | --- | --- | --- |
| [yetone/magpie](https://github.com/yetone/magpie) | `6b813443019ce385fbc7a1d2a320bd22e4193a1e` | 页面约 28.4k stars、2.4k forks、6,237 commits；2026-11-03 仍更新 | 大规模实际网关的身份、组织、区域和额度结构；当前 Factory 请求实现已迁移至插件仓库 |
| [magpie-community/plugins](https://github.com/magpie-community/plugins) | `da94c2768af9edfa24e87b2ac77dcdb4e8f2c9e3` | API 记录 187 stars、252 forks，2026-11-03 推送 | 当前 Factory 协议、OAuth、usage 测试及 Droid 0.242.0 / 0.251.0 客户端样本；与 magpie 同源 |
| [1e0n/droid2api](https://github.com/1e0n/droid2api) | `9613b95d1137c950276e143f49a36129f0c89fa8` | 页面 359 stars、147 forks、58 commits；锁定提交时间为 2026-03-25 | 独立 Node 直接 HTTP 路线，交叉确认路由和 WorkOS client_id；不照搬固定 token 有效期、敏感日志或旧凭据文件假设 |
| [mrwogu/factory-droid-openai](https://github.com/mrwogu/factory-droid-openai) | `8092260a6d859a00767fbd2363cc5137e8129f48` | API 记录 16 stars、1 fork、4 issues；2026-11-02 仍有推送 | SDK/CLI JSON-RPC 桥、动态模型和测试矩阵；其 sampling 参数、工具与多模态限制不适合当成原生 HTTP 透传 |
| [tjboudreaux/pi-provider-factory](https://github.com/tjboudreaux/pi-provider-factory) | `300c1c0f791c956cf70a22026994a863804a803f` | API 记录 6 stars、3 forks；提交时间为 2026-08-06 | 独立直接 HTTP / OAuth、脱敏 live probe、Droid 0.184.0 契约；注释和验证文件冲突时采用实际验证证据 |
| [heyhuynhgiabuu/pi-factory-droid](https://github.com/heyhuynhgiabuu/pi-factory-droid) | `5b16ee9542f6b7a2afde313656daa82ea45daf4a` | API 记录 1 star；提交时间为 2026-10-27 | 较新直接 HTTP、Droid identity、模型发现与 provider routing 交叉验证；不作为唯一依据 |
| [steipete/CodexBar](https://github.com/steipete/CodexBar) | `897bff31731bb4b21b105f6e8bf4c32a8a8bf095` | 页面约 23k stars、2.2k forks、7,696 commits；本次读取 HEAD | 独立额度客户端的 Key、billing limits 与旧订阅用量路径；用于核对权限和数据单位 |
| [Factory-AI/droid-sdk-typescript](https://github.com/Factory-AI/droid-sdk-typescript) | `4e2c75277e4da54e9afca1977472063ea0a4cce2` | API 记录 43 stars、5 forks；提交时间为 2026-10-20，包 `0.10.0` | 官方 SDK 的 API Key、process transport、会话与 token usage；不等同于公开 HTTP 推理接口 |
| [Factory-AI/factory](https://github.com/Factory-AI/factory) | `892925c655a97e7a0bc7d558236015ebb22cb214` | API 记录 79 stars、6 forks；提交时间为 2026-10-23 | 官方文档源码；在线文档更新较快，Analytics 角色和套餐限制优先核对在线页面 |

选择依据不是 stars 排名。优先级是：与“Factory 账号接入 CPA”的方向是否一致、是否有近期客户端样本、关键请求能否由独立来源交叉确认、是否保留工具/流/usage 语义、错误和凭据处理是否可靠、测试与维护是否能支持后续变更。小项目的抓取契约可能比大项目更直接，大项目的实现也可能过期或已经迁移。

例如 `automazeio/vibeproxy` 主要将其他订阅接入 Droid，方向与本项目相反；高 stars 不能使它成为本项目的主要上游证据。magpie 与其插件只能算一条实现脉络。

## 官方资料

- [Factory CLI reference](https://docs.factory.com/reference/cli-reference)：`FACTORY_API_KEY` 与 CLI `/limits`。
- [Factory Security](https://docs.factory.com/enterprise/security)：CLI 凭据加密保存、令牌轮换及 CI/CD API Key。
- [Factory 模型目录](https://docs.factory.com/models)：公开模型与倍率资料，不能证明具体账号已获权限。
- [Factory Analytics](https://docs.factory.com/api-reference/analytics)：FSC 与原始 token、个人/组织接口、角色和套餐限制。
- [Factory 服务账号源码文档](https://github.com/Factory-AI/factory/blob/892925c655a97e7a0bc7d558236015ebb22cb214/docs/enterprise/service-accounts.mdx)：Enterprise 服务账号、Key 到期与禁用行为。
- [WorkOS CLI Auth reference](https://workos.com/docs/reference/authkit/cli-auth) 与 [Device Flow 指南](https://workos.com/docs/authkit/cli-auth)：设备码、轮询、`authorization_pending`、`slow_down`、过期与拒绝处理。
- [WorkOS refresh token 说明](https://workos.com/blog/why-your-app-needs-refresh-tokens-and-how-they-work)：刷新与轮换；不能据此把 access token 有效期固定为 30 天。

## 额度证据的冲突与采用规则

旧 Pi live probe 记录 API Key 调用 billing limits 返回 401；当前 CodexBar 的 API 模式会发送 Key，magpie 的 Key 账号也进入额度查询。由此只能得出“按当前凭据尝试读取并检查返回”，不能得出“Key 永远支持”或“Key 永远不支持”。

本版只尝试当前 billing limits 与公开个人 Analytics，不盲目遍历旧端点。官方 `usedPercent` 对应各自滚动窗口；没有绝对 allowance 时不推导剩余 token。`extraUsageBalanceCents` 是 USD 余额，Analytics 的 FSC 是已消耗单位，本地倍率产生的是加权 token，三者各自展示。

历史订阅端点在不同客户端中还有 GET / POST 差异；未做当前账号验证前不作为自动降级方案。官方接口查询 401 / 403 时保留不可读取状态；模型身份验证成功与额度权限是两件事。

## 待真实账号确认的范围

源码和模拟服务可以验证 ABI、同协议原生内容、错配拒绝、错误、取消、候选/验证/发布分离与账本行为，不能证明真实 Factory 账号的对应权限。本项目仍需按账号确认：whoami 的区域和专属主机、三类模型的小请求与原生工具续接、SSE 中断、上游完整 usage、billing limits 权限与窗口，以及 Analytics 套餐范围。Chat 自定义 event-first SSE 已确认受宿主重包装限制，未将该边界写成全量端到端保真通过。实际完成情况统一写入 [验证记录](validation.md)。
