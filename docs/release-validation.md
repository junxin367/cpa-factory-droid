# v0.1.1 发布与长期回归验证

核验日期：2026-11-04。当前版本的验证入口为本文；`validation.md` 保留此前各阶段历史。

## 修复范围

1. 明确的 Codex 线程 ID 优先于子代理名称；相同根会话、相同代理名称下的不同线程各自隔离。
2. API Key 凭据带有 CPA 定时刷新所需的间隔与时间元数据，目录到期后可自动更新；首次模型加载前也可恢复账号缓存。
3. 模型目录交付与 CPA 最终可见模型分别校验，尊重原生排除、别名和前缀规则；未交付、凭据缺失和无规则时的不完整发布仍报未同步。

## 可复现验证

```powershell
cargo fmt --check
cargo test --locked
node --check web/app.js
node --check scripts/release.mjs
node scripts/release.mjs registry
./scripts/build.ps1 -Target x86_64-pc-windows-msvc -ReleaseAssets
python -B scripts/verify-cpa.py --library target/x86_64-pc-windows-msvc/release/cpa_factory_droid.dll --report temp/cpa-validation.json
```

验证器默认下载固定版本 CPA v8.0.52，并核对对应平台的固定包 SHA256。也可用 `--host` 指向已核验的同版本本机宿主。测试创建独立回环端口、临时凭据、插件数据目录和模拟上游，不访问真实 Factory，也不改变现有 CPA 配置。结束时停止测试宿主并清理本次创建的隔离目录。

14 项 Rust 测试覆盖价格兼容、元数据调度和匿名标识边界。33 项宿主检查覆盖：

- 原生凭据创建、模型发布与健康状态。
- 会话、缓存、安全标识分别映射，调用方隔离，同名代理多线程与 16 会话并发。
- 三类原生非流式和 SSE、原生续接字段与缓存断点、代理包装清理。
- 重启后的会话/缓存稳定性。
- 原生排除、别名、前缀、全部模型排除、目录同步与实际别名调用。
- API Key 目录到期后的自动刷新，模拟 OAuth 登录、到期轮换与令牌持久化。

本机修复前后对照数据见 `release-validation.json`。GitHub 普通 CI 在 Windows/Linux 运行，发布任务在四个平台分别构建和运行同一验证器，原始 JSON 作为 `validation-<平台>` 工作流附件保存。

## 构建与发布

参考 `cpa-apikey-manager` 与 `cpa-window-primer` 的 GitHub 配置：四平台矩阵、固定提交构建、标签与版本一致性校验、统一命名 ZIP、校验和及 CPA 更新清单。macOS 改用当前可用的 `macos-15-intel` 和 `macos-15`。

四个平台全部成功后，发布任务上传完整资产到草稿，再公开 Release；任一构建或宿主检查失败都不会首次公开该版本。正式发布状态以 GitHub 工作流及 Release 资产为准，不能以本机测试或标签已推送代替。

更新清单中的发布页和下载链接按已校验的正式标签构造，不能直接保留 GitHub 草稿 API 返回的 `untagged-*` 地址。公开后另下载四个 ZIP，核对清单、ZIP/动态库 SHA256、对应宿主检查报告以及本机实装库的一致性。

## 验证边界

- 真实 Factory OAuth 授权、真实推理和缓存命中率继续保持未验收，未代替用户完成登录。
- 全局别名的完整 `fork` 语义不在 ABI 10 摘要中，插件沿用 CPA 最终注册结果；不会另建一套路由配置。
- Manager 补丁针对 v1.14.10，升级 Manager 后需重新核对补丁兼容性。
