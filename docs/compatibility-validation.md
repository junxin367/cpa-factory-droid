# v0.1.2 宿主兼容性验证

日期：2026-11-04。

本文保留 v0.1.2 发布时的两版本验证记录。后续 CI 与发布构建仅要求 CPA v8.0.52；v0.1.3 验证脚本扩展为 71 项检查，新增反代连接头兼容、显式升级请求拒绝、上游请求头隔离、页面增强资源及原生额度窗口名称与比例验证。

v0.1.4 扩展为 78 项宿主检查，增加窗口元数据身份匹配、官方起止时间保留、脱敏、前端模块资源和 Service Worker 作用域响应头检查。另使用 Manager v1.14.10 独立实例导入测试请求，验证当前/前期区间、跨文件/跨索引隔离、空统计和统一价格重新计算；不改线上 Manager 数据。

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
