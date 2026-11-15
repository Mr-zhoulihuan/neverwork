//! 应用状态与行为：工作区、会话、输入缓冲区、事件分发。
//!
//! 视图渲染全部在 `views/` 下，本文件只管「状态怎么变」。
//!
//! 输入架构（重要）：
//! - 文本插入**只**经 `EntityInputHandler`（见 `input_handler.rs`），因此中文输入法不会重复插入。
//! - 键盘只处理非文本按键，路由在 `keymap.rs`。
//! - 光标/选区/撤销统一由 `TextBuffer` 提供，对话输入框与编辑器共用同一套语义。

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, App, Bounds, Context, ElementInputHandler, FocusHandle, Focusable, KeyDownEvent,
    Pixels, Window,
};

use neverwork_core::{
    ConnectionKind, Config, FileNode, Message, MessageKind, Session, Workspace,
};
use neverwork_ui::theme;

use crate::data;
use crate::filesystem;
use crate::keymap::{self, KeyAction, Motion};
use crate::line_cache::LineCache;
use crate::text_buffer::TextBuffer;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CenterView {
    Chat,
    File,
    Team,
}

/// 当前接收文本输入的目标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputTarget {
    Chat,
    Editor,
    Workspace,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RightTab {
    Files,
    Changes,
    Preview,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Chat,
    Team,
    Save,
    RefreshTree,
    NewSession,
    Changes,
    Preview,
    SwitchWorkspace,
    FocusMode,
}

impl Command {
    pub fn all() -> Vec<(Command, &'static str, &'static str)> {
        vec![
            (Command::Chat, "回到对话", "Esc"),
            (Command::Team, "打开团队编辑器", ""),
            (Command::Save, "保存当前文件", "Ctrl+S"),
            (Command::RefreshTree, "刷新文件树", ""),
            (Command::NewSession, "新建会话", ""),
            (Command::Changes, "查看最近改动", ""),
            (Command::Preview, "预览当前文件", ""),
            (Command::SwitchWorkspace, "切换工作区…", ""),
            (Command::FocusMode, "专注模式（隐藏侧栏）", ""),
        ]
    }
}

pub struct AppState {
    // ===== 工作区 =====
    pub workspace: Workspace,
    pub root: PathBuf,
    pub files: Vec<FileNode>,
    pub expanded: HashSet<PathBuf>,
    pub recent: Vec<(String, std::time::SystemTime)>,

    // ===== 视图 =====
    pub center: CenterView,
    pub right_tab: RightTab,
    pub palette_open: bool,
    /// 专注模式：隐藏左右栏，中栏独占（窄屏 / 远程桌面下必备）。
    pub focus_mode: bool,

    // ===== 会话 =====
    pub sessions: Vec<Session>,
    pub active_session: usize,

    // ===== 编辑 / 输入 =====
    pub open_path: Option<PathBuf>,
    pub editor: TextBuffer,
    pub chat: TextBuffer,
    pub ws_input: TextBuffer,
    pub ws_prompt_open: bool,
    pub input_target: InputTarget,
    pub marked_range: Option<Range<usize>>,
    pub dirty: bool,
    pub line_cache: RefCell<LineCache>,
    pub editor_scroll: gpui::UniformListScrollHandle,

    // ===== AI / 团队 =====
    pub streaming: bool,
    pub ai_ready: bool,
    pub notice: Option<String>,
    pub config: Config,
    pub cancel: Arc<AtomicBool>,

    // ===== 焦点 =====
    pub focus_handle: FocusHandle,
    pub editor_focus: FocusHandle,
    pub ws_focus: FocusHandle,
    /// 输入区在窗口中的位置（IME 候选框定位用）。
    pub input_area_bounds: std::rc::Rc<Cell<Option<Bounds<Pixels>>>>,
}

fn resolve_root() -> PathBuf {
    if let Ok(p) = std::env::var("NW_ROOT") {
        let path = PathBuf::from(p);
        if path.exists() {
            return path;
        }
    }
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let start = dir.clone();
    loop {
        if dir.join("Cargo.toml").exists() {
            return dir;
        }
        if !dir.pop() {
            return start;
        }
    }
}

impl AppState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let root = resolve_root();
        let expanded = filesystem::default_expanded(&root);
        let files = filesystem::build_tree(&root, &expanded).unwrap_or_default();
        let ws = Workspace {
            name: root
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".into()),
            path: root.to_string_lossy().to_string(),
            kind: ConnectionKind::Local,
            remote_url: None,
            online: true,
        };
        let ai_ready = neverwork_agent::config_from_env().is_some();
        let config = neverwork_core::config::load();
        let sessions = data::demo_sessions();

        let mut me = Self {
            workspace: ws,
            root,
            files,
            expanded,
            recent: Vec::new(),
            center: if std::env::var("NW_VIEW").map(|v| v == "team").unwrap_or(false) {
                CenterView::Team
            } else {
                CenterView::Chat
            },
            right_tab: RightTab::Files,
            palette_open: false,
            focus_mode: false,
            sessions,
            active_session: 0,
            open_path: None,
            editor: TextBuffer::multi_line(),
            chat: TextBuffer::single_line(),
            ws_input: TextBuffer::single_line(),
            ws_prompt_open: false,
            input_target: InputTarget::Chat,
            marked_range: None,
            dirty: false,
            line_cache: RefCell::new(LineCache::new()),
            editor_scroll: gpui::UniformListScrollHandle::new(),
            streaming: false,
            ai_ready,
            notice: None,
            config,
            cancel: Arc::new(AtomicBool::new(false)),
            focus_handle: cx.focus_handle(),
            editor_focus: cx.focus_handle(),
            ws_focus: cx.focus_handle(),
            input_area_bounds: std::rc::Rc::new(Cell::new(None)),
        };

        // 启动时按需打开文件（便于验证）。
        if let Ok(path) = std::env::var("NW_OPEN") {
            let p = PathBuf::from(&path);
            if let Ok(content) = filesystem::read_file(&p) {
                me.editor.reset(content);
                me.editor.move_to(0, false);
                me.open_path = Some(p);
                me.center = CenterView::File;
            }
        }

        start_watcher(me.root.clone(), cx);
        cx.notify();
        me
    }

    // ===== 会话 =====

    pub fn session(&self) -> &Session {
        &self.sessions[self.active_session.min(self.sessions.len().saturating_sub(1))]
    }


    pub(crate) fn new_session(&mut self, cx: &mut Context<Self>) {
        let id = format!("s{}", self.sessions.len() + 1);
        self.sessions.push(Session {
            id,
            title: "新会话".into(),
            subtitle: "空白会话".into(),
            team_name: self
                .config
                .active_team()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "默认协作团队".into()),
            messages: Vec::new(),
            updated_label: "刚刚".into(),
            preview: "空白会话，输入第一条指令开始".into(),
            status: neverwork_core::SessionStatus::Idle,
            meta: String::new(),
            accent: self.sessions.len() % 6,
            icon: "＋".into(),
            badge: String::new(),
            action: neverwork_core::SessionAction::None,
        });
        self.active_session = self.sessions.len() - 1;
        self.center = CenterView::Chat;
        self.palette_open = false;
        cx.notify();
    }

    pub(crate) fn select_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.sessions.len() {
            self.active_session = idx;
            self.center = CenterView::Chat;
            cx.notify();
        }
    }

    // ===== 文件 =====

    pub fn open_file_name(&self) -> String {
        self.open_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "未命名".into())
    }

    pub(crate) fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        self.files = filesystem::build_tree(&self.root, &self.expanded).unwrap_or_default();
        cx.notify();
    }

    pub(crate) fn toggle_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
        self.refresh_tree(cx);
    }

    pub(crate) fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.refresh_tree(cx);
    }

    pub(crate) fn refresh_recent(&mut self, cx: &mut Context<Self>) {
        self.recent = filesystem::recent_files(&self.root, 40);
        cx.notify();
    }

    pub(crate) fn set_right_tab(&mut self, tab: RightTab, cx: &mut Context<Self>) {
        self.right_tab = tab;
        if tab == RightTab::Changes {
            self.recent = filesystem::recent_files(&self.root, 40);
        }
        cx.notify();
    }

    pub(crate) fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        match filesystem::read_file(&path) {
            Ok(content) => {
                self.editor.reset(content);
                // 打开文件时光标回到开头并按需滚动，避免落在文件末尾看不见。
                self.editor.move_to(0, false);
                self.scroll_editor_to_cursor();
                self.open_path = Some(path);
                self.center = CenterView::File;
                self.dirty = false;
                self.notice = None;
                self.on_editor_opened();
            }
            Err(e) => {
                self.notice = Some(format!("无法读取文件：{e}"));
            }
        }
        cx.notify();
    }

    pub(crate) fn save_file(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.open_path.clone() else {
            self.notice = Some("没有打开的文件".into());
            cx.notify();
            return;
        };
        match filesystem::write_file(&path, self.editor.text()) {
            Ok(()) => {
                self.dirty = false;
                self.notice = Some("已保存".into());
                self.refresh_tree(cx);
            }
            Err(e) => self.notice = Some(format!("保存失败：{e}")),
        }
        cx.notify();
    }

    /// 在当前行插入 4 空格缩进；有选区时整块缩进。
    pub(crate) fn insert_indent(&mut self, cx: &mut Context<Self>) {
        match self.editor.selection() {
            Some(sel) => {
                // 选中多行时逐行缩进。
                self.editor.move_to(sel.end, false);
                let text = self.editor.text().to_string();
                let sel_start = sel.start;
                let sel_end = sel.end;
                let block = text[sel_start..sel_end].to_string();
                let mut indented = String::new();
                for (i, line) in block.split('\n').enumerate() {
                    if i > 0 {
                        indented.push('\n');
                    }
                    if !line.trim().is_empty() || i == 0 {
                        indented.push_str("    ");
                    }
                    indented.push_str(line);
                }
                self.editor.move_to(sel_start, false);
                self.editor.move_to(sel_end, true);
                self.editor.insert(&indented);
            }
            None => {
                self.editor.insert("    ");
            }
        }
        self.dirty = true;
        cx.notify();
    }

    pub(crate) fn show_chat(&mut self, cx: &mut Context<Self>) {
        self.center = CenterView::Chat;
        self.input_target = InputTarget::Chat;
        cx.notify();
    }

    pub(crate) fn set_center(&mut self, v: CenterView, cx: &mut Context<Self>) {
        self.center = v;
        cx.notify();
    }

    // ===== 工作区切换 =====

    pub(crate) fn open_workspace_prompt(&mut self, cx: &mut Context<Self>) {
        self.ws_prompt_open = true;
        self.palette_open = true;
        self.ws_input.reset("");
        cx.notify();
    }

    pub(crate) fn apply_workspace_input(&mut self, cx: &mut Context<Self>) {
        let raw = self.ws_input.text().trim().to_string();
        if raw.is_empty() {
            return;
        }
        let path = PathBuf::from(&raw);
        if !path.is_dir() {
            self.notice = Some(format!("不是一个目录：{raw}"));
            cx.notify();
            return;
        }
        self.root = path;
        self.expanded = filesystem::default_expanded(&self.root);
        self.files = filesystem::build_tree(&self.root, &self.expanded).unwrap_or_default();
        self.workspace = Workspace {
            name: self
                .root
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".into()),
            path: self.root.to_string_lossy().to_string(),
            kind: ConnectionKind::Local,
            remote_url: None,
            online: true,
        };
        self.open_path = None;
        self.editor.reset("");
        self.dirty = false;
        self.recent.clear();
        self.center = CenterView::Chat;
        self.ws_prompt_open = false;
        self.palette_open = false;
        self.ws_input.reset("");
        self.notice = Some(format!("已切换到 {}", self.root.display()));
        cx.notify();
    }

    // ===== 命令面板 =====

    pub(crate) fn toggle_focus_mode(&mut self, cx: &mut Context<Self>) {
        self.focus_mode = !self.focus_mode;
        cx.notify();
    }

    pub(crate) fn toggle_palette(&mut self, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        if !self.palette_open {
            self.ws_prompt_open = false;
        }
        cx.notify();
    }

    pub(crate) fn run_command(&mut self, cmd: Command, cx: &mut Context<Self>) {
        self.palette_open = false;
        match cmd {
            Command::Chat => self.show_chat(cx),
            Command::Team => self.set_center(CenterView::Team, cx),
            Command::Save => self.save_file(cx),
            Command::RefreshTree => self.refresh_tree(cx),
            Command::NewSession => self.new_session(cx),
            Command::Changes => self.set_right_tab(RightTab::Changes, cx),
            Command::Preview => self.set_right_tab(RightTab::Preview, cx),
            Command::SwitchWorkspace => self.open_workspace_prompt(cx),
            Command::FocusMode => {
                self.focus_mode = !self.focus_mode;
                cx.notify();
            }
        }
    }

    // ===== 键盘 =====

    fn buffer(&self, target: InputTarget) -> &TextBuffer {
        match target {
            InputTarget::Chat => &self.chat,
            InputTarget::Editor => &self.editor,
            InputTarget::Workspace => &self.ws_input,
        }
    }

    fn buffer_mut(&mut self, target: InputTarget) -> &mut TextBuffer {
        match target {
            InputTarget::Chat => &mut self.chat,
            InputTarget::Editor => &mut self.editor,
            InputTarget::Workspace => &mut self.ws_input,
        }
    }

    fn note_edit(&mut self, target: InputTarget) {
        if target == InputTarget::Editor {
            self.dirty = true;
        }
    }

    pub(crate) fn on_chat_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        self.switch_input_target(InputTarget::Chat);
        if let Some(action) = keymap::route(&ev.keystroke) {
            self.apply_action(InputTarget::Chat, action, cx);
        }
    }

    pub(crate) fn on_editor_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        self.switch_input_target(InputTarget::Editor);
        if let Some(action) = keymap::route(&ev.keystroke) {
            self.apply_action(InputTarget::Editor, action, cx);
        }
    }

    pub(crate) fn on_workspace_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        self.switch_input_target(InputTarget::Workspace);
        if let Some(action) = keymap::route(&ev.keystroke) {
            self.apply_action(InputTarget::Workspace, action, cx);
        }
    }

    fn apply_action(&mut self, target: InputTarget, action: KeyAction, cx: &mut Context<Self>) {
        match action {
            KeyAction::Motion { motion, extend } => {
                let buf = self.buffer_mut(target);
                match motion {
                    Motion::Left => buf.move_left(extend),
                    Motion::Right => buf.move_right(extend),
                    Motion::WordLeft => buf.move_word_left(extend),
                    Motion::WordRight => buf.move_word_right(extend),
                    Motion::Up => buf.move_up(extend),
                    Motion::Down => buf.move_down(extend),
                    Motion::Home => buf.move_home(extend),
                    Motion::End => buf.move_end(extend),
                    Motion::DocStart => buf.move_doc_start(extend),
                    Motion::DocEnd => buf.move_doc_end(extend),
                }
                if target == InputTarget::Editor {
                    self.scroll_editor_to_cursor();
                }
                cx.notify();
            }
            KeyAction::Backspace => {
                self.buffer_mut(target).backspace();
                self.note_edit(target);
                if target == InputTarget::Editor {
                    self.scroll_editor_to_cursor();
                }
                cx.notify();
            }
            KeyAction::Delete => {
                self.buffer_mut(target).delete_forward();
                self.note_edit(target);
                cx.notify();
            }
            KeyAction::SelectAll => {
                self.buffer_mut(target).select_all();
                cx.notify();
            }
            KeyAction::Undo => {
                if self.buffer_mut(target).undo() {
                    self.note_edit(target);
                    if target == InputTarget::Editor {
                        self.scroll_editor_to_cursor();
                    }
                }
                cx.notify();
            }
            KeyAction::Redo => {
                if self.buffer_mut(target).redo() {
                    self.note_edit(target);
                }
                cx.notify();
            }
            KeyAction::Copy => {
                if let Some(text) = self.buffer(target).selected_text().map(|s| s.to_string()) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
            }
            KeyAction::Cut => {
                let taken = self.buffer_mut(target).take_selection();
                if let Some(text) = taken {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                    self.note_edit(target);
                    cx.notify();
                }
            }
            KeyAction::Paste => {
                if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    self.buffer_mut(target).insert(&text);
                    self.note_edit(target);
                    if target == InputTarget::Editor {
                        self.scroll_editor_to_cursor();
                    }
                    cx.notify();
                }
            }
            KeyAction::Save => self.save_file(cx),
            KeyAction::Enter => match target {
                InputTarget::Chat => self.send_message(cx),
                InputTarget::Editor => {
                    self.editor.newline();
                    self.dirty = true;
                    self.scroll_editor_to_cursor();
                    cx.notify();
                }
                InputTarget::Workspace => self.apply_workspace_input(cx),
            },
            KeyAction::Tab { shift } => {
                if target == InputTarget::Editor {
                    if shift {
                        // Shift+Tab：去掉行首 4 空格（简化版反缩进）。
                        let start = self.editor.line_start_of(self.editor.cursor());
                        let text = self.editor.text().to_string();
                        let rest = &text[start..];
                        let strip = if rest.starts_with("    ") {
                            4
                        } else if rest.starts_with("  ") {
                            2
                        } else {
                            0
                        };
                        if strip > 0 {
                            self.editor.move_to(start, false);
                            self.editor.move_to(start + strip, true);
                            self.editor.take_selection();
                            self.dirty = true;
                        }
                    } else {
                        self.insert_indent(cx);
                    }
                    cx.notify();
                }
            }
            KeyAction::Escape => {
                if self.ws_prompt_open {
                    self.ws_prompt_open = false;
                    self.palette_open = false;
                } else if self.palette_open {
                    self.palette_open = false;
                } else if self.streaming {
                    self.stop_streaming(cx);
                }
                cx.notify();
            }
        }
    }

    // ===== 发送 =====

    pub(crate) fn send_message(&mut self, cx: &mut Context<Self>) {
        let text = self.chat.text().trim().to_string();
        if text.is_empty() || self.streaming {
            return;
        }
        let Some((provider_id, config)) = neverwork_agent::config_from_env() else {
            self.notice = Some(
                "未配置 AI：请设置环境变量 NW_API_KEY 后重启（可选 NW_PROVIDER=openai|anthropic、NW_BASE_URL、NW_MODEL）".into(),
            );
            cx.notify();
            return;
        };
        let _ = provider_id;

        let team = self
            .config
            .active_team()
            .cloned()
            .unwrap_or_else(neverwork_core::Team::default_team);

        self.chat.reset("");
        self.notice = None;
        self.center = CenterView::Chat;

        let session_idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        {
            let session = &mut self.sessions[session_idx];
            session.messages.push(Message {
                kind: MessageKind::User,
                body: text.clone(),
                code: None,
            });
            session.team_name = team.name.clone();
            if session.title.is_empty() || session.title == "新会话" {
                session.title = text.chars().take(16).collect();
            }
        }

        self.streaming = true;
        self.cancel.store(false, Ordering::Relaxed);
        cx.notify();

        let cancel = self.cancel.clone();
        let (tx, rx) = flume::unbounded::<neverwork_agent::TeamEvent>();

        std::thread::spawn(move || {
            let mut emit = |ev: neverwork_agent::TeamEvent| {
                let _ = tx.send(ev);
            };
            neverwork_agent::run_team(&team, &text, config, &mut emit, cancel);
        });

        cx.spawn(async move |weak, async_cx| {
            let mut role_msg: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            loop {
                match rx.recv_async().await {
                    Ok(neverwork_agent::TeamEvent::RoleStart { role_index }) => {
                        let _ = weak.update(async_cx, |this, cx| {
                            if let Some(team) = this.config.active_team().cloned() {
                                if let Some(role) = team.roles.get(role_index) {
                                    let idx = this.sessions[session_idx].messages.len();
                                    this.sessions[session_idx].messages.push(Message {
                                        kind: MessageKind::Agent {
                                            participant: neverwork_core::Participant {
                                                role_id: role.id.clone(),
                                                role_name: role.name.clone(),
                                                color_index: role.color_index,
                                                initial: role.initial.clone(),
                                            },
                                        },
                                        body: String::new(),
                                        code: None,
                                    });
                                    role_msg.insert(role_index, idx);
                                }
                            }
                            cx.notify();
                        });
                    }
                    Ok(neverwork_agent::TeamEvent::Delta { role_index, text }) => {
                        let msg_idx = role_msg.get(&role_index).copied();
                        let _ = weak.update(async_cx, |this, cx| {
                            if let Some(idx) = msg_idx {
                                if let Some(m) = this.sessions[session_idx].messages.get_mut(idx) {
                                    m.body.push_str(&text);
                                }
                            }
                            cx.notify();
                        });
                    }
                    Ok(neverwork_agent::TeamEvent::RoleError { error, .. }) => {
                        let _ = weak.update(async_cx, |this, cx| {
                            this.notice = Some(format!("Agent 错误：{error}"));
                            cx.notify();
                        });
                    }
                    Ok(neverwork_agent::TeamEvent::RoleDone { .. }) => {}
                    Ok(neverwork_agent::TeamEvent::AllDone) | Err(_) => {
                        let _ = weak.update(async_cx, |this, cx| {
                            this.streaming = false;
                            cx.notify();
                        });
                        break;
                    }
                }
            }
        })
        .detach();
    }

    pub(crate) fn stop_streaming(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.notice = Some("已停止生成".into());
        cx.notify();
    }
}

impl Focusable for AppState {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 响应式布局：`bounds()` 已是逻辑像素（GPUI 内部做过 physical / scale），不要再除一次。
        // 布局优先级：中栏 > 左侧会话栏 > 右侧文件栏，窄窗口先牺牲右栏，
        // 极窄时才隐藏会话列表（图标栏始终保留，否则没有任何全局入口）。
        let visible_w: f32 = window.bounds().size.width.into();
        let hide_sides = self.focus_mode;
        let hide_right = visible_w < 1080.0;
        let hide_list = visible_w < 620.0;
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::surface())
            .text_color(theme::fg())
            .child(crate::views::titlebar(self, cx))
            .child({
                let row = div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_row()
                    .overflow_hidden();
                if hide_sides {
                    row.child(self.center_area(cx))
                } else {
                    let row = row
                        .child(self.left_sidebar(hide_list, cx))
                        .child(self.center_area(cx));
                    if hide_right {
                        row
                    } else {
                        row.child(self.right_panel(cx))
                    }
                }
            })
            .child(input_bridge(
                cx,
                self.focus_handle.clone(),
                self.editor_focus.clone(),
                self.ws_focus.clone(),
            ));

        if self.palette_open {
            root = root.child(self.palette_overlay(cx).into_any_element());
        }
        root
    }
}

/// 把 IME/文本输入接到当前聚焦的输入目标上。
fn input_bridge(
    cx: &mut Context<AppState>,
    chat_focus: FocusHandle,
    editor_focus: FocusHandle,
    ws_focus: FocusHandle,
) -> impl IntoElement {
    let entity = cx.entity();
    gpui::canvas(
        |_bounds, _window, _cx| {},
        move |bounds, _state, window, cx| {
            let handle = if ws_focus.is_focused(window) {
                Some(&ws_focus)
            } else if editor_focus.is_focused(window) {
                Some(&editor_focus)
            } else if chat_focus.is_focused(window) {
                Some(&chat_focus)
            } else {
                None
            };
            if let Some(handle) = handle {
                window.handle_input(
                    handle,
                    ElementInputHandler::new(bounds, entity.clone()),
                    cx,
                );
            }
        },
    )
    .absolute()
    .top(px(0.0))
    .left(px(0.0))
    .size(px(1.0))
}

/// 监听工作区变化，外部改动后刷新文件树（防抖 300ms）。
fn start_watcher(root: PathBuf, cx: &mut Context<AppState>) {
    use std::time::Duration;
    let (tx, rx) = flume::unbounded::<()>();
    std::thread::spawn(move || {
        let (etx, erx) = std::sync::mpsc::channel::<()>();
        let mut watcher = match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                let _ = etx.send(());
            }
        }) {
            Ok(w) => w,
            Err(_) => return,
        };
        use notify::Watcher;
        if watcher.watch(&root, notify::RecursiveMode::Recursive).is_err() {
            return;
        }
        loop {
            if erx.recv().is_err() {
                break;
            }
            while erx.recv_timeout(Duration::from_millis(300)).is_ok() {}
            let _ = tx.send(());
        }
    });

    cx.spawn(async move |weak, async_cx| {
        while rx.recv_async().await.is_ok() {
            let _ = weak.update(async_cx, |this, cx| {
                this.files =
                    filesystem::build_tree(&this.root, &this.expanded).unwrap_or_default();
                cx.notify();
            });
        }
    })
    .detach();
}
