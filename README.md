# Factory Droid

通过 OAuth 登录或 API Key 将 Factory 接入 CLIProxyAPI（CPA）的原生提供商插件。插件负责身份校验、官方模型目录、请求转发和额度查询；登录入口、凭证管理、模型调度及请求统计使用 CPA 原生能力。

## 使用方式

1. 安装并启用插件，配置官方 Droid CLI 的原生可执行文件路径。
2. 在 CPA 的 **OAuth 登录** 页面选择 **Factory Droid**，打开官方授权链接完成登录。也可在插件页面输入 Factory API Key 添加凭证。
3. 插件验证身份、读取目录，创建 CPA 认证文件，自动注册全部已支持且未被官方标记为禁用、废弃或自定义的模型。
4. 后续在 CPA 的认证文件页管理账号、优先级和模型规则，并查看用量及刷新额度。

不再需要手工添加模型、逐个允许或执行付费验证。读取目录不发送推理请求。
每个上游模型只有一个共享 ID，例如 `factory/claude-haiku-5-5`；不再发布带账号 UUID 的固定账号模型。多凭证由 CPA 调度。

## 自动目录与请求边界

已支持 Anthropic Messages、OpenAI Responses、OpenAI Chat Completions 三类原生协议。Google、自动路由、自定义/BYOK 和未知原生协议不会被冒充可调用模型。
未配置 Droid CLI 时使用内置目录；配置 CLI 后，添加和原生凭证刷新会同步官方目录。目录随 CPA 凭据保存，导入和重启不需要先打开插件页面。

目录表示支持的请求路由，并不保证某个账号拥有所有模型权限。实际权限、限流和上游错误由 Factory 返回，交给 CPA 记录和处理。OAuth 令牌由 CPA 调度插件自动续期；授权被撤销后需要重新登录。API Key 撤销或到期后需要更新，目录刷新不会生成新 Key。详见 [OAuth 登录](docs/oauth.md)。

插件继续负责原生请求、响应和 SSE 转发，运行时需要保持加载。完整协议限制见 [协议说明](docs/protocol.md)。

出站请求以官方 Droid CLI `1.2.0` 的本机采样为基线。会话和消息使用独立 UUID，移除外层代理、客户端和追踪元数据，不透传下游鉴权、转发头或管理端标识。规则及验证边界见 [出站协议与元数据](docs/outbound-protocol.md)。

## 原生统计与额度

- 请求次数、Token、成功率、历史记录和费用统一使用 CPA 及其管理端的数据，不再重复记录插件账本或提供另一套价格页面。
- 额度通过 CPA 的通用插件额度接口读取，返回官方标准 5 小时、7 天、30 天窗口与额外余额。读取失败保留错误，未知不当作零。
- 原生统计应启用：

```yaml
observability:
  usage:
    usage-statistics-enabled: true
```

CPA-Manager-Plus 完整模式从 CPA 用量通道采集新请求，价格在 Manager 中统一配置；旧插件账本不会伪造为新的 CPA 请求历史。

## 管理端兼容

CPA 官方管理端已支持通用插件额度。CPA-Manager-Plus v1.14.10 原版只对固定 provider 显示刷新按钮；从插件 v0.1.3 开始，打开一次 Factory Droid 页即可激活原生凭证页面的额度增强，无需修改或单独部署 Manager。
增强通过 CPA 的插件额度接口读取真实 Factory 额度，不伪装成 Devin 或 Codex。此前的源码补丁保留作历史参考，使用此增强不需要应用补丁。

详见 [Manager 适配](docs/integrations/cpa-manager-plus/README.md)、[模型目录](docs/models.md) 与 [原生统计](docs/usage.md)。

## 运行要求

- 当前支持与验收基线为 CPA v8.0.52，C ABI 1 / RPC Schema 7，需要带 CGO 原生插件支持的构建。插件声明实际使用的最低协议版本，不要求宿主支持多提供商扩展。
- Windows x64 需要 Microsoft Visual C++ v14 Redistributable；Linux x64 构建基线为 Ubuntu 22.04 / GLIBC 2.35；另提供 macOS Intel 和 Apple Silicon 原生动态库。
- 可登录的 Factory 账号或 API Key（`fk-` 开头），以及可访问 Factory 的 CPA 出站网络；OAuth 还需要访问 `api.workos.com`。
- 官方 Droid CLI 是独立依赖，不随插件分发。插件只用 CLI 查询模型目录，不以 CLI 执行推理。

## 安装

在 CPA 插件商店中添加本仓库的注册表源：

```text
https://raw.githubusercontent.com/junxin367/cpa-factory-droid/main/registry.json
```

也可从 [GitHub Releases](https://github.com/junxin367/cpa-factory-droid/releases) 下载对应平台的 ZIP，并按下列步骤手动安装。`checksums.txt` 校验 ZIP，包内 `SHA256SUMS.txt` 校验动态库，`latest.json` 提供 CPA 更新清单。

1. 使用对应系统及架构的 ZIP，或按下文从源码构建。
2. 解压，将 ZIP 根目录的 `factory-droid.dll`（Windows）、`factory-droid.so`（Linux）或 `factory-droid.dylib`（macOS）放入 CPA 的 `plugins` 目录。不要直接复制 Cargo 产物的 crate 文件名。
3. 在 CPA 配置中合并下列片段，保留已有插件配置：

   ```yaml
   plugins:
     enabled: true
     dir: "plugins"
     configs:
       factory-droid:
         enabled: true
         data-dir: "data/factory-droid"
   ```

4. 重启 CPA，确认插件列表中出现 `Factory Droid`，再从插件菜单打开页面。

`data-dir` 相对于 CPA 进程的工作目录，用于保存脱敏账号兼容状态和模型目录。API Key、OAuth access token 和 refresh token 由 CPA 的 auth 存储保存，不写入插件 YAML 或插件状态。

要自动同步完整的官方模型目录，先按 [Factory 官方 CLI 文档](https://docs.factory.com/reference/cli-reference) 独立安装 Droid，取得原生可执行文件的绝对路径，再补充插件配置。例如下面的路径只是占位示例，须替换成自己的安装位置：

```yaml
plugins:
  configs:
    factory-droid:
      droid-cli-path: "C:/Tools/Factory/droid.exe"
      discovery-timeout-seconds: 30
```

Linux 同样填写实际原生可执行文件的绝对路径。不要填写 `.cmd`、`.bat`、JavaScript 脚本或 shell 包装器。CLI 在隔离目录运行，不复用用户既有 Factory 登录配置；配置后重启插件。

可选的插件配置：

| 字段 | 默认值 | 用途 |
| --- | --- | --- |
| `data-dir` | `data/factory-droid` | 兼容状态与模型目录，需可写 |
| `factory-client-version` | `1.2.0` | 已采样的 Droid 客户端版本；修改前应重新核对实际报文 |
| `droid-cli-path` | 未配置 | 官方 Droid 原生可执行文件的绝对路径，仅用于读取候选 |
| `discovery-timeout-seconds` | `30` | 候选发现超时，范围 10–120 秒 |
| `upstream-error-body-limit-bytes` | `1048576` | 上游错误正文最多读取 1 MiB，可配置 1024–16777216 字节；不额外设置错误读取超时 |
| `api-base-url` | `https://api.factory.ai` | Factory API 地址，仅接受受信任 Factory HTTPS 域名 |
| `allow-local-mock` | `false` | 开发联调开关，仅在显式开启时允许本机模拟服务 |

数据目录或网络配置变更需要重启插件。正常使用不需要配置模拟服务。

## 调用模型

客户端连接 **CPA 地址**，使用 **CPA 客户端 API Key**；Factory 凭证已由插件后端使用。先通过 `/v1/models` 或 CPA 原生凭证抽屉确认实际发布的 `factory/` 模型 ID。

**按模型配置的 `protocol` 选择入口，不能仅看模型名称。** 自动注册的模型沿用其官方目录对应的原生协议：

| 模型配置 `protocol` | 调用入口 | 示例模型 |
| --- | --- | --- |
| `anthropic` | `/v1/messages` | `factory/claude-opus-5-5` |
| `responses` | `/v1/responses` | `factory/gpt-5.4` |
| `chat_completions` | `/v1/chat/completions` | `factory/glm-5.3` |

以下均是 CPA 请求。把 `CPA_CLIENT_API_KEY` 替换为已配置的 CPA 客户端密钥，并使用自己的服务地址。示例模型仍须当前账号有权限且映射已启用。

Anthropic Messages：

```http
POST http://127.0.0.1:8317/v1/messages
Authorization: Bearer CPA_CLIENT_API_KEY
anthropic-version: 2023-06-01
Content-Type: application/json

{
  "model": "factory/claude-opus-5-5",
  "messages": [{"role": "user", "content": "请用一句话介绍你自己。"}],
  "max_tokens": 128,
  "stream": false
}
```

OpenAI Responses：

```http
POST http://127.0.0.1:8317/v1/responses
Authorization: Bearer CPA_CLIENT_API_KEY
Content-Type: application/json

{
  "model": "factory/gpt-5.4",
  "input": "请用一句话介绍你自己。",
  "max_output_tokens": 128,
  "store": false,
  "stream": false
}
```

OpenAI Chat Completions：

```http
POST http://127.0.0.1:8317/v1/chat/completions
Authorization: Bearer CPA_CLIENT_API_KEY
Content-Type: application/json

{
  "model": "factory/glm-5.3",
  "messages": [{"role": "user", "content": "请用一句话介绍你自己。"}],
  "max_tokens": 128,
  "stream": false
}
```

需要 SSE 时在对应原生请求中设置 `"stream": true`。客户端使用 CPA 客户端密钥，Factory 授权信息保存在 CPA 凭据文件中。

## 升级与恢复

升级前备份 CPA 的 `auth-dir`、插件 `data-dir` 和动态库，再替换插件并重启 CPA。
旧凭据已有的官方目录会自动纳入注册；旧的逐模型允许和验证状态不再限制注册集合。更新后的凭据携带目录，可以独立导出、导入并恢复。
此前的 `factory/<账号ID>/<模型>` 不再发布，客户端改用 `factory/<模型>`；账号选择、启停和模型排除交给 CPA。

旧插件账本和价格字段保留用于兼容读取，不自动删除或转换成 CPA 历史。卸载前可在 CPA 原生凭证页移除账号。一个插件数据目录只供一个 CPA 进程使用。

版本变化见 [v0.1.10 发布说明](docs/releases/v0.1.10.md)，请求处理与模型能力验收见 [本机验证记录](docs/validation.md)，跨宿主支持范围见 [兼容性验证记录](docs/compatibility-validation.md)。

### CPA Manager Plus 原生额度

在 CPA Manager Plus 中打开一次侧边栏的 **Factory Droid**，连接成功后显示“原生额度已启用”。返回凭证管理，Factory 凭证的卡片、列表和详情抽屉会出现刷新额度按钮，并展示官方 5 小时、7 天和月额度；月额度仍使用 Standard 额度池，未返回的窗口不虚构数值。凭证总体请求数、Token 和费用仍由 CPA / Manager 自身统计。

增强脚本和样式随插件动态库一起发布，无需安装 JS Handler 或替换 `management.html`。它只适用于插件 iframe 和 Manager 父页面同源且允许访问父页面的部署，当前适配 CPA Manager Plus v1.14.10。

在 Factory 页勾选 **自动加载额度增强**，每个浏览器只需启用一次；后续刷新 Manager 会自动加载，登录成功后恢复额度功能，无需每次打开插件。需要 HTTPS 或本机地址，作用域仅为 `/management.html`，不缓存管理页面或保存额外密钥，也不修改 API 请求。关闭同一开关即可取消，插件被移除时自动回退原管理页面。已有其他 Service Worker 接管管理页时不会覆盖它。未启用自动加载时仍使用原来的手动激活方式。

v0.1.4 起，卡片同时显示当前窗口已用费用 / Token 和可用的容量预测；详情抽屉提供前期、当前窗口已用、当前窗口预测三列。费用和请求数据直接读取 Manager 窗口统计接口，复用已有统一价格。窗口边界不完整、无请求样本或存在混合 Core 用量时，不生成虚构数值；未实际观测到上一周期时，比较区间标为“前一等长区间”。具体口径见 [窗口适配规则](docs/integrations/cpa-manager-plus/README.md)。

额度查询不使用定时轮询。进入凭证页或切回 Factory 筛选时，每个当前显示的凭证只检查一次；距上次查询尝试不足 3 分钟则复用缓存。已知的官方周期结束时，会隐藏旧周期的消耗和余额，并在满足 3 分钟间隔后补一次新周期查询；失败或官方尚未更新时，同一周期在此次页面访问中不反复重试。后台标签页不发起该查询，重新可见时再检查。刷新额度、刷新历史和页面刷新按钮仍可立即手动查询。

短期额度快照按管理连接、授权和凭证名称 / `auth_index` 隔离，保存在当前标签页的 `sessionStorage`，使整页重载也遵守 3 分钟间隔；不另存管理密钥，退出或更换连接即清理。查询最多两个并发，失败保留仍有效的上次额度并提示，不沿用旧预测。重置时间在卡片进度条下方右侧显示“n 小时 / 天后”，悬停显示完整重置日期时间；新周期尚未返回时显示等待状态，不推算并冒充官方重置时间。用量使用蓝色图标，预测使用紫色图标，金额与 Token 使用主、次文字色。增强不修改管理端文件，也不伪装凭证类型。

## 开发与构建

需要 Rust 工具链（版本由 `rust-toolchain.toml` 固定）、PowerShell 7、Node.js 24。Windows 还需 MSVC C++ 构建工具；Linux x64 需要原生链接器，CI 构建系统固定为 Ubuntu 22.04。前端为内嵌 HTML / CSS / JavaScript，无需 npm 安装或前端构建服务。

在仓库根目录运行：

```powershell
cargo fmt --check
node --check web/app.js
node --check web/manager-extension.js
node --check web/manager-usage.js
node --check web/manager-refresh.js
node --check web/manager-worker.js
node --check web/manager-bootstrap.js
node --check scripts/release.mjs
node scripts/release.mjs registry
cargo test --locked
./scripts/build.ps1 -Target x86_64-pc-windows-msvc -ReleaseAssets
python -B scripts/verify-cpa.py --library target/x86_64-pc-windows-msvc/release/cpa_factory_droid.dll --host-version 8.0.52
```

Linux x64 在相应系统运行：

```powershell
./scripts/build.ps1 -Target x86_64-unknown-linux-gnu -ReleaseAssets
```

安装包写入 `release/`。构建脚本会从 Cargo 元数据定位 `cpa_factory_droid` 动态库，重命名为 CPA 插件文件名后打包，并检查根目录动态库、文档链接和校验和。ZIP 外的同名 `.build.json` 记录源码指纹、构建环境及产物信息，`.sha256` / `checksums.txt` 提供 ZIP 校验；ZIP 内 `SHA256SUMS.txt` 校验动态库。指定 `-Profile debug` 可生成带 `_debug` 后缀的本地包；它不能与 `-ReleaseAssets` 同时使用。

常规 CI 检查 Windows x64 / Linux x64；发布工作流构建 Windows x64、Linux x64、macOS Intel、macOS Apple Silicon 四个平台。每个平台均测试 Rust、打包并通过固定 CPA v8.0.52 的真实宿主执行模拟上游回归，v8.0.47 不再作为构建验收目标；宿主下载包验证固定 SHA256，并核对实际 HTTP 返回的内核版本，不读取本机真实凭据。Linux 另校验 GLIBC 2.35 基线。

发布由 `v*` 标签触发，也可手动选择已有标签重试；标签、Cargo 和注册表版本必须一致。全部平台完成后，工作流先创建草稿，上传四个 ZIP、`checksums.txt` 和 `latest.json`，核对完整性后公开。macOS runner 使用当前可用的 `macos-15-intel` 和 `macos-15`。本地构建不需要创建或推送标签。

实现结构与限制见 [架构说明](docs/architecture.md)，选型依据见 [调研记录](docs/research.md)。CPAMP 的历史统计、独立价格表和认证差异见 [Manager Plus 兼容说明](docs/manager-plus.md)。

## 许可证

[MIT](LICENSE)
