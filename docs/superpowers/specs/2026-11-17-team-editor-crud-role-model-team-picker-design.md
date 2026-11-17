# 团队编辑器 CRUD / 角色编辑与模型下拉 / 新建会话选团队

状态：已确认，实施中
日期：2026-11-17

## 背景

用户提出三项需求：

1. 团队编辑器需要创建 / 删除 / 编辑团队，并能查看团队详情。
2. 团队里的角色（Agent 助手）可以编辑；切换模型不能再用「换模型」循环 4 个硬编码 id，要从**已配置好的模型**里下拉选择；插件要能安装（市场面板可用性增强）。
3. 新建会话选团队模式时，可以选择使用哪支团队。

已确认口径：

- 新建会话时选**整支团队**（不是角色子集），会话记住团队。
- 团队管理入口：团队页顶部「团队名下拉 + 操作按钮」。
- 角色编辑：卡片内联展开表单。
- 插件：保留现有市场面板，增强来源切换 / 刷新 / 安装状态。

## 数据模型（core，向后兼容）

`Role` 新增：

```rust
/// 绑定的已配置厂商 id；空 = 跟随会话默认。
#[serde(default)]
pub provider_id: String,
```

`Session` 新增：

```rust
/// 创建时选择的团队 id；旧会话为空 → 回落当前 active team。
#[serde(default)]
pub team_id: String,
```

`Role::new` 默认 `provider_id = ""`、`model = ""`（显示「跟随会话默认」）。
所有结构体字面量（测试）补齐字段；旧 JSON 直接可读。

## 团队管理

团队页顶部：

- 团队名 + `▾` 展开内联下拉：列出全部团队（名称 + N 个角色，当前高亮）；点行切换
  `config.active_team_id` 并立即落盘；下拉底部「＋ 新建团队」。
- 右侧「重命名」→ 出现单行输入（`FormField::TeamName`，回车/失焦提交，立即落盘）。
- 「删除」→ 第一次点变「确认删除？」，再点才删。至少保留 1 个团队；删当前团队后
  自动切到第一个；同时清空角色编辑 / 插件 / 模型面板状态。
- 新建团队：唯一 id（时间戳，冲突时递增）、默认名「新团队 N」、一个默认角色；
  建完直接进入重命名输入态。

新增状态：`team_menu_open: bool`、`team_delete_confirm: bool`。
新增函数：`select_team`、`add_team`、`rename_team`（begin_field）、`delete_team`。
页面本身即团队详情：角色卡显示名称 / 类型 / 模型 / 提示词摘要 / 插件。

## 角色编辑 + 模型下拉

角色卡右上「换模型」改为「编辑」，卡内展开表单（与插件面板同一种内联展开，
两者互斥）：

- 名称 / 类型 / 首字母：`FormField::RoleName / RoleKind / RoleInitial`，复用
  `views::settings::input_box`（回车 / 失焦提交）。
- 提示词：新增 `InputTarget::RolePrompt` + 多行 `role_prompt_buf`。点击进入编辑；
  Enter 插入换行（IME 组合中不插）；Esc / 「完成」提交到内存角色；「保存」前也会
  先同步。提交时空提示词保留原文。
- 模型行：显示当前 `厂商 · 模型`；「选择 ▾」展开下拉，数据来自
  `picker_groups("")`（只列 settings 中启用且就绪的厂商及其启用模型），首行
  「跟随会话默认」（清空 `provider_id` + `model`）。没有可用模型时给「去设置」
  按钮跳 `SettingsTab::Models`。
- 删除角色时修正 / 关闭编辑、模型、插件面板下标；至少保留 1 个角色。
- 新角色默认 `model = ""`、`provider_id = ""`（跟随会话）。

团队页「保存」：先提交当前字段与提示词缓冲，再写 `config.json`。

## 运行时模型路由

现状：`run_agent_in_team` 根本不读角色模型，`role_provider` 的启发式也只切
provider 名不换 Key。

改为：

- `neverwork_agent::team::role_provider` 改为 `pub`。
- app 新增 `role_runtime_config(role, session_idx, base_provider, base)`：
  1. `NW_*` 环境变量强制时用会话配置（验收行为不变）；
  2. `role.provider_id` 命中启用且就绪的厂商 → 用该厂商的 key / base_url /
     temperature，模型取 `role.model`（空则厂商默认模型）；
  3. 否则退回 `role_provider`（只有像真实模型 id 的 `role.model` 才覆盖）。
- `spawn_agents_chain` 的 `(provider, config)` 改为与 `roles` 下标对齐的
  `runtimes: Vec<(String, ProviderConfig)>`；每个 hop（含 handoff 下一跳）用自己的
  配置；单助手 / 关工具路径传单元素 vec。
- 团队气泡页脚：按角色名找到团队角色时显示该角色的 `厂商 · 模型`，否则沿用会话
  模型。

## 插件面板增强

角色卡内市场面板：

- 顶部来源 chips：MCP 官方 / 腾讯云广场 / skills.sh，点击走 `set_market_source`
  （无缓存自动拉取）。列表 = 已安装 + 内建 + 当前来源条目（去重）。
- 「刷新」按钮：`market_loading` 显示「刷新中…」；`market_error` 显示错误 +
  「重试」。刷新前清空 `skill_search`，避免 skills.sh 串用设置页旧搜索词。
- 行状态：`market_installing` 命中显示「安装中…」；已安装未绑定显示「已安装」
  徽标 + 「＋ 添加」；已绑定「已添加」；远程托管「仅远程」。
- MCP 确认条、详情抽屉、绑定逻辑保持不变。

## 新建会话选团队

- 新建浮层的团队卡改成团队列表（每行：名称 + N 角色 + 角色 chips），点哪支用哪支
  建会话；单助手卡不变。
- `new_session_with_team(mode, team_id, cx)`；`new_session(mode)` 保持原签名
  （侧栏 + / NW_SEND 走 active team）。创建后 `active_team_id` 同步为所选团队。
- `blank_session` 写入 `team_id` + `team_name`。
- `team_for_session(idx)`：`session.team_id` 命中 → 该团队；为空 / 已删 → active
  team。对话头部团队名、状态条角色链、@ 角色 chips、角色类型标签、发送时的团队
  取用全部改成按会话。
- 删除团队后旧会话下一次发送回落 active team，并更新会话展示的团队名。

## 测试与验收

- 单测：旧 JSON 反序列化默认值；`team_for_session` 回落；`role_runtime_config`
  三分支；团队 id 唯一；现有 handoff / 团队测试保持绿。
- `cargo test --workspace`、`cargo build -p neverwork`。
- 真机（临时 `NW_CONFIG_DIR`，含 Key 的副本用完即删）：团队切换 / 新建 / 重命名 /
  删除确认；角色编辑与模型下拉；插件来源 / 刷新 / 安装状态；新建会话选团队；
  `@Coder` 只由 Coder 回答且用其绑定模型；DPI-aware 截图。
- 新增验收钩子：`NW_TEAM_MENU=1`、`NW_ROLE_EDIT=<idx>`、`NW_ROLE_MODEL=<idx>`
  （配合已有 `NW_PICK_MODE=1`）。

## 涉及文件

`core/src/model.rs`、`app/src/state.rs`、`app/src/settings_ops.rs`、
`app/src/input_handler.rs`、`views/team.rs`、`views/new_session.rs`、
`views/chat.rs`、`agent/src/team.rs` 及测试字面量。
