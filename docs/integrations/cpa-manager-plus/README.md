# CPA-Manager-Plus 通用插件额度适配

## v0.1.3 起：通过插件页激活

更新 Factory Droid 后，在 Manager 侧边栏打开一次 **Factory Droid** 并连接当前管理端，看到“原生额度已启用”后返回凭证页，即可在卡片、列表和详情抽屉中刷新官方标准额度。脚本和样式随插件动态库发布，不需要单独部署 `management.html` 或安装 JS Handler。

前提是插件 iframe 与 Manager 同源，且允许访问父页面。脚本加载到父页面后持续工作，普通切页不会失效；浏览器整页刷新、退出登录或切换管理连接后，重新进入插件页激活。未保存管理密码时在插件页手工连接即可，不要求保存密码。

展示限定为 Factory 官方标准 5 小时、7 天和月额度。CPA / Manager 原有总体用量继续使用自身数据，内置额度风险筛选、批量额度刷新和窗口预测没有被改写。额度结果只保存在当前页面内存中，按凭证和管理连接隔离，不把令牌写入新存储项。失败保留上次结果并提示重试。

## 历史源码补丁（可选）

适用版本：`seakee/CPA-Manager-Plus v1.14.10`，提交 `a67ed8834a914ab24c36d2490fa6bb6d4166724c`。
以下 `plugin-quota.patch` 是先前独立维护前端时使用的方案，包含新增模块，未修改 Manager Server 和 CPA 内核。新插件的页面增强不依赖此补丁，用户无需执行以下构建与安装步骤。

## 修复范围

原版账号页只为固定提供商显示额度刷新控件。本补丁读取原生凭据的 `supports_quota`、`quota_provider` 和可选的 `quota_plugin_id`，通过 CPA 原生插件额度接口刷新。

卡片、批量栏与详情抽屉共用原生刷新流程；主卡片显示第一个额度组的前三个窗口，详情可查看全部窗口。
额度请求和缓存绑定当前凭据及连接，未知值不会当作零。
插件返回的组名不能证明其对应的完整模型集合，因此不把整账号请求量冒充某个额度池的用量预测。

## 构建与安装

在干净的对应上游源码中执行：

```powershell
git apply --check E:/code/junes/github/cpa-factory-droid/docs/integrations/cpa-manager-plus/plugin-quota.patch
git apply E:/code/junes/github/cpa-factory-droid/docs/integrations/cpa-manager-plus/plugin-quota.patch
npm ci
npm run type-check
npm run build
```

将 `apps/web/dist/index.html` 复制到 Manager 安装目录的 `management.html`，备份并在 `config.json` 中合并：

```json
{
  "panelPath": "./management.html"
}
```

重启 Manager，再刷新浏览器页面。本机安装目录为 `E:/code/junes/github/cpa-manager-plus-local`，入口为 `http://127.0.0.1:18317/management.html`。
升级 Manager 时应重新核对补丁，不直接用旧 HTML 覆盖不同版本的页面。

## 验证

类型检查和生产构建通过；7 项临时回归覆盖能力判断、零/未知额度、标准窗口、v0 插件解析、v8 明确插件 ID、身份缺失拒绝，以及切页缓存和连接切换隔离。
临时测试文件在验证后移除，不包含在补丁内。
