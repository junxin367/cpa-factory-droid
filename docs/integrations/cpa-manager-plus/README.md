# CPA-Manager-Plus 通用插件额度适配

适用版本：`seakee/CPA-Manager-Plus v1.14.10`，提交 `a67ed8834a914ab24c36d2490fa6bb6d4166724c`。
`plugin-quota.patch` 是对该版本的前端补丁，包含新增模块，未修改 Manager Server 和 CPA 内核。

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
