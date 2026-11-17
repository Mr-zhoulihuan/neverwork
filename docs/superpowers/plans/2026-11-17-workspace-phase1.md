# 工作区阶段一实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 Esc/打开文件夹/切换工作区失效，并把「无工作区」做成真实状态（对话可用、工具不受 root 限制）。

**Architecture:** 先给 core 的会话库加字符串键 API，再让 `ToolContext` 支持无限制模式并把 agent 入口参数从 `PathBuf` 换成 `ToolContext`；随后把 `AppState.root/workspace` 改成 `Option`，统一会话键、watcher 与所有本地路径调用；最后收口输入目标生命周期、接入系统原生目录选择器、补无工作区占位 UI。

**Tech Stack:** Rust 2021、gpui 0.2.1、serde、flume、notify。

> 仓库规则：不主动 `git commit`，本计划用「验证」代替提交步骤；需要提交时由用户明确指示。

---

### Task 1: 会话库键化（core）

**Files:**
- Modify: `NeverWork/crates/core/src/sessions.rs`

- [ ] **Step 1: 写失败测试**

在 `sessions.rs` 的 `mod tests` 末尾追加：

```rust
#[test]
fn path_wrapper_matches_key_variant() {
    let root = Path::new("E:\\AI\\ide");
    assert_eq!(store_path(root), store_path_key(&root.to_string_lossy()));
}

#[test]
fn scratch_key_is_stable_and_readable() {
    let a = file_name_for_key("scratch");
    let b = file_name_for_key("scratch");
    assert_eq!(a, b);
    assert!(a.starts_with("scratch-"), "{a}");
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p neverwork-core sessions`
Expected: 编译失败（`file_name_for_key` / `store_path_key` 不存在）。

- [ ] **Step 3: 最小实现**

`file_name_for(root: &Path)` 改为 `file_name_for_key(key: &str)`（函数体里 `raw` 直接是 `key`），并新增：

```rust
pub fn store_path_key(key: &str) -> PathBuf {
    sessions_dir().join(file_name_for_key(key))
}
pub fn store_path(root: &Path) -> PathBuf {
    store_path_key(&root.to_string_lossy())
}
pub fn load_key(key: &str) -> SessionStore {
    load_with_recovery_key(key).0
}
pub fn load_with_recovery_key(key: &str) -> (SessionStore, LoadReport) {
    // 原 load_with_recovery 的函数体，store_path(root) → store_path_key(key)
}
pub fn load_with_recovery(root: &Path) -> (SessionStore, LoadReport) {
    load_with_recovery_key(&root.to_string_lossy())
}
pub fn save_key(key: &str, store: &SessionStore) -> AppResult<()> {
    // 原 save 的函数体，store_path(root) → store_path_key(key)
}
pub fn save(root: &Path, store: &SessionStore) -> AppResult<()> {
    save_key(&root.to_string_lossy(), store)
}
```

同步把现有测试里的 `file_name_for(Path::new(...))` 改成 `file_name_for_key("...")`。

- [ ] **Step 4: 验证**

Run: `cargo test -p neverwork-core`
Expected: 全绿（含原有 53 条）。

---

### Task 2: ToolContext 无限制模式 + agent 入口改传 ctx

**Files:**
- Modify: `NeverWork/crates/agent/src/tools.rs:237-307`
- Modify: `NeverWork/crates/agent/src/agent_loop.rs:361-452, 879-925`
- Modify: `NeverWork/crates/agent/tests/handoff_flow.rs:168-187`
- Modify: `NeverWork/crates/agent/tests/deepseek_tools.rs:115-127`
- Modify: `NeverWork/crates/app/src/state.rs:2733, 2780-2805`

- [ ] **Step 1: 写失败测试**

`tools.rs` 测试模块新增：

```rust
#[test]
fn unrestricted_allows_absolute_paths() {
    let dir = std::env::temp_dir().join("nw-unrestricted-test");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a.txt");
    std::fs::write(&file, "hi").unwrap();
    let ctx = ToolContext::unrestricted();
    let got = ctx.resolve(file.to_str().unwrap()).unwrap();
    assert_eq!(got, file.canonicalize().unwrap());
}

#[test]
fn unrestricted_relative_resolves_under_home() {
    let ctx = ToolContext::unrestricted();
    let got = ctx.resolve("nw-probe-file").unwrap();
    assert!(got.starts_with(ToolContext::home_dir()));
    assert!(ctx.display_rel(&got).starts_with("nw-probe-file"));
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p neverwork-agent --lib tools::tests`
Expected: FAIL（`unrestricted`/`home_dir` 不存在）。

- [ ] **Step 3: 实现**

`ToolContext` 加字段与方法：

```rust
pub struct ToolContext {
    /// 工作区根；无限制模式下是用户主目录，仅作相对路径基准。
    pub root: PathBuf,
    /// 未指定工作区：不限制在 root 内。
    unrestricted: bool,
}

impl ToolContext {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = fs::canonicalize(&root).unwrap_or(root);
        Self { root, unrestricted: false }
    }

    /// 未指定工作区：绝对路径直接可用，相对路径以用户主目录为基准。
    pub fn unrestricted() -> Self {
        Self { root: Self::home_dir(), unrestricted: true }
    }

    pub fn is_unrestricted(&self) -> bool { self.unrestricted }

    pub fn home_dir() -> PathBuf {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn resolve(&self, rel: &str) -> Result<PathBuf, String> {
        let raw = rel.trim();
        let raw = if raw.is_empty() { "." } else { raw };
        let p = Path::new(raw);
        if self.unrestricted {
            let joined = if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) };
            let normalized = normalize(&joined);
            if normalized.exists() {
                if let Ok(canon) = fs::canonicalize(&normalized) {
                    return Ok(canon);
                }
            }
            return Ok(normalized);
        }
        // ……原有受限逻辑不变
    }
}
```

`display_rel` 保持不变（主目录外走 `unwrap_or_else` 返回绝对路径，已满足需求）；`resolve_write` 复用 `resolve`，`.git` 目录检查自然生效。

`agent_loop.rs`：

- `run_agent` / `run_agent_in_team` / `run_agent_impl` 的 `root: PathBuf` 全部改为 `ctx: tools::ToolContext`；
- 删除 `let ctx = ToolContext::new(root.clone());`；
- `build_system_prompt(system_prompt, &ctx)`：签名改为 `fn build_system_prompt(persona: &str, ctx: &tools::ToolContext) -> String`，根目录行与「路径一律用相对工作区根的路径」按 `ctx.is_unrestricted()` 分支：

```rust
let root_p = ctx.root.display();
let paths_hint = if ctx.is_unrestricted() {
    "当前没有指定工作区：相对路径以用户主目录为基准，也可以直接使用绝对路径。"
} else {
    "路径一律用相对工作区根的路径。"
};
```

- 两个测试文件的调用处改为 `neverwork_agent::tools::ToolContext::new(dir.clone())` / `ToolContext::new(std::env::temp_dir())`（`tools` 模块已在 lib 中公开）。

`app/src/state.rs` 的 `spawn_agents_chain`：`let root = self.root.clone();` 改为 `let ctx = neverwork_agent::tools::ToolContext::new(self.root.clone());`（Task 3 再按 Option 分支），两处调用改传 `ctx.clone()`。

- [ ] **Step 4: 验证**

Run: `cargo test -p neverwork-agent --lib`
Expected: 全绿；`cargo check -p neverwork` 通过。

---

### Task 3: AppState 的 root / workspace 改为 Option

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`（结构体字段、`new`、`save_sessions`、`refresh_tree`、`git_dir`、`discover_git_repos`、`refresh_git`、`enter_workspace`、`resolve_root`）
- Modify: `NeverWork/crates/app/src/views/files.rs:200,215,555,578`
- Modify: `NeverWork/crates/app/src/views/mod.rs:48-77`
- Modify: `NeverWork/crates/app/src/views/sidebar.rs:279`
- Modify: `NeverWork/crates/app/src/views/palette.rs:123`
- Modify: `NeverWork/crates/app/src/views/workspace_picker.rs:47`
- Modify: `NeverWork/crates/app/src/views/chat.rs:925`

- [ ] **Step 1: 写失败测试**

`state.rs` 内新增纯函数与测试：

```rust
fn pick_root(nw_root: Option<PathBuf>, saved: &str) -> Option<PathBuf> {
    if let Some(p) = nw_root.filter(|p| p.is_dir()) { return Some(p); }
    let saved = saved.trim();
    if !saved.is_empty() {
        let p = PathBuf::from(saved);
        if p.is_dir() { return Some(p); }
    }
    None
}

fn resolve_root(saved: &str) -> Option<PathBuf> {
    pick_root(std::env::var_os("NW_ROOT").map(PathBuf::from), saved)
}
```

测试（`tests` 模块或文件末尾）：

```rust
#[test]
fn pick_root_prefers_valid_nw_root() {
    let dir = std::env::temp_dir();
    assert_eq!(pick_root(Some(dir.clone()), ""), Some(dir));
}

#[test]
fn pick_root_falls_back_to_none_without_valid_dirs() {
    assert_eq!(pick_root(None, ""), None);
    assert_eq!(pick_root(None, "Z:\\no-such-dir-nw"), None);
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p neverwork --lib state::tests`
Expected: 编译失败（`resolve_root` 返回类型不匹配）。

- [ ] **Step 3: 实现**

`AppState` 字段：

```rust
pub root: Option<PathBuf>,
pub workspace: Option<Workspace>,
```
（移除原来的 `initial_view` 依赖之外的旧构造；`new()` 里：）

```rust
let root = resolve_root(&settings.last_workspace);
let expanded = root.as_ref().map(filesystem::default_expanded).unwrap_or_default();
let files = root.as_ref()
    .and_then(|r| filesystem::build_tree(r, &expanded).ok())
    .unwrap_or_default();
let ws = root.as_ref().map(|root| Workspace { /* 原逻辑 */ });
let session_key = root.as_ref().map(|r| r.to_string_lossy().to_string())
    .unwrap_or_else(|| "scratch".to_string());
let (store, report) = neverwork_core::sessions::load_with_recovery_key(&session_key);
```

新增 `impl AppState`：

```rust
/// 会话库键：有工作区用本地路径（兼容旧文件），无工作区用固定 `scratch`。
pub(crate) fn session_key(&self) -> String {
    self.root.as_ref().map(|r| r.to_string_lossy().to_string())
        .unwrap_or_else(|| "scratch".to_string())
}
pub(crate) fn has_workspace(&self) -> bool { self.root.is_some() }
```

`save_sessions`：`sessions::save(&self.root, ...)` → `sessions::save_key(&self.session_key(), ...)`。

`refresh_tree`：root 为 None 时 `self.files.clear()` 后 notify。

`git_dir` → `Option<PathBuf>`（`git_active.clone().or_else(|| self.root.clone())`）；`discover_git_repos` / `refresh_git` 在 `None` 时清空 git 状态并 return。

`spawn_agents_chain`：`let ctx = match &self.root { Some(r) => ToolContext::new(r.clone()), None => ToolContext::unrestricted() };`

各视图调用点：

- `files.rs:200` `.map(|w| w.name.clone()).unwrap_or_else(|| "未选择工作区".into())`
- `files.rs:215` `let abs = self.root.as_ref().map(|r| r.join(&f.path));`（`is_open` 用 `abs.as_deref()`）
- `files.rs:555/578`（`child_repos` 区块）包一层 `if let Some(root) = &self.root { ... }`，否则不渲染
- `mod.rs:48` `let kind = state.workspace.as_ref().map(|w| w.kind == ConnectionKind::Remote && "· 远程")...` 与名称回落「未选择工作区」
- `sidebar.rs:279`、`workspace_picker.rs:47`、`palette.rs:123`、`chat.rs:925` 同理回落 / `as_ref()`

- [ ] **Step 4: 验证**

Run: `cargo check -p neverwork && cargo test -p neverwork --lib`
Expected: 编译通过；除已知 3 条需 Key 的测试外全绿。

---

### Task 4: watcher 可停止 + enter_workspace 加固 + 启动接入

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`（字段 `watcher_stop`、`new`、`enter_workspace`、`start_watcher`）

- [ ] **Step 1: 实现**

字段：`watcher_stop: Option<flume::Sender<()>>`（`new` 里 `None`）。

`start_watcher(root: PathBuf, cx) -> flume::Sender<()>`：线程循环改为可退出：

```rust
loop {
    if stop.try_recv().is_ok() { break; }
    match erx.recv_timeout(Duration::from_millis(200)) {
        Ok(()) => { while erx.recv_timeout(Duration::from_millis(300)).is_ok() {} let _ = tx.send(()); }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
        Err(_) => break,
    }
}
```

`new()`：`me.watcher_stop = me.root.clone().map(|r| start_watcher(r, cx));`

`enter_workspace`：成功分支追加

```rust
if let Some(stop) = self.watcher_stop.take() { let _ = stop.send(()); }
```
并在换根后 `self.watcher_stop = self.root.clone().map(|r| start_watcher(r, cx));`；失败分支补 `self.workspace_popup = false; self.restore_input_target();`。

- [ ] **Step 2: 验证**

Run: `cargo check -p neverwork`
Expected: 通过。

---

### Task 5: 输入目标生命周期 + Esc 收口

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`（`restore_input_target`、`apply_action` Escape、`close_workspace_prompt`、`enter_workspace`、render clamp）

- [ ] **Step 1: 实现**

```rust
/// 关闭浮层后把键盘交还正文：路径行/重命名行不在时，Workspace 目标必须复位。
pub(crate) fn restore_input_target(&mut self) {
    if self.input_target != InputTarget::Workspace { return; }
    if self.ws_prompt_open || self.ws_rename.is_some() { return; }
    self.input_target = if self.center == CenterView::File && self.open_path.is_some() {
        InputTarget::Editor
    } else {
        InputTarget::Chat
    };
    self.marked_range = None;
}
```

调用点：`close_workspace_prompt`、`apply_action` 的 `KeyAction::Escape` 分支末尾、`enter_workspace` 两个失败分支与成功末尾。

渲染 clamp（`render` 里计算 `want` 之前）：

```rust
if self.input_target == InputTarget::Workspace
    && !self.ws_prompt_open
    && self.ws_rename.is_none()
{
    self.restore_input_target();
}
```

- [ ] **Step 2: 验证**

Run: `cargo check -p neverwork`
Expected: 通过。

---

### Task 6: 原生目录选择器 + 管理器底部入口

**Files:**
- Modify: `NeverWork/crates/app/src/state.rs`（`pick_workspace_folder`）
- Modify: `NeverWork/crates/app/src/views/workspace_picker.rs`（底部两个按钮）

- [ ] **Step 1: 实现**

```rust
pub(crate) fn pick_workspace_folder(&mut self, cx: &mut Context<Self>) {
    use gpui::PathPromptOptions;
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("选择工作区文件夹".into()),
    });
    self.workspace_popup = false;
    cx.spawn(async move |weak, async_cx| {
        match rx.await {
            Ok(Ok(Some(paths))) => {
                if let Some(p) = paths.into_iter().next() {
                    let _ = weak.update(async_cx, |this, cx| this.enter_workspace(p, cx));
                }
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => { let _ = weak.update(async_cx, |this, cx| {
                this.notice = Some(format!("打开文件夹失败：{e}")); cx.notify(); }); }
            Err(_) => {}
        }
    }).detach();
}
```

`workspace_picker.rs` 底部改成两个按钮：`＋ 打开文件夹…` → `this.pick_workspace_folder(cx)`；`⌨ 手动输入路径…` → 原 `open_workspace_folder_input(cx)`。

- [ ] **Step 2: 验证**

Run: `cargo check -p neverwork`
Expected: 通过（若 `Context` 不直接提供 `prompt_for_paths`，用 `cx.cx().prompt_for_paths(...)` 或 `App` 上的同名方法）。

---

### Task 7: 无工作区占位 UI

**Files:**
- Modify: `NeverWork/crates/app/src/views/files.rs`（`files_panel` / `git_panel` 的根节点）

- [ ] **Step 1: 实现**

`files_panel` 开头：`if self.root.is_none() { return 占位 div（「未选择工作区」+「打开文件夹…」按钮 → pick_workspace_folder）; }`

`git_panel` 开头：`if self.root.is_none() { return 占位 div（「未选择工作区」）; }`

- [ ] **Step 2: 验证**

Run: `cargo check -p neverwork && cargo test -p neverwork --lib`
Expected: 通过。

---

### Task 8: 全量验证

- [ ] **Step 1: 单测**

Run: `cargo test --workspace`
Expected: 全绿（`NW_DEEPSEEK_KEY` 未设置时 3 条真实调用自动跳过）。

- [ ] **Step 2: 手动验收（本机 GUI）**

用临时配置目录启动：`$env:NW_CONFIG_DIR="$PWD\.tmp_ws"; cargo run -p neverwork`
依次验证：

1. 侧栏工作区卡片 → 「打开文件夹…」→ 系统目录框 → 选中目录后列表/文件树/标题栏切换；
2. 手动输入路径行 → Esc → 聊天框立即可输入（不再卡死）；
3. 管理器/⌘K/模型选择器 Esc 关闭；
4. 最近列表点不存在目录 → 浮层关闭 + notice；
5. 切换后外部新建文件 → 文件树自动刷新；
6. 无工作区启动 → 聊天可用；agent 读工作区外绝对路径成功，写操作仍弹审批。

- [ ] **Step 3: 清理**

删除 `.tmp_ws` 与截图临时文件。
