# 团队会话「无 @ 不派发全员」设计

日期：2026-11-17
状态：已实施
适用版本：NeverWork（`NeverWork/` 工作区）

## 1. 背景与问题

默认协作团队（Planner / Coder / Reviewer）在团队会话里只发一句「你好」，
三个角色依次全部回答。原因是团队 + 开工具（默认）时 `send_text` 在
**没有 @ 点名**的情况下走 `spawn_agents_chain(..., None, ...)`，按 `roles`
顺序把消息派发给全员，完全忽略「编排模式」（流水线 / 并行 / 主管指派）。
只有 `@角色名` 才会进入「入口角色 + handoff 按需接力」的工作流。

用户期望：没有 @ 时**不把消息派发给对应人员**——由首角色先接手，
简单问候只答一次；需要协作时再由它 handoff 给队友。

## 2. 行为定义

| 场景 | 改后行为 |
|---|---|
| 团队 + 开工具 + 无 @ | 首角色（`roles[0]`，默认 Planner）作为入口，带 handoff 工具；需要时由它自动交接 |
| 团队 + 开工具 + @ 点名 | 点名角色起步 + handoff 接力（不变） |
| 团队 + 关工具 + 无 @ | 只跑首角色（纯文本），不派发给全员 |
| 团队 + 关工具 + @ 点名 | 只跑点名角色（不变） |
| 单助手会话 | 不变 |

- 「首角色」= 当前团队角色列表的第一个；用户可通过调整角色顺序改变入口。
- 无 @ 时不再触达其余角色，除非首角色主动调用 `handoff`（仅开工具时可用）。
- handoff 校验、6 跳上限、写操作审批、停止逻辑全部不变。

## 3. 移除「编排模式」设置

`FlowMode` 原本只服务于「无 @ 时怎么轮流发言」这条被去掉的路径，
保留 UI 会出现「切了没反应」的假设置：

- 团队页删除流水线 / 并行 / 主管指派三个 chip，以及头部「编排：xxx」文案，
  改为固定说明「协作：@ 点名或首角色先行」。
- `FlowMode` 字段保留在 `Team` 结构里（旧 `config.json` 照常反序列化），
  `agent` 侧 `run_team_with_history` 的纯文本编排能力保留（测试仍覆盖），
  只是 app 不再调用。
- 对话顶部状态条的「团队流水线」标签改为「协作团队」——它只是按角色
  高亮的状态条，不是编排模式。

## 4. 技术改动

### app（`NeverWork/crates/app`）

**`src/state.rs`**

- 新增纯函数：

  ```rust
  /// 团队会话的入口角色：@ 点名优先，否则首角色（0）。
  pub(crate) fn team_entry_index(text: &str, roles: &[Role]) -> usize {
      neverwork_core::parse_role_mention(text, roles).unwrap_or(0)
  }
  ```

- `send_text` 团队分支重写：
  - 开工具：`roles` 为空时补默认 agent；入口 `entry = team_entry_index(&text, &roles)`；
    统一走 `spawn_agents_chain(..., Some((team, entry)), ...)`。
  - 关工具：取 `@点名` 角色或 `roles[0]`，组成单角色 `Vec`（空团队回退默认
    agent），走 `spawn_agents_chain(..., None, ...)`；`with_tools` 仍为 false。
- 删除私有方法 `spawn_team`（改动后无调用点）。
- 更新「三种走法」与 `spawn_agents_chain` 内「老路径全员」的过时注释。

**`src/views/team.rs`**

- 删除 `flow_chip`、`set_flow`、三 chip 区块；头部文案改为「协作：@ 点名或首角色先行」。

**`src/views/chat.rs`**

- 状态条标签「团队流水线」改为「协作团队」。

### 不动的部分

- `agent/src/team.rs`、`agent/src/supervisor.rs`、`agent/src/agent_loop.rs` 不改。
- handoff 全链路（校验 / 限跳 / 卡片 / 历史回放）不改。
- 单助手会话、关工具 @ 点名行为不改。

## 5. 测试

**单测**

- `team_entry_index`：@ 命中返回对应下标；无 @ 返回 0；空角色返回 0。

**回归**

- `cargo test --workspace` 全绿（app / agent / core 现有用例不回归）。

**真机验收**

- `NW_TEAM` + `NW_SEND=你好`：只有一个角色（Planner）气泡回答，其余角色不出场。
- `NW_SEND=@Coder 你好`：只有 Coder 回答。
- 发送一个需要读写文件的任务：首角色正常调用工具；需要协作时可 handoff。
- 团队页：编排 chips 已移除，头部显示「协作：@ 点名或首角色先行」。
- 对话顶部状态条显示「协作团队」。

## 6. 风险与取舍

- **入口固定为首角色**：用户想让别的角色先上时用 `@点名`；调整角色顺序也可改变默认入口。
- **删掉 flow UI 后旧配置里的 flow 不再生效**：字段保留避免迁移问题；agent 侧能力保留，未来若要「显式全员协作」可再设计入口。
- **关工具无法自动交接**：首角色只能单次发言，符合「不派发」的语义。

## 7. 验收标准

- 团队会话问「你好」，只有一个角色回答。
- `@` 点名的行为与改动前一致。
- 团队页不再出现编排模式切换，且没有不生效的开关。
- `cargo test --workspace` 通过。
