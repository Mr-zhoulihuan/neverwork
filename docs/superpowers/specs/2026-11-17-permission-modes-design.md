# NeverWork 会话权限模式设计

日期：2026-11-17
状态：已确认，开发中

## 背景

当前 Agent 的写操作与命令执行是**逐次审批**（`requires_approval → NeedApproval`），
没有会话级权限概念：用户无法「只看不改」，也无法在信任的会话里关掉逐次打扰。
本次为对话框增加四档权限：只读 / 计划 / 标准 / 完整。

## 档位语义

| 档位 | 模型可见工具 | 写操作与命令 |
| --- | --- | --- |
| 只读 `read_only` | 读文件、列目录、搜索、联网 | 不发送写工具（含全部 MCP 工具） |
| 计划 `plan` | 同只读 | 同只读；系统提示强制先产出实施计划，等用户切档 |
| 标准 `standard`（默认） | 全部 | 逐次弹审批（历史行为） |
| 完整 `full` | 全部 | 直接执行、不弹审批；首次切入需确认 |

## 执行链约束

- `core::PermissionMode`，serde `snake_case`，默认 `standard`；`Session.permission`
  随会话落盘，旧会话缺字段回落标准。
- `run_agent` / `run_agent_in_team` 增加 `permission` 参数，团队每个角色同档。
- 工具清单按档过滤：只读/计划剔除 `requires_approval = true` 的工具（内置写工具 +
  全部 MCP 工具）。**隐藏 + 执行兜底**：若模型仍发来写调用，执行前直接回
  `PERMISSION_DENIED` 的 blocked 结果，不弹审批。
- `need_approval`：标准=按工具；完整=false；只读/计划=false（写工具已不可见）。
- 系统提示按档追加：只读=不能改、只说明方案；计划=先出计划、不要声称已修改；
  完整=已获完整授权、仍避免破坏性操作。
- **完整权限只免审批，不免沙箱**：`ToolContext::resolve`（挡 `..`/跨盘/符号链接）、
  `.git` 禁写、命令超时与输出截断全部保留。

## UI

- 输入框底栏「＋」右侧常驻权限 chip：`◉ 只读 / ◇ 计划 / ◈ 标准 / ⚡ 完整`；
  工具关闭时置灰，点击提示先开工具。
- 点击 chip 在输入框上方展开内联面板（与思考面板同款）：四档卡片（名称 + 一句
  说明 + 选中标记），头部显示「随会话保存」与「收起」。
- 切「完整」时面板原地出现确认条：「完整权限会跳过写操作与命令审批」，
  取消 / 确认开启；确认后才生效。
- 与「＋」菜单、思考面板互斥；Esc / 收起可关；生成中可切，下一条消息生效。

## 状态与验证

- `set_permission` 写 `session.permission` + `save_sessions` + notice 提示。
- 新会话默认标准；设置页不加全局默认（YAGNI）。
- 验证变量 `NW_PERMISSION=read_only|plan|standard|full`；`NW_COMPOSER=permission`
  启动即展开面板（截图用）。
- 测试：core 旧会话缺字段默认标准 + serde 往返；agent `specs_for` 在只读/计划下
  不含任何 `requires_approval` 工具、标准/完整含全部；系统提示含档位说明。
- 真机验证：只读档请求写文件应被拒绝且不落盘；完整档写文件不弹审批直接执行。
