# 工作区体验修复与「无工作区」设计（阶段一）

日期：2026-11-17
状态：已确认（用户批准「先开发第 1 阶段」）
范围：仅本地工作区体验与无工作区语义；远程 SSH 见阶段二（另行设计）。

## 1. 背景：三个症状同一条根因链

用户反馈：切换工作区没生效、Esc 关不掉浮层、打开文件夹没反应。

排查结论（代码级）：

- 文本输入由 `input_target` 决定写入哪个 `TextBuffer`，`EntityInputHandler` 按它注册；
- `open_workspace_folder_input` 把 `input_target` 设为 `Workspace`，但路径行被 Esc/切换/取消关闭时**从不复位**；
- `ws_focus` 对应的输入元素只在路径输入行渲染时存在。复位缺失后，键盘事件被派发到一个没有节点的句柄上，全部丢弃；`render` 的焦点补回逻辑还会每帧把焦点拉回 `ws_focus`；
- 于是打字、Esc、回车同时失效，直到重启。这同时解释了三个「不生效」。

附带问题：

- 「打开文件夹」是手输路径，不是系统目录选择器；
- 切换失败（目录不存在/正在生成）只在底部写 notice、浮层不关，看起来像点击没反应；
- `start_watcher` 只在启动时绑定一次旧 root，切换后外部改动不再刷新文件树；
- 没有真正的「无工作区」：启动总会回落到自动创建的 `~/.neverwork/workspace`，工具又被 `ToolContext` 钉死在这个空目录里。

## 2. 目标 / 非目标

目标：

1. Esc、打开文件夹、切换工作区全部可用，且键盘状态不会再次卡死。
2. 无工作区成为真实状态：对话可用，工具不受 root 限制（用户已确认）。
3. 切工作区后 watcher、文件树、会话库、Git 面板跟随切换。

非目标：

- 远程 SSH 工作区（阶段二）。
- 工作区管理器的视觉重做（沿用现有居中卡片）。
- 终端、LSP、全局搜索。

## 3. 设计

### 3.1 输入目标生命周期（`app/src/state.rs`、`input_handler.rs`）

- 新增 `AppState::restore_input_target()`：仅当 `input_target == Workspace` 且路径行/重命名行都不再渲染时复位——有打开的文件回 `Editor`，否则回 `Chat`。
- 调用点：`apply_action(Escape)` 末尾、`close_workspace_prompt`、`enter_workspace`（成功与失败）、`commit/cancel_rename_workspace`（已有复位，改为调用同一函数）。
- 渲染期防御：在计算 `want` 之前做同样的 clamp（`Workspace` 无对应节点 → 回落），保证即使漏掉某个关闭路径，键盘也不会再被锁死。
- Esc 优先级维持现有顺序（详情抽屉 → 工作区管理器 → 新建会话 → 模型选择器 → 路径行/命令面板 → 改名 → 会话菜单 → 归档确认 → 搜索词 → 角色技能 → 停止生成）。

### 3.2 打开文件夹：系统原生选择器

- 新增 `AppState::pick_workspace_folder(window, cx)`：调用 `cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("选择工作区文件夹") })`，结果经 `cx.spawn` 回到主线程后调用 `enter_workspace`；取消/出错只写 notice。
- 工作区管理器底部拆成两个入口：
  - `＋ 打开文件夹…` → 原生选择器（主入口）；
  - `⌨ 手动输入路径…` → 现有内联路径行（备用入口）。
- 路径行打开时由渲染期焦点补回自动聚焦；`open_workspace_folder_input` 不再依赖点击输入框才能输入。
- 命令面板的「切换工作区…」保持打开管理器（不直接弹系统框）。

### 3.3 切换工作区与 watcher

- `enter_workspace` 失败分支（流式生成中、目录不存在）：写 notice、关闭管理器、`restore_input_target`，不再「装死」。
- 成功分支保持现有流程（存旧会话 → 换根 → 记 settings → 重建文件树 → 换会话库 → 复位浮层 → refresh_git），追加：启动新 watcher、`restore_input_target`。
- `start_watcher` 改为可停止：返回 `flume::Sender<()>` 停止信号，AppState 保存 `watcher_stop: Option<Sender<()>>`；切换/进入无工作区时先发停止信号再重建。
- 无工作区时不启动 watcher。

### 3.4 无工作区状态

- `AppState.root: Option<PathBuf>`、`AppState.workspace: Option<neverwork_core::Workspace>`。
- 启动解析 `resolve_root(saved)` 改为 `Option<PathBuf>`：`NW_ROOT`（必须 is_dir）→ `settings.last_workspace`（必须 is_dir）→ `None`。**删除**自动创建 `~/.neverwork/workspace` 的回落。
- 会话库键：`AppState::session_key()` → 有工作区用 `root.to_string_lossy()`（与现有文件命名完全一致），无工作区用固定键 `scratch`。`core/sessions.rs` 增加 `store_path_key/load_key/load_with_recovery_key/save_key`，旧 `store_path/load/...` 保留为基于路径字符串的包装，保证已有会话文件可继续加载。
- 工具：`ToolContext` 增加 `unrestricted: bool` 与 `ToolContext::unrestricted()`：
  - 根取用户主目录（`USERPROFILE`/`HOME`）；
  - `resolve` 允许绝对路径与 `..`（词法规范化），相对路径基于主目录；
  - `display_rel` 对主目录外路径返回绝对路径；
  - 写操作仍由现有 `requires_approval` → NeedApproval 流程把关，工具内部不落盘。
- Agent API：`run_agent` / `run_agent_in_team` 的 `root: PathBuf` 参数改为 `ctx: ToolContext`；`build_system_prompt` 追加「当前未指定工作区」说明（相对路径以主目录为准、可直接用绝对路径）。app 端按 `self.root` 构造。
- UI 占位：
  - 文件树/Git 面板：显示「未选择工作区」+「打开文件夹…」按钮；
  - 标题栏/侧栏工作区卡片：名称回落为「未选择工作区」，不显示「远程」；
  - 编辑器无文件可开；`refresh_git`、`child_repos` 等本地路径操作在无工作区时跳过。

### 3.5 兼容性

- `settings.json` 结构不变；`last_workspace` 为空串或目录不存在即视为无工作区。
- `NW_ROOT` 语义不变（有效目录才生效）。
- 本地会话文件命名不变；无工作区会话写入 `sessions/scratch-<hash>.json`。

## 4. 错误处理

- 目录不存在/不是目录：notice「不是一个目录：…」，关闭管理器。
- 原生选择器打开失败或用户取消：仅 note（取消不提示）。
- watcher 启动失败（无权限等）：静默跳过，不影响其它功能。
- 无工作区时工具访问失败：沿用现有工具错误回填给模型。

## 5. 测试与验收

单元测试：

- `pick_root(nw_root, saved)`：NW_ROOT 无效时回落 saved；两者都无效 → `None`；不再创建目录。
- `sessions::store_path_key("scratch")` 稳定、`store_path(path)` 与 `store_path_key(path 字符串)` 结果一致。
- `ToolContext::unrestricted()`：绝对路径可用、相对路径落在主目录、`..` 规范化；`ToolContext::new` 的越界拒绝测试保持通过。
- `restore_input_target` / 渲染 clamp 的纯逻辑抽成可测函数（目标 + 渲染状态 → 目标）。

手动验收（本机）：

1. 侧栏工作区卡片 → 打开文件夹 → 原生目录框 → 选中目录后会话列表/文件树/标题栏切换。
2. 打开路径行后按 Esc：浮层关闭，聊天框可立即打字（不再卡死）。
3. 工作区管理器/命令面板/模型选择器：Esc 均可关闭。
4. 最近列表点一个不存在的目录：管理器关闭并显示 notice。
5. 切换后外部新建文件：文件树自动刷新（新 watcher 生效）。
6. 无工作区启动（临时 `NW_CONFIG_DIR`）：聊天可用；让 agent 读取工作区外的绝对路径文件成功；写入仍需审批。
7. `cargo test --workspace` 全绿（依赖真实 Key 的 3 条按原规则跳过）。

## 6. 阶段二（不在本 spec）

远程 SSH 工作区：工作区键与 `RecentWorkspace` 扩展（kind/host）、`SshFs`（系统 ssh CLI：探测、读、写、树、grep、超时、路径引号安全）、`ToolContext` 后端化、远程文件树/编辑器的异步加载与保存、会话仍存本地。传输层已确认走系统 OpenSSH。
