# 工作区管理与会话多状态 · 设计

日期：2026-11-16
范围：`neverwork_core`（模型/设置）、`neverwork`（app：状态与视图）

## 背景

1. 工作区切换只有命令面板里的手输路径，无最近列表、收藏、重命名；且 `switch_workspace()` 只重建文件树，**不重载会话**——切工作区后列表仍是旧项目的会话（会话本身已按工作区隔离存储于 `~/.neverwork/sessions/<slug>-<hash>.json`）。
2. 会话状态 `SessionStatus` 已有 Idle/Working/Done/Dropped，但 Dropped 无任何入口可设置，Done 也由系统自动打上；缺少「待执行」。
3. 会话列表只有选中、分类循环、置顶；无搜索、状态筛选、删除、归档、批量清理，列表变长后不可用。

## 一、工作区管理器

### 数据（core/settings.rs）

```rust
pub struct RecentWorkspace {
    pub path: String,      // 绝对路径，兼作键
    pub name: String,      // 用户重命名；空则显示目录名
    pub pinned: bool,      // 收藏
    pub opened_at: u64,    // 最近打开（Unix 秒）
}
// Settings 新增：
#[serde(default)] pub workspaces: Vec<RecentWorkspace>,
```

- `record_workspace(path, now)`：同路径 upsert（更新 opened_at，保留 name/pinned）；上限 12 条，淘汰最久未打开且未收藏的。
- `last_workspace` 保留：决定启动根，旧配置兼容。
- `display_name()`：name 为空回退为目录名，再回退完整路径。

### 切换行为（正确性修复）

`enter_workspace(path)` 统一两条旧入口（`switch_workspace` / `apply_workspace_input`）：

1. 生成中（`streaming`）拒绝切换并提示；
2. `save_sessions()` 存旧工作区；
3. 换 root / 文件树 / workspace / `settings.last_workspace` / `record_workspace` / 保存设置；
4. `sessions::load_with_recovery(新 root)` → 空则建 blank → 重算 `relative_label` → 恢复 `active_id`（抽 `adopt_session_store`，与启动加载共用）；
5. 清空搜索/筛选/菜单状态，通知。

### UI

- 现有「工作区细条」点击 → 居中遮罩卡片「切换工作区」（与命令面板同款浮层，不做锚定 popup）。
- 卡片：搜索/最近列表，行 = 📁 名称 + 路径（淡）；收藏在前，再按 opened_at 倒序；路径已不存在标「不存在」。
- 行操作：✎ 重命名（复用 `ws_input` + 行内确认）/ ⭐ 收藏切换 / ✕ 从列表移除（不删目录）。
- 底部「＋ 打开文件夹…」→ 打开现有手输路径行。
- `Command::SwitchWorkspace` 打开同一浮层。

## 二、会话状态

### 枚举（core/model.rs）

`SessionStatus` 增加 `Todo`（待执行），顺序：Idle / Todo / Working / Done / Dropped。

显示（胶囊）：○ 待执行（琥珀）/ 工作中（绿，现有）/ 已完成（蓝，现有）/ 已废弃（灰，现有）/ 无标记不显示。

### 流转

- 发消息 → `Working`（现有）；一轮结束 → `Done`（现有，保留）。
- 手动标记：待执行 / 已完成 / 已废弃 / 清除（→ Idle）。
- `Working` 中标记入口禁用（生成中状态以运行态为准）。
- 待执行发消息自动转 Working（发消息路径已统一置 Working，天然覆盖）。
- 崩溃恢复：Working → Idle（现有 `normalize_after_crash`）。

### 入口

- 卡片悬浮 `⋯`（foot 行 pin 左侧 16px）→ **卡片下方内联展开**操作面板（与团队插件面板同形态，规避锚定 popup 在滚动列表中的定位问题）：
  - 状态行：○ 待执行 / ✓ 已完成 / ✗ 已废弃 / 清除
  - 操作行：置顶 / 分类 / 重命名 / 导出 / 删除（删除需再点一次确认）
- 命令面板：`状态：待执行/已完成/已废弃/清除` 作用于当前会话。

## 三、列表操作

- **搜索**：标题 + 摘要 + 副标题（不扫全文）；`InputTarget::SessionSearch`，Esc 清空。
- **状态筛选**：chips 一行（全部 / 待执行 / 已完成 / 已废弃，只显示有会话的）；分类 chips 保留。
- **归档组**：`is_archived = Done | Dropped`，且非置顶的会话收进列表底部「已归档 N」折叠组，默认收起；搜索或状态筛选激活时自动展开。
- **批量清理**：归档组头「清理」→ 二次确认 → 删除 Done/Dropped（保留置顶）；active 会话按 id 修正。
- **删除单条**：内联二次确认；删除后 active 索引修正，删空补 blank。
- **重命名**：`rename_buf` 行内输入（Enter/按钮提交，Esc 取消）。
- **导出 Markdown**：`~/.neverwork/exports/<安全标题>-<id>.md`，内容含元信息 + 逐条消息（工具调用压成一行摘要），notice 显示路径。
- 排序不变：置顶优先 → `updated_at` 倒序。

## 验证

- core 单测：Todo 序列化 round-trip、旧 settings/session JSON 兼容、`record_workspace` upsert/上限淘汰、`display_name` 回退、过滤/归档纯函数。
- 构建 + `cargo test --workspace` 全量回归（基线 461 passed / 2 ignored）。
- 截图 env：`NW_SESSION_MENU`、`NW_SESSION_QUERY`、`NW_SESSION_STATUS`、`NW_WORKSPACE_POPUP`、`NW_ARCHIVE_OPEN`。
