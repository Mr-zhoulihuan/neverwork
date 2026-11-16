# 团队模式 @角色 工作流（handoff）设计

日期：2026-11-16
状态：已确认，待实施
适用版本：NeverWork（`NeverWork/` 工作区）

## 1. 背景与问题

用户要在团队模式里用一句话驱动多角色协作：

```
@需求验证 麻烦你帮我验证下需求 是否需要修改 如果需要修改 就通知@接口成员去按需求进行修改
```

当前代码能做到的和做不到的：

| 能力 | 现状 |
|---|---|
| 角色/团队模型 | 有（`Role` / `Team` / `FlowMode`） |
| 团队 + 开工具 | 有，但 `spawn_agents_chain` **按 roles 顺序全部跑一遍**，不看 `flow` |
| 团队 + 关工具 | 有，`Pipeline` / `Parallel` / `Supervisor`；Supervisor 由 Lead 决策 |
| `@` 语义 | 只有「@ 引用文件」，发送时**不解析角色** |
| 角色间交接 | `MessageKind::Handoff` 有模型和渲染，但**全仓没有生产者**（死代码） |
| 角色上下文 | 仅靠 `[角色名] 正文` 拼进 history，没有结构化「结论 + 指令 + 目标」 |

结论：需要补三件事——入口路由（@ 只启动被点名角色）、结构化交接（角色调用工具把活交给另一个角色）、可追溯呈现（Handoff 卡片 + 独立气泡）。

## 2. 目标与非目标

### 目标

1. `@角色名` 作为本轮入口，只启动该角色；其余角色待命。
2. 角色通过 `handoff` 工具把控制权交给另一个角色，携带结构化修改点，运行时**自动接力**。
3. 每次交接在对话里落一张 Handoff 卡片，角色输出按气泡呈现，可追溯到「谁因为什么交给了谁」。
4. 写操作仍然逐个审批；用户可随时停止整条链路。

### 非目标（本次不做）

- 不做 DSL / 可视化流程编排。
- 不支持一轮交接给多个角色、不支持并行交接。
- 不为 Anthropic / Gemini 原生通道做文本兜底（它们当前本身不支持 tools）。
- 不改 Supervisor 的决策逻辑。
- 不改单 Agent（非团队）模式的行为。

## 3. 用户可见行为

### 3.1 入口路由

- 团队模式下，输入框上方显示一行角色 chips（颜色点 + 名字），点击在光标处插入 `@名字 `。
- `parse_role_mention` 匹配规则：
  - `@` 前必须是行首、空白或标点，避免邮箱式误匹配。
  - 角色名后必须是空白、标点或文本结尾；`@需求验证.md` 不算角色提及。
  - 多个角色名前缀重叠时取最长匹配（`接口成员` 胜过 `接口`）。
  - 多处命中时取**最早出现**的一个作为入口；同长同位置按角色列表顺序取第一个。
  - 边界判定中的「标点」包含中英文标点（空白、`，。！？；：`、`,`、`.`、`!`、`?` 等）。
- 命中时输入框上方显示「本轮交给：<角色名>」；未命中按普通文本/文件引用处理，行为不变。
- 一条消息里后续的 `@其他角色` 只是给模型看的正文，不改变入口。

### 3.2 气泡呈现

- 每个角色的输出渲染为独立 `agent_bubble`（现有 `MessageKind::Agent { participant }`）。
- 同一角色连续文本增量进入同一个气泡；工具调用是阶段边界，工具后的正文开新气泡（现有 `RoundBubbles` 行为）。
- Handoff 卡片居中显示在两条角色气泡之间，不占角色气泡。
- 目标角色登场开**新气泡**；同一角色被交回时同样开新气泡（角色本轮结束时 `bubbles.end_round(ri)`）。
- 工具步骤行、审批卡片仍按现有方式穿插在气泡之间。
- 单 Agent 模式不显示角色 chips，行为不变。

### 3.3 交接

角色需要其他角色接手时，**必须调用 `handoff` 工具**；正文里写 `@接口成员` 不会产生任何通知。系统提示会明确这一点。

`handoff` 参数：

| 字段 | 类型 | 说明 |
|---|---|---|
| `to` | string，enum 为团队成员名 | 目标角色 |
| `verdict` | `pass` / `needs_change` | 验证结论 |
| `summary` | string | 一句话结论 |
| `changes[]` | array | `file` / `location` / `issue` / `required` |
| `acceptance` | string | 验收标准 |

规则：

- `verdict = needs_change` 时 `changes` 不能为空。
- `verdict = pass` 表示「验证通过，把结论交给下一个角色」，允许 `changes` 为空；不需要交接时**不要调用** `handoff`，直接输出结论结束。
- `to` 不能是自己。
- `handoff` 必须单独一轮调用；同轮混有其他工具或出现多个 handoff，全部拒绝并回结构化错误。
- 合法交接后：当前角色本轮结束（此前正文保留）→ 落 Handoff 卡片 → 目标角色以交接载荷作为本轮指令启动。

### 3.4 链路约束

- 单次任务最多 6 次交接（不含入口角色的首次运行，即最多 7 个角色轮次），到达上限提示并正常结束。
- 用户点停止：当前角色结束后不再启动下一跳。
- 被交接角色的写操作仍逐个弹审批；拒绝后它按现有逻辑换方案继续。
- 目标角色改完即结束，不自动回检；它认为有歧义时可以主动 `handoff` 回需求验证（计入 6 跳）。

### 3.5 降级

- 团队模式关工具，或 Provider 不支持 tools（Anthropic / Gemini 原生）：
  - `@` 入口仍生效（只跑该角色，纯文本）。
  - 不发送 handoff 工具，不做自动交接。
  - 输入框提示「当前模型不支持工具，角色间不会自动交接」。

## 4. 技术设计

### 4.1 core（`NeverWork/crates/core`）

- `Team::role_by_name(name: &str) -> Option<usize>`：trim 后按 ASCII 忽略大小写匹配，统一角色查找。
- `parse_role_mention(text: &str, roles: &[Role]) -> Option<usize>`：纯函数，边界规则见 3.1，配单元测试。
- `MessageKind::Handoff` 扩展：
  - 现有 `from` / `to` / `action` 保留；`action` 是一行摘要（卡片默认显示）。
  - 新增 `#[serde(default)] detail: String`：完整修改点文本（卡片展开、历史回放）。
  - 旧会话反序列化 `detail` 为空串，兼容。

### 4.2 agent（`NeverWork/crates/agent`）

**tools.rs**

- 常量 `HANDOFF_TOOL = "handoff"`。
- `HandoffPayload` / `HandoffChange`（Serialize + Deserialize）。
- `handoff_spec(roles: &[Role]) -> ToolSpec`：
  - `to` 的 schema 用 `enum` 列出角色名。
  - `verdict` 的 schema 用 `enum ["pass", "needs_change"]`。
  - `requires_approval = false`（不会走 `tools::execute`，由循环拦截）。
- `requires_approval()` 不修改；handoff 在调用它之前被识别。

**agent_loop.rs**

- 新增 `AgentEvent::HandoffRequest { from: String, payload: HandoffPayload }`。
- 新增 `run_agent_in_team(team: &Team, role_idx: usize, ...)`：与 `run_agent` 共用主体，额外做三件事：
  1. 在工具清单末尾追加 `handoff_spec(team.roles)`（仅 `enable_tools && provider.supports_tools()` 时）。
  2. 系统提示追加团队花名册（名字 + 类型 + 职责）与交接规则：正文里的 @ 不算通知；需要接手必须调用 `handoff`；handoff 单独一轮。
  3. 识别并拦截 `handoff` 调用，做校验后 emit 事件并结束本轮。
- `run_agent` 签名保持不变，内部退化为 `team = None`；单 Agent 与现有调用点零改动。
- 校验与处理顺序：
  1. 本次模型回复里出现 `handoff` 且数量 > 1 → 全部回 `BAD_ARGS`（一次只能交接一个角色），回填后继续下一轮模型调用。
  2. `handoff` 与其它工具同轮 → 全部回 `BAD_ARGS`（handoff 必须单独调用），回填后继续下一轮模型调用。
  3. 解析参数失败 / `to` 不存在 / `to == 当前角色` / `needs_change` 但 `changes` 为空 → 回结构化错误 JSON，**不结束本次运行**，模型可改后重试。
  4. 校验通过 → 回填一条 tool 结果（`已交接给 <to>`，保证协议配对）→ emit `HandoffRequest` → trace 记录 `handoff` → 返回，结束当前角色本轮。
- 常量 `MAX_HANDOFF_STEPS = 6`，由 app 侧 runner 使用。
- `supervisor.rs` / `team.rs` 不修改。

### 4.3 app（`NeverWork/crates/app`）

**入口路由（`state.rs::send_text`）**

- 取当前激活团队，调用 `parse_role_mention`。
- 团队模式 + 开工具 + 命中：走新工作流 runner，入口角色 = 命中角色。
- 团队模式 + 关工具 + 命中：走 `spawn_team`，只跑命中角色（传入 `selected_role`，UI 侧用同一份 roles 映射）。
- 无 `@`：保持现状（开工具全员串行 / 关工具按 flow）。

**工作流 runner**

由 `spawn_agents_chain` 演进，线程内状态机：

```
current = 入口角色下标
hops = 0
input = 原始用户文本
loop {
    run_agent_in_team(team, current, history, input, root, with_tools, limits, emit, cancel)
    if cancel { break }
    if 捕获到 HandoffRequest {
        if hops >= MAX_HANDOFF_STEPS { 提示并 break }
        history.push("[当前角色] 本轮正文")
        input = render_handoff(payload)
        current = team.role_by_name(payload.to)  // 找不到则提示并 break
        hops += 1
        continue
    }
    history.push("[当前角色] 本轮正文")
    break   // 入口模式跑完即结束
}
```

- 无 `@` 的老路径仍按 roles 顺序循环，不注入 handoff 工具。
- `emit` 闭包除了收集 `TextDelta`，还要把 `HandoffRequest` 捕获到线程局部变量，供状态机继续。

**UI 事件**

- `AgentEvent::HandoffRequest` → 立即插入 `MessageKind::Handoff { from, to, action, detail }` 并 `save_sessions()`。
- `AgentEvent::Done` 时 `bubbles.end_round(ri)`，保证同一角色再次登场时开新气泡。
- 新事件不改变现有 `ToolStarted` / `NeedApproval` / `ToolFinished` 流程。

**历史回放**

- `history_for_model` 把 `MessageKind::Handoff` 也纳入块收集（用 `m.body` 作为正文）。
- 落 Handoff 卡片时同时写 `body = detail`，于是下一轮追问时模型能看到「上次谁把什么交给了谁、要求了哪些修改」。
- `MessageKind::Handoff` 在历史里按 assistant 正文处理，不伪造工具协议。

### 4.4 UI（`views/chat.rs`）

- 团队模式输入框上方：
  - 角色 chips：点击插入 `@名字 `（复用 `chat.insert`）。
  - 命中提示：「本轮交给：<角色名>」；关工具时追加降级说明。
- Handoff 卡片：
  - 默认一行：`需求验证 → 接口成员 · 需要修改 3 处`。
  - 点击展开 `detail`；展开状态用独立的 `expanded_handoff: HashSet<usize>`，不与 `expanded_steps` 混用。
- 卡片标签沿用现有 `theme` 令牌，不写死颜色。

## 5. 错误处理

| 情况 | 处理 |
|---|---|
| `to` 不在团队 / 等于自己 | 结构化 `BAD_ARGS` 回填，本轮继续，模型可重试 |
| `needs_change` 但 `changes` 为空 | 同上，要求补全修改点 |
| 同轮多个 handoff | 全部拒绝，提示一次只能交接一个角色 |
| handoff 与其他工具同轮 | 全部拒绝，提示必须单独调用 |
| 交接链到 6 跳 | 输入框提示「已达交接上限」，正常收尾，不标 Failed |
| 目标角色找不到（理论上不会） | 提示并结束，不进入下一跳 |
| 用户停止 | 当前角色结束后不再启动下一跳；沿用现有 Cancel 语义 |
| 目标角色写操作被拒 | 现有逻辑：回填拒绝原因，目标角色换方案继续 |
| Provider 报错 | 现有 `Error` 事件；会话标 `Failed`，不自动重试 |
| 关工具 / 不支持 tools | 不发送 handoff，入口仍生效；提示无法自动交接 |

## 6. 测试计划

**core 单测**

- `@需求验证 请…` 命中；`@src/main.rs`、`@a@b.com`、`@需求验证.md` 不命中。
- 前缀重叠取最长；多处命中取最早；大小写不敏感；空角色名跳过。

**agent 单测**

- `handoff_spec` 的 `to.enum` 等于角色名列表，`verdict` enum 正确。
- 合法 handoff：emit `HandoffRequest`、不触发审批、trace 有记录、本轮结束。
- 自交接 / 空 changes / 多 handoff / 混合工具轮：回结构化错误，不 emit 事件。
- `run_agent` 原签名路径不受影响（工具清单里没有 handoff）。

**app 单测**

- 入口选择：命中 / 未命中 / 多个 `@` 取第一个。
- `render_handoff` 文本包含结论、修改点与验收标准。
- `history_for_model` 带回 Handoff 的 `detail`。
- 6 跳上限的纯函数判断。

**集成（`tests/team_flow.rs` 风格）**

- mock SSE 服务器脚本化两轮：
  - 第一轮返回 `tool_calls`（`handoff`），断言请求体 `tools` 含 handoff 且 enum 正确。
  - 第二轮返回正文，断言目标角色收到的指令包含修改点。
- 事件序列：`HandoffRequest → 目标角色 TextDelta`。
- 无 `@` 的老路径请求体不含 handoff。

**手动验收**

- 造一个含「需求验证 / 接口成员」的团队配置，`NW_SEND` 发送本文开头的原句。
- 观察：只启动需求验证 → 读取文件 → Handoff 卡片 → 接口成员新气泡 → 写操作审批 → 最终报告。
- 刷新会话，Handoff 卡片仍在且可展开。

## 7. 实施顺序与改动文件

1. **core**：`model.rs` 增加 `role_by_name`、`parse_role_mention`、Handoff `detail` 字段 + 单测。
2. **agent**：`tools.rs` 增加 handoff spec/payload；`agent_loop.rs` 增加事件、`run_agent_in_team`、校验、trace；`lib.rs` 导出。
3. **app**：`state.rs` 入口路由、工作流 runner、Handoff 卡片、Done 收气泡、历史回放；`views/chat.rs` chips、提示、卡片展开。
4. **测试**：core/agent/app 单测 + 集成；最后跑 `cargo test --workspace`。

## 8. 风险与取舍

- **模型判断不稳定**：「是否需要修改」仍是模型结论。缓解：`verdict` 与 `changes` 强制结构化；修改仍需用户逐个审批；用户可随时停止。
- **交接成本**：每一跳都是完整 Agent 运行，token 成本随跳数上升。缓解：6 跳上限 + 独立交接卡片让用户看清每一步。
- **只支持 tools 通道**：Anthropic / Gemini 原生暂不可用。接受，因为现状本就不支持工具；后续可加文本标记兜底。
- **Handoff 是死代码，本次首次启用**：需同时补生产者与渲染细节；旧会话字段用 `serde(default)` 兼容。
- **不改老路径**：无 `@` 时行为完全不变，降低回归面。

## 9. 验收标准

- 发送 `@需求验证 …` 后，只有需求验证先开口，其余角色不自动运行。
- 需求验证判定需要修改时，出现 Handoff 卡片，接口成员以新气泡自动接手，并按 `changes[]` 修改。
- 所有写操作仍需用户批准；拒绝后流程不崩。
- 没有 `@` 时，团队模式行为与现在一致。
- 关工具或不支持 tools 时，@ 入口生效但不自动交接，并有明确提示。
- 非法 handoff 只回结构化错误，不产生卡片、不切换角色。
- 刷新后 Handoff 卡片与展开内容仍在。
- `cargo test --workspace` 通过。
