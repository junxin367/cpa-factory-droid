# v0.1.2 宿主兼容性验证

日期：2026-11-04。

本文保留 v0.1.2 发布时的两版本验证记录。后续 CI 与发布构建仅要求 CPA v8.0.52；v0.1.3 验证脚本扩展为 71 项检查，新增反代连接头兼容、显式升级请求拒绝、上游请求头隔离、页面增强资源及原生额度窗口名称与比例验证。

v0.1.4 扩展为 78 项宿主检查，增加窗口元数据身份匹配、官方起止时间保留、脱敏、前端模块资源和 Service Worker 作用域响应头检查。另使用 Manager v1.14.10 独立实例导入测试请求，验证当前/前期区间、跨文件/跨索引隔离、空统计和统一价格重新计算；不改线上 Manager 数据。

v0.1.5 扩展为 79 项宿主检查，验证进入页面刷新策略模块可加载。前端模拟时钟与存储检查覆盖进入一次、停留不轮询、DOM 重绘、180 秒边界、手动绕过、整页重载和退出清理；预测模块未修改。

v0.1.7 包含 19 项 Rust 测试和 85 项宿主检查。新增三种协议下的调用者版本、beta、客户端和代理头隔离检查：向 CPA 传入 Claude Code beta 及冲突头，验证上游只收到 Factory CLI 默认值；JSON/SSE 均须成功。验证使用本机模拟上游，不执行真实账号付费推理。

## v0.1.8 验证

2026-11-04 的 Claude Code 兼容修复增加请求功能与 beta 配套、上游原始错误正文保留，以及 Messages 本地 token 估算。验证使用实际 CPA v8.0.52 和本机严格模拟上游，未修改用户运行中的 CPA，也未执行真实账号推理。

宿主及前端模块检查扩展至 114 项，覆盖完整工具历史、签名与功能字段保留，缺少功能标志时拒绝请求，SDK `betas` 不透传，计数入口鉴权和不产生推理调用，以及分段、超限、HTML、停滞的错误响应。前端增加周期切换、三分钟间隔、失败不重试、旧用量隔离和月额度名称迁移的模拟时钟检查。Rust 测试共 25 项，包含计数/生成路径凭证不可混用及不可重放。

浏览器使用实际增强脚本和模拟管理接口验证了卡片渲染：用量图标为蓝色、预测图标为紫色，金额与 Token 分层显示；模拟周期跨越后只新增一次额度查询，旧用量被新周期统计替换，普通页面重绘不增加查询，月窗口显示“月额度”。

原始错误展示按宿主实际行为验收：Claude 保留上游类型与消息原文，Chat 与非流式 Responses 保留错误 JSON；Responses SSE 仍受 CPA 自身脱敏逻辑影响，已作为明确边界断言。此次验证不能替代同事原始失败请求的线上复测。

## 原因与修复

线上 CPA v8.0.47 在安装 v0.1.1 后记录 `plugin schema version 10 is not supported`。该宿主最高支持 Schema 8；插件把验收宿主的最高版本 10 当作了自身协议需求，导致动态库虽然已发现，注册却被拒绝，菜单及 OAuth 能力无法提供。

插件实际需要的最新能力是 Schema 7 的 `request_interceptor_omit_body_before/after`。当前不使用 Schema 8 的原始请求字节补丁、Schema 9 的多提供商模型目录或 Schema 10 的多授权入口，因此声明最低协议 7，并对宿主显式提供的不兼容协议进行初始化前检查。

旧验证只固定在 CPA v8.0.52 上，缺少线上内核版本的注册门禁。v0.1.2 发布时，常规 CI 与四平台发布均同时验证 v8.0.47、v8.0.52，固定下载包 SHA256，并核对 `X-CPA-VERSION` 返回的实际版本。

## 可复现检查

```powershell
cargo fmt --check
cargo test --locked
./scripts/build.ps1 -Target x86_64-pc-windows-msvc -ReleaseAssets
python -B scripts/verify-cpa.py --library target/x86_64-pc-windows-msvc/release/cpa_factory_droid.dll --host-version 8.0.47 --report temp/cpa-validation-8.0.47.json
python -B scripts/verify-cpa.py --library target/x86_64-pc-windows-msvc/release/cpa_factory_droid.dll --host-version 8.0.52 --report temp/cpa-validation-8.0.52.json
```

`--host` 可复用同版本的本机官方宿主，但仍核对实际 HTTP 版本；不能把调用参数当作已运行对应版本的证据。

每个宿主执行 38 项检查：实际版本、注册启用、菜单、OAuth/额度能力、页面资源，以及原有 33 项请求、缓存、模型规则、目录刷新和 OAuth 流程回归。16 项 Rust 测试包含最低协议声明与宿主版本校验。

本机对照结果见 `compatibility-validation.json`；GitHub 四个平台的两版本验收报告保存在发布任务的 `validation-<平台>` 附件。任一宿主的检查失败都阻止该平台发布。

## 验证边界

- 注册成功、菜单可见与真实账号推理权限是不同层级的证据。未自动执行用户此前暂缓的真实 Factory OAuth 登录或付费推理。
- 不宣称所有更旧或未来 CPA 版本均已测试；当前实测边界是 v8.0.47 与 v8.0.52。
- 升级保留 CPA 凭证、模型管理规则和插件 `wire_namespace`，不要求修改线上 CPA 内核。
- 在 v8.0.47 的隔离验证中，原生通用字段 PATCH 清空 `model_aliases` 后曾残留旧运行时别名，重启后恢复；测试复原该夹具时会重新启动隔离宿主。原生别名配置、调用与全部排除均实际验证，但不把本次兼容修复描述为修复了宿主热清空别名行为。
