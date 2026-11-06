# Factory OAuth 登录

在 CPA 或 CPA-Manager-Plus 的 **OAuth 登录** 页面选择 **Factory Droid**，打开返回的 Factory 官方链接完成登录。授权成功后由 CPA 自动保存认证文件，模型、额度和请求统计沿用原生凭证流程。

不需要手工粘贴回调 URL、复制 refresh token 或逐个添加模型。插件原有 API Key 接入仍可使用。

## 授权与续期

- 使用 Factory 官方 Droid 客户端的 WorkOS 设备授权：`/user_management/authorize/device` 与 `/user_management/authenticate`。无需在 CPA 主机监听额外回调端口。
- 轮询遵守服务端 `interval`；`slow_down` 至少增加 5 秒，拒绝和过期停止授权。临时网络故障保留当前会话，已交换成功的设备码不会重复消费。
- 登录令牌没有组织范围时读取 `/api/cli/org`，与官方客户端一致使用返回的首个 WorkOS 组织，再通过 Factory `whoami` 确认实际组织、区域及专属主机。两类组织 ID 分开保存。
- 同一 Factory 用户在同一组织重新 OAuth 授权时，更新已有 OAuth 凭证中的令牌，保留原账号 ID、文件名、启停状态、名称、优先级和原生模型规则；API Key、其他用户和其他组织的凭证分别保存。首次授权使用用户 ID 与组织 ID 生成的稳定文件名，避免同时授权生成重复文件。缺少可信身份或凭据列表读取失败时不新建；已有多个重复项时固定选取一条更新，不自动清理旧项。
- CPA 依据 `expires_at`、`refresh_interval_seconds` 和 `NextRefreshAfter` 调度 `auth.refresh`。按真实到期时间提前 120 秒续期，不把官方“30 天轮换”说明当作 access token 的固定有效期。
- 同一凭证的并发刷新串行执行；已轮换的旧 refresh token 复用内存中的新结果。身份或目录接口暂时失败时，仍交回新令牌给 CPA 保存，避免丢失轮换结果。
- 密钥和令牌只保存在 CPA 认证文件；插件状态仅保留目录与脱敏身份。设备码会话最多 32 个，过期、插件停用或重启后失效。不读取用户现有 Droid 登录存储。
- OAuth 模型目录使用隔离且未登录的官方 CLI。令牌不放入 `FACTORY_API_KEY`，也不写入 CLI 配置。目录失败时保留已有目录或使用内置目录；实际账号调用权限仍由 Factory 判断。

默认只向 `api.workos.com` 和校验过的 Factory 主机发送授权请求。开发配置同时设置 `allow-local-mock: true` 与本机 `api-base-url` 时，设备授权和令牌交换也指向该本机模拟服务。

## 验证记录

日期：2026-11-03 至 2026-11-04（Asia/Shanghai）。CPA v8.0.52。

- Rust 既有单元测试 6 项通过。
- 隔离 CPA 27 项检查通过，覆盖设备授权、原生保存、目录加载、自动续期、轮换持久化、模拟推理、CPA 原生统计、额度查询、重启恢复、拒绝、过期、pending、slow_down 和非受信任登录地址。
- ABI 隔离回归 15 项通过，覆盖并发续期、先于模型缓存加载的续期、身份查询失败后的令牌保留，以及续期中的凭证停用和名称修改。明细见 [验收数据](oauth-validation.json)。
- 真实 WorkOS 设备授权接口返回 HTTP 200，登录域名为 `auth.factory.ai`，有效期 300 秒、轮询间隔 5 秒。
- 用户选择暂不登录真实账号，待授权页面已关闭。真实账号的浏览器确认、令牌交换及随后真实上游推理尚未验收；模拟验证不代表真实账号已授权。
- 本地 `60892` 已部署并重启，保留原 API Key 凭据的 53 个模型，账号 UUID 模型为 0。Chrome 已验证原生 OAuth 弹窗可生成并打开 Factory 官方设备确认页，Manager Plus `18317` 也自动出现 Factory Droid 登录入口，无需修改其前端。

官方资料：[Factory Security](https://docs.factory.com/enterprise/security)、[WorkOS CLI Auth](https://workos.com/docs/reference/authkit/cli-auth)。
