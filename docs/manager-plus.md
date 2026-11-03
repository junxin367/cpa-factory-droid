# CPA-Manager-Plus 兼容说明

源码核验基线为 `seakee/CPA-Manager-Plus v1.14.10`，提交 `a67ed8834a914ab24c36d2490fa6bb6d4166724c`，核验日期为 2026-11-03。Manager Server / 管理面板版本与 CPA 内核版本分别计算；本插件的 CPA 基线仍为 v8.0.52。

## 账号卡片与历史统计

CPAMP 根据实际 auth 文件动态生成 provider 筛选，因此 `factory-droid` 可以出现在账号页。插件将每个账号保存为独立 CPA 凭据，保持文件名、provider 和账号 ID 稳定；Manager 可按宿主 `auth_index` 关联请求快照。

零模型账号也可以保存并出现在凭据页；这不代表已有可调用模型。候选目录、账号基础测试/允许与 CPA 实际发布在插件页分别管理，操作见 [账号与模型管理](models.md)。

在 Full Mode 历史采集正常时，现有卡片可以复用请求数、token、成功率和最近请求。Manager 最近请求展示最多 10 条；本插件页面的本地最近记录另有自己的范围，两者可能因采集时点和保留范围而不同。Manager 尚未采集或索引尚未匹配时，不能凭空补全历史。

本插件卡片顶部四指标使用同一配置报表窗口（默认 UTC 自然月）并包含验证；不是全历史。下方额度行分别使用各官方窗口，最近请求条只是报表范围内的最近子集。不要与 Manager 的历史总量或官方账单混称。

可在 CPAMP 账号页选择 `factory-droid`；对应页面路径为：

```text
/management.html#/accounts?provider=factory-droid&layout=grid
```

该路径是 CPAMP 页面，不是 CPA 官方管理前端的通用路由。

## 客户端协议选择

账号出现在 Manager 中，不表示客户端可以任意选择 OpenAI 兼容入口。Factory 模型按显式配置的 `protocol` 调用：`anthropic` 使用 `/v1/messages`，`responses` 使用 `/v1/responses`，`chat_completions` 使用 `/v1/chat/completions`。错配返回 400 `protocol_mismatch`，不向 Factory 发送请求；宿主的已知早期兼容修正边界见 [原生协议与调用限制](protocol.md)。

共享模型可由 CPA 调度，固定账号模型 ID 用于用户主动选择账号，两者遵循相同的原生协议规则。固定账号是可选能力，不是多账号工具调用的强制步骤。客户端需自行按原生协议保存完整工具历史。

## 官方额度为什么在插件页面

v1.14.10 的额度类型、stores、刷新 switch 和 `resolveAccountQuota` 只接固定 provider；未知 provider 返回空额度，也未消费宿主的通用 plugin quota 元数据。因而 Factory 实现 `quota_provider`，仍不会自动进入该版本原生账号额度卡片。

Factory 官方滚动窗口、额外 USD 余额、Analytics FSC 和本地样本容量估算在 Factory Droid 插件页面展示。不能通过伪装成既有 provider、随意增加 auth 字段或修改父页面 DOM 来补齐通用额度支持。若要让原生卡片显示，需要在 Manager 中实现 generic plugin quota 及对应窗口语义；本仓库没有修改或部署 Manager。

## 管理认证与主题

同源插件页面复用当前适用的 `cli-proxy-auth` 登录记录，兼容 v1 / v2 编码；没有已保存会话时提供手工输入：

| 打开方式 | 使用的管理密钥 |
| --- | --- |
| 直接连接 CPA | CPA 管理 Key |
| CPAMP Full Mode 的同源插件页 | 当前 CPAMP Admin Key |

Full Mode 由 Manager 验证 Admin Key，再用服务端持有的 CPA 管理 Key 反代。浏览器无需获得服务端真实 CPA Key；直接把内核 Key 提交给 Manager 保留的 `/v0/management/plugins/*` 前缀可能被拒绝。

插件管理请求使用 `/v0/management/plugins/factory-droid/*`。v1.14.10 没有通用 `/v8/management/*` 反代，不能仅照抄 CPA 官方管理前端的 v8 URL。

插件样式兼容 Manager 同源主题属性，并在独立打开或主题桥不可用时保留自身样式。认证或主题跨来源时不假定父页面会自动注入 token。

## 费用是独立估算

Manager 卡片的费用按 **Manager 自己的模型价格表**计算。插件的全局及账号独立输入、输出、缓存读取、缓存写入价格仅用于插件本地估算，不会同步到 Manager。账号别名只改账号专属 ID，不创建全局别名；使用它时也要核对 Manager 实际账单模型键。模型注册成功不等于价格已注册。

如需 Manager 卡片费用，在其现有模型价格页检查实际账单模型 ID 与价格键，确认 `factory/` 别名、resolved / billing model 的对应关系。单价单位为 USD / 百万 token。缺价格时看到 `$0` 或 `-` 不能证明 Factory 免费，也不代表官方 FSC、赠送额度或套餐账单。

不要使用只含 Factory 条目的 `PUT /v0/management/model-prices` 覆盖已有价格表；该接口替换整本价格表。本插件不会自动写入该表。

## usage 采集边界

Manager Full Mode 的采集器消费 CPA `usage-queue` 并写入自己的数据库。该队列读取是 pop 操作，不是历史查询；额外消费者会抢走事件。Factory 插件使用宿主 `usage.handle` 回调记账，不轮询生产队列。需要查看 Manager 历史时，使用其现有账号页或 monitoring 历史接口。

请求统计依赖 CPA 正确发布 usage、Manager 采集正常和稳定 auth 索引。Factory executor 返回对应原生协议的 usage；三类协议的输入与缓存口径须分别核对，不能把 Anthropic 的原始 input 当作已经包含缓存。本机验收范围见验证记录，真实 Factory 上游缺失 usage 仍按未知处理。

显式基础测试直接写插件账本为 validation，不伪造或重复发布业务 usage；因此不能预期 Manager 的业务历史会自动包含插件验证明细。账号卡片按周期展示已记录 M / USD 与预测总量，隐藏 Droid Core 5 小时行；后端 `/usage?kind=validation` 和 `/settings` 继续提供验证统计与预算 API，不对应独立页面 Tab。partial 覆盖与 v2 预测口径见 [用量说明](usage.md)。

## 对照源码

- [账号 provider 与 quota 汇总](https://github.com/seakee/CPA-Manager-Plus/blob/v1.14.10/apps/web/src/features/accounts/model/accountQuotaSummary.ts)
- [账号历史关联](https://github.com/seakee/CPA-Manager-Plus/blob/v1.14.10/apps/web/src/features/accounts/model/accountHistoryRows.ts)
- [插件元数据归一化](https://github.com/seakee/CPA-Manager-Plus/blob/v1.14.10/apps/web/src/services/api/plugins.ts)
- [管理路由反代](https://github.com/seakee/CPA-Manager-Plus/blob/v1.14.10/apps/manager-server/internal/service/proxy/service.go)
- [模型价格服务](https://github.com/seakee/CPA-Manager-Plus/blob/v1.14.10/apps/manager-server/internal/service/modelprice/service.go)
- [HTTP usage queue 采集](https://github.com/seakee/CPA-Manager-Plus/blob/v1.14.10/apps/manager-server/internal/httpqueue/client.go)
