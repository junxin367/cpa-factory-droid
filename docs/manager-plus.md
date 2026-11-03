# CPA-Manager-Plus 适配

基线：`seakee/CPA-Manager-Plus v1.14.10`，提交 `a67ed8834a914ab24c36d2490fa6bb6d4166724c`。

## 原生凭证与模型

Factory 账号是 CPA 原生认证文件，直接出现在 Manager 的凭证列表。账号模型只发布共享的 `factory/<模型>` ID。
启停、优先级、模型排除及历史统计由 CPA/Manager 负责，插件只保留添加凭证入口。

## 刷新按钮的原因与修复

原版账号页用固定 provider 集合决定是否显示“刷新额度”，也只读取这些 provider 的额度状态。
CPA 已在 Factory 凭据上返回 `supports_quota=true` 与 `quota_provider=factory-droid`，原版 Manager 未消费这些字段，因此没有刷新按钮。

本仓库适配：

1. 读取宿主声明的能力；内置提供商继续走原有逻辑。
2. 优先使用凭据的 `quota_plugin_id`，旧 v0 接口则通过已启用插件元数据解析提供商。
3. 调用原生插件额度接口，将结果接入账号页现有额度状态和窗口展示。
4. 在卡片、批量操作栏和详情抽屉显示刷新按钮；卡片可显示前三个主额度窗口，含标准月额度。
5. 请求结果绑定凭据身份与管理连接，切换连接或发起新请求后丢弃旧结果。

这属于本地前端适配，不代表上游 v1.14.10 已包含修复。补丁及部署步骤见 [适配目录](integrations/cpa-manager-plus/README.md)。

## 统计与价格

Manager 从 CPA 原生 usage 通道采集新请求并使用自己的模型价格表。Factory 插件不再保存第二套统计。
不自动写入或覆盖 Manager 的模型价格表；根据实际使用模型配置价格。

## 授权

Manager 完整模式用独立的 Admin Key 登录，并在服务端保存 CPA Management Key。
内嵌 Factory 插件手工授权时也使用当前 Manager Admin Key。
插件只复用当前来源明确保存的会话，手工输入只保留在页面内存中。
