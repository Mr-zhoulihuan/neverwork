# 远程 SSH 工作区实现计划（阶段二）

> 依据 `docs/superpowers/specs/2026-11-17-workspace-phase2-design.md`。执行时逐任务验证，本地既有测试必须保持全绿。

**目标：** 通过系统 OpenSSH 打开远程目录为工作区，文件树 / 编辑器 / Agent 工具作用于远端，会话存本机。

**技术要点：** `core::ssh`（SshTarget + RemoteFs）、`agent::ToolContext` 后端化、app 异步远程 IO（thread + flume + cx.spawn）、`remote_gen` 丢弃过期结果。

---

### Task 1: core 新增 ssh 模块（连接串 + 远程文件操作）

**Files:**
- Create: `NeverWork/crates/core/src/ssh.rs`
- Modify: `NeverWork/crates/core/src/lib.rs`（`pub mod ssh;`）

**内容：**
- `SshTarget { host, port, root }`：`parse(&str)` 支持 `[user@]host[:port]:/abs/path` 与 `ssh://...`；`url()` 输出 `ssh://[user@]host[:port]/root`；`name()` 取 root 末段；`is_remote_url(&str)`。
- 纯函数：`sh_quote`、`normalize_rel`（拒绝 `..` 越界）、`abs_path(root, rel)`。
- `RemoteFs`：`probe`（`cd '<root>' && pwd`）、`tree(expanded: &HashSet<String>, max_depth)`、`list(rel)`、`read_text(rel)`、`write_text(rel, content)`、`search_fixed(pattern, base, limit)`。
- `run_ssh(target, command, stdin: Option<&[u8]>)`：BatchMode + ConnectTimeout=8 + ServerAlive；`-p` 仅非 22；stderr 映射错误分类。
- 解析函数 `parse_ls`、`parse_tree_output`、`parse_grep` 独立成纯函数便于测试。
- 测试：roundtrip parse/url、引号、相对路径越界、解析函数；`#[ignore]` 的 `NW_SSH_TEST` 集成测试。

**验证：** `cargo test -p neverwork-core ssh`

---

### Task 2: agent `ToolContext` 后端化（本地行为不变）

**Files:**
- Modify: `NeverWork/crates/agent/src/tools.rs`
- Modify: `NeverWork/crates/agent/src/multi_patch.rs`

**内容：**
- `ToolContext` 内部 `Backend { Local(LocalFs), Remote(RemoteFs) }`，保留 `new/unrestricted`，新增 `remote(SshTarget)`、`is_remote()`、`describe()`。
- 统一接口：`read_text(rel) -> Result<String>`、`write_text(rel, content)`、`exists(rel)`、`list(rel) -> Vec<(bool,String)>`、`search_fixed(pattern, base) -> Vec<String>`、`display(rel) -> String`、`local_root() -> Option<&Path>`。
- 本地分支复用现有 resolve/canonicalize/`.git` 拦截逻辑；`write_text` 内做 `.git` 拒绝。
- `tools.rs` 六个工具与 `preview` 改为调用接口；`multi_patch::plan/execute` 改用 `ctx.read_text/write_text/exists`。
- 远程 `search_code` 返回 `UNSUPPORTED`「远程暂不支持语义检索，请用 search_files」。

**验证：** `cargo test -p neverwork-agent`（134 条既有用例全过）

---

### Task 3: 系统提示词与远程说明

**Files:**
- Modify: `NeverWork/crates/agent/src/agent_loop.rs::build_system_prompt`

**内容：** 本地/无工作区表述不变；远程追加「远程工作区 host:path，工具操作远端文件，路径相对该根」。

**验证：** `cargo test -p neverwork-agent tests`

---

### Task 4: AppState 远程工作区生命周期

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`

**内容：**
- 字段：`remote: Option<SshTarget>`、`remote_expanded: HashSet<String>`、`remote_open_rel: Option<String>`、`remote_gen: u64`、`ws_remote_open: bool`、`ws_remote_err: Option<String>`。
- `has_workspace()` = `workspace.is_some()`；`session_key()` 远程用 `target.url()`。
- `enter_workspace` 本地分支清空 remote_* 并停 watcher；新增 `enter_remote_workspace(target, cx)`（存旧会话→设置 workspace/remote→停 watcher→换会话库→清展开/打开文件→异步树→复位浮层）。
- 启动：`NW_REMOTE` 或 `last_workspace` 以 `ssh://` 开头 → 解析并 `enter_remote_workspace`（先置状态，再异步 probe；失败 notice + `workspace.online=false`）。
- `spawn_agents_chain`：远程用 `ToolContext::remote(target)`。
- `git_dir()` 远程返回 `None`；`refresh_git` 无工作区/远程静默清状态。

**验证：** `cargo check -p neverwork`

---

### Task 5: 远程文件树（异步）

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`
- Modify: `NeverWork/crates/app/src/views/files.rs`

**内容：**
- `refresh_tree`：远程 → `refresh_remote_tree(cx)`（thread + flume + `remote_gen` 校验；成功填 `files`，失败 notice）。
- `toggle_dir`：远程切换 `remote_expanded` 后刷新；本地不变。
- 文件树刷新按钮对远程可用；`files_panel` 点击目录/文件按远程分支处理。
- 远程 Git 面板占位「远程工作区暂不支持 Git 面板」。

**验证：** `cargo check -p neverwork`

---

### Task 6: 远程编辑器打开 / 保存

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`
- Modify: `NeverWork/crates/app/src/views/files.rs`
- Modify: `NeverWork/crates/app/src/views/chat.rs`

**内容：**
- `open_remote_file(rel, cx)`：异步读（512KB 上限），成功 `editor.reset`、`remote_open_rel=Some`、`open_path=None`、`center=File`、`dirty=false`。
- `save_file`：远程异步写；成功清 dirty 并刷新远程树，失败保留 dirty。
- `open_file_name` / `has_open_file` / `auto_save` / `center==File` 判断同时考虑 `remote_open_rel`。
- 文件面板选中高亮与点击按远程 rel；对话工具步骤路径拼接远程根。

**验证：** `cargo check -p neverwork`

---

### Task 7: 工作区管理器远程入口与最近列表

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`（`connect_remote_from_input` 等）
- Modify: `NeverWork/crates/app/src/views/workspace_picker.rs`
- Modify: `NeverWork/crates/app/src/views/mod.rs`（标题栏远程名，如需）

**内容：**
- 底部「⇅ 连接远程 SSH…」→ 内联输入（复用 `ws_input`/`ws_focus`，`InputTarget::Workspace`），Enter/按钮触发 `connect_remote_from_input`：解析 → 异步 probe（`cd && pwd` 展开 `~`）→ 成功切工作区；失败 `ws_remote_err` 就地显示。
- 最近列表远程行显示「远程」标签、跳过 `is_dir`；点击远程项走 probe 后切换；本地项不变。
- Esc / 关闭浮层调用 `restore_input_target`（阶段一机制）。

**验证：** `cargo check -p neverwork`；`NW_WORKSPACE_POPUP=1` 手动看布局

---

### Task 8: 全量验证与文档

**内容：**
- `cargo test --workspace` 全绿；`cargo build -p neverwork`。
- 更新 `AGENTS.md` 的远程说明（一段：远程走系统 ssh、Git/watcher 不支持、NW_REMOTE 调试开关）。
- 输出手动验收清单给用户。

---

## 完成记录（2026-11-17）

- Task 1–8 全部完成；`cargo test --workspace` 全绿（2 ignored：真实远端 / 真实 API），`cargo build -p neverwork` 通过。
- 实现期补充：`RemoteFs::read_text_exact`（>512KB 直接报错）——避免截断内容保存时写坏远端文件。
- 实现期补充：远程连接 / 打开文件加 `remote_conn_gen` / `remote_open_gen` 代次守卫，切工作区或快速连点文件时迟到结果不会覆盖新状态；保存结果也校验当前文件。
- 入口补充：命令面板「手动输入路径」同样接受 `ssh://` 连接串；侧栏工作区条与文件树标题用 ⇅ / `host:path` 标识远程。

## 追加：应用内密码认证（2026-11-17，用户验收时追加）

- `core::ssh::RemoteAuth`（KeyOnly / Password）+ `RemoteFs::with_auth`；有密码时 ssh 改用
  `BatchMode=no`、`NumberOfPasswordPrompts=1`、`PreferredAuthentications=publickey,password,keyboard-interactive`，
  密码经 `SSH_ASKPASS` + `NW_ASKPASS_SECRET` 交给应用本体的 askpass 分支（`main.rs` 最先处理），
  不落盘、不进命令行；`Debug` 手写打码，单测断言不泄密。
- `AppState.remote_pw` 只存内存；连接行新增密码框（`ws_pw_input` + `ws_pw_focus`，与连接串共用
  `ws_focus` / `InputTarget::Workspace`，失焦只显示圆点）；连接成功转存内存并清空输入框，
  切本地工作区或进本地时清掉。最近项重连先试密钥，失败就地展开输入行让用户补密码。
- `ToolContext::remote_auth` 让 Agent 工具也带同一认证。
- 测试：新增 `debug_never_leaks_password`（已绿）与 `remote_password_roundtrip`（`#[ignore]`，
  需 `NW_SSH_TEST_PASSWORD` + `NW_ASKPASS_EXE`）；真机连接由用户自测。
