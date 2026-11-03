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

CPA 官方管理端已支持通用插件额度。CPA-Manager-Plus v1.14.10 原版只对固定 provider 显示刷新按钮，本仓库提供了可复现的前端适配补丁，本机 Manager 已安装该补丁。
适配读取宿主声明的插件能力，在原生凭证卡片和抽屉中提供“刷新额度”，不伪装成 Devin 或 Codex。

详见 [Manager 适配](docs/integrations/cpa-manager-plus/README.md)、[模型目录](docs/models.md) 与 [原生统计](docs/usage.md)。

## 运行要求

- CPA v8.0.52，C ABI 1 / RPC schema 10，需要带 CGO 原生插件支持的构建。
- Windows x64 需要 Microsoft Visual C++ v14 Redistributable；Linux x64 构建基线为 Ubuntu 22.04 / GLIBC 2.35。
- 可登录的 Factory 账号或 API Key（`fk-` 开头），以及可访问 Factory 的 CPA 出站网络；OAuth 还需要访问 `api.workos.com`。
- 官方 Droid CLI 是独立依赖，不随插件分发。插件只用 CLI 查询模型目录，不以 CLI 执行推理。

## 本地安装（Windows x64 已实装验证）

1. 使用本地 `release/` 中对应系统的 ZIP，或按下文从源码构建。Linux x64 使用相应产物，运行要求见上节。
2. 解压，将 ZIP 根目录的 `factory-droid.dll`（Windows）或 `factory-droid.so`（Linux）放入 CPA 的 `plugins` 目录。不要直接复制 Cargo 产物的 crate 文件名。
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

当前工作区变化及本地更新尚未发布为新的 GitHub Release。验证范围见 [验证记录](docs/validation.md)。

## 开发与构建

需要 Rust 工具链（版本由 `rust-toolchain.toml` 固定）、PowerShell 7、Node.js 24。Windows 还需 MSVC C++ 构建工具；Linux x64 需要原生链接器，CI 构建系统固定为 Ubuntu 22.04。前端为内嵌 HTML / CSS / JavaScript，无需 npm 安装或前端构建服务。

在仓库根目录运行：

```powershell
cargo fmt --check
node --check web/app.js
node --check scripts/release.mjs
node scripts/release.mjs registry
cargo test --locked
./scripts/build.ps1 -Target x86_64-pc-windows-msvc -ReleaseAssets
```

Linux x64 在相应系统运行：

```powershell
./scripts/build.ps1 -Target x86_64-unknown-linux-gnu -ReleaseAssets
```

安装包写入 `release/`。构建脚本会从 Cargo 元数据定位 `cpa_factory_droid` 动态库，重命名为 CPA 插件文件名后打包，并检查根目录动态库、文档链接和校验和。ZIP 外的同名 `.build.json` 记录源码指纹、构建环境及产物信息，`.sha256` / `checksums.txt` 提供 ZIP 校验；ZIP 内 `SHA256SUMS.txt` 校验动态库。指定 `-Profile debug` 可生成带 `_debug` 后缀的本地包；它不能与 `-ReleaseAssets` 同时使用。

CI 在 Windows x64 / Linux x64 检查格式、测试和打包，并校验 Linux 产物的 GLIBC 导入基线。本地 Linux 构建也可运行 `node scripts/check-linux-runtime.mjs target/x86_64-unknown-linux-gnu/release/libcpa_factory_droid.so 2.35`，需要系统安装 `readelf`。发布工作流仅响应 `v*` 标签，并校验标签、Cargo 版本和 `registry.json` 一致；它还会生成 ZIP 校验和、发布说明和 `latest.json`。推送标签会触发发布，因此本地构建无需创建或推送标签。

实现结构与限制见 [架构说明](docs/architecture.md)，选型依据见 [调研记录](docs/research.md)。CPAMP 的历史统计、独立价格表和认证差异见 [Manager Plus 兼容说明](docs/manager-plus.md)。

## 许可证

[MIT](LICENSE)
