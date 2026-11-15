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
    ConnectionKind, Config, FileNode, Message, MessageKind, Session, ToolRunStatus, Workspace,
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
    Settings,
    Skills,
}

/// 设置页的分区。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    General,
    Models,
    Skills,
    Sync,
    About,
}

impl SettingsTab {
    pub fn all() -> [SettingsTab; 5] {
        [
            SettingsTab::General,
            SettingsTab::Models,
            SettingsTab::Skills,
            SettingsTab::Sync,
            SettingsTab::About,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            SettingsTab::General => "常规",
            SettingsTab::Models => "模型与厂商",
            SettingsTab::Skills => "技能",
            SettingsTab::Sync => "同步与升级",
            SettingsTab::About => "关于",
        }
    }

    pub fn glyph(&self) -> &'static str {
        match self {
            SettingsTab::General => "⚙",
            SettingsTab::Models => "◆",
            SettingsTab::Skills => "✦",
            SettingsTab::Sync => "⟳",
            SettingsTab::About => "ⓘ",
        }
    }
}

/// 当前接收文本输入的目标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputTarget {
    Chat,
    Editor,
    Workspace,
    /// 设置页 / 技能页里的某个表单字段。
    Field(FormField),
}

/// 可编辑的表单字段。索引指向 `settings.providers` 的下标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormField {
    /// 技能市场搜索框。
    SkillSearch,
    /// 厂商 API Key。
    ProviderKey(u8),
    /// 厂商 API 基址。
    ProviderUrl(u8),
    /// 厂商模型名。
    ProviderModel(u8),
    /// 厂商显示名。
    ProviderName(u8),
    /// 导入配置的路径。
    ImportPath,
    /// 模型选择器里的搜索框。
    ModelSearch,
    /// 编辑器查找框。
    FindQuery,
    /// 编辑器替换框。
    FindReplace,
}

impl FormField {
    pub fn index(self) -> Option<u8> {
        match self {
            FormField::ProviderKey(i)
            | FormField::ProviderUrl(i)
            | FormField::ProviderModel(i)
            | FormField::ProviderName(i) => Some(i),
            _ => None,
        }
    }
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
    Settings,
    Skills,
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
            (Command::Settings, "打开设置", ""),
            (Command::Skills, "技能市场", ""),
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
    /// 对话区滚动句柄：新消息到来时自动滚到底部。
    pub chat_scroll: gpui::ScrollHandle,
    /// 对话内容有更新、需要滚到底部（渲染时消费）。
    pub chat_scroll_dirty: std::cell::Cell<bool>,

    // ===== 设置 =====
    pub settings: neverwork_core::Settings,
    pub settings_tab: SettingsTab,
    /// 设置页表单当前编辑的字段与它的缓冲区。
    pub active_field: Option<FormField>,
    pub field_buf: TextBuffer,
    /// 技能市场：分类过滤（None = 全部）。
    pub skill_filter: Option<neverwork_core::SkillCategory>,
    /// 技能市场排序：0=综合 1=安装量 2=星标。
    pub skill_sort: u8,
    /// 技能页顶部 Tab：0=技能 1=Agent 模板。
    pub skills_tab: u8,
    /// 技能搜索关键词。
    pub skill_search: String,
    /// 导入配置的路径。
    pub import_path: String,
    /// 打开中的模型选择器（厂商下标）；None 表示未打开。
    pub model_picker: Option<usize>,
    /// 模型选择器的搜索词。
    pub model_query: String,
    /// 模型选择器里键盘高亮的下标。
    pub model_highlight: usize,
    /// 导出配置时是否包含 API Key（默认不含）。
    pub export_with_secrets: bool,
    /// 是否允许 Agent 调用工具（读写文件）。关掉就退化为纯对话。
    pub tools_enabled: bool,
    /// 等待用户批准的写操作。同一时刻最多一个。
    pub pending: Option<PendingApproval>,
    /// 自动批准写操作（仅供自动化验证，`NW_AUTO_APPROVE=1` 开启）。
    pub auto_approve: bool,
    /// 展开的工具步骤组（键 = 该组第一条消息的下标）。
    pub expanded_steps: std::collections::HashSet<usize>,
    /// 右侧栏是否收起。
    pub right_collapsed: bool,
    /// 新出现的工具步骤组默认展开（仅 `NW_EXPAND_STEPS=1` 时打开）。
    pub expand_steps_all: bool,
    /// 编辑器查找 / 替换状态。
    pub find: crate::find::FindState,
    /// 查找 / 替换输入框的缓冲区。
    pub find_query_buf: TextBuffer,
    pub find_replace_buf: TextBuffer,
    /// 已折叠的块（键 = 起始行号，0 基）。
    pub folds: std::collections::BTreeSet<usize>,
    /// 最近一次操作的结果提示（设置页内显示）。
    pub action_msg: Option<String>,
    /// 升级检查结果。
    pub update_status: Option<String>,
    /// 当前窗口的可用逻辑宽度（render 时写入，供网格类视图算列数）。
    pub layout_w: std::rc::Rc<std::cell::Cell<f32>>,
    /// 自动保存计时代数（每次改动 +1，用于判定"最后一次改动"）。
    pub auto_save_gen: std::rc::Rc<std::cell::Cell<u64>>,
    /// 是否有待保存的自动保存。
    pub auto_save_pending: bool,

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

/// 启动直达视图：`NW_VIEW=chat|team|file|skills|settings`。
fn initial_view() -> CenterView {
    match std::env::var("NW_VIEW").unwrap_or_default().to_lowercase().as_str() {
        "team" => CenterView::Team,
        "skills" => CenterView::Skills,
        "settings" => CenterView::Settings,
        _ => CenterView::Chat,
    }
}

/// `NW_SETTINGS_TAB=general|models|skills|sync|about`。
fn initial_settings_tab() -> SettingsTab {
    match std::env::var("NW_SETTINGS_TAB")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "models" => SettingsTab::Models,
        "skills" => SettingsTab::Skills,
        "sync" => SettingsTab::Sync,
        "about" => SettingsTab::About,
        _ => SettingsTab::General,
    }
}

/// 等待用户批准的写操作。
pub struct PendingApproval {
    pub session_idx: usize,
    /// 对应的会话消息下标（用于把按钮渲染到那张卡片上）。
    pub msg_idx: usize,
    pub tool: String,
    pub summary: String,
    /// 把用户决定送回 Agent 线程。
    pub respond: flume::Sender<bool>,
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
        let config = neverwork_core::config::load();
        let settings = neverwork_core::config::load_settings();
        // 主题立即生效，首次渲染就是用户上次选的颜色。
        neverwork_ui::theme::set_theme_mode(settings.theme);
        let ai_ready = settings.usable_provider_count() > 0
            || neverwork_agent::config_from_env().is_some();
        let sessions = data::demo_sessions();

        let mut me = Self {
            workspace: ws,
            root,
            files,
            expanded,
            recent: Vec::new(),
            center: initial_view(),
            settings_tab: initial_settings_tab(),
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
            chat_scroll: gpui::ScrollHandle::new(),
            chat_scroll_dirty: std::cell::Cell::new(false),
            settings,
            active_field: None,
            field_buf: TextBuffer::single_line(),
            skill_filter: None,
            skill_sort: 0,
            skills_tab: 0,
            skill_search: String::new(),
            import_path: String::new(),
            model_picker: None,
            model_query: String::new(),
            model_highlight: 0,
            export_with_secrets: false,
            tools_enabled: true,
            pending: None,
            auto_approve: false,
            expanded_steps: std::collections::HashSet::new(),
            right_collapsed: false,
            expand_steps_all: false,
            find: crate::find::FindState::default(),
            find_query_buf: TextBuffer::single_line(),
            find_replace_buf: TextBuffer::single_line(),
            folds: std::collections::BTreeSet::new(),
            action_msg: None,
            update_status: None,
            layout_w: std::rc::Rc::new(std::cell::Cell::new(1200.0)),
            auto_save_gen: std::rc::Rc::new(std::cell::Cell::new(0)),
            auto_save_pending: false,
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
        // 自动化验证用：`NW_SEND=<文本>` 启动后自动发送一条消息。
        let auto_send = std::env::var("NW_SEND").ok().filter(|t| !t.trim().is_empty());
        // 调试/截图用：启动即打开某个厂商的模型选择器。
        if let Ok(v) = std::env::var("NW_PICK_MODEL") {
            if let Ok(i) = v.parse::<usize>() {
                if i < me.settings.providers.len() {
                    me.model_picker = Some(i);
                    me.active_field = Some(FormField::ModelSearch);
                }
            }
        }
        // 验证用：展开所有工具步骤组 / 收起右栏 / 预置查找词，
        // 便于在无法点击的环境（高 DPI 远程桌面）下截图核对界面。
        if std::env::var("NW_EXPAND_STEPS").map(|v| v == "1").unwrap_or(false) {
            for (i, m) in me.sessions[me.active_session].messages.iter().enumerate() {
                if matches!(m.kind, MessageKind::ToolCall { .. }) {
                    me.expanded_steps.insert(i);
                }
            }
            me.expand_steps_all = true;
        }
        if std::env::var("NW_RIGHT_COLLAPSED").map(|v| v == "1").unwrap_or(false) {
            me.right_collapsed = true;
        }
        // 验证用：预置多光标（逗号分隔的字节偏移），便于截图确认渲染。
        if let Ok(v) = std::env::var("NW_CURSORS") {
            if me.open_path.is_some() {
                let mut first = true;
                for part in v.split(',').filter(|x| !x.trim().is_empty()) {
                    if let Ok(p) = part.trim().parse::<usize>() {
                        if first {
                            me.editor.move_to(p, false);
                            first = false;
                        } else {
                            me.editor.add_cursor(p);
                        }
                    }
                }
            }
        }
        // 验证用：在所有光标处插入同一段文本，确认多光标编辑生效。
        if let Ok(t) = std::env::var("NW_TYPE") {
            if me.open_path.is_some() && me.editor.has_multi_cursor() {
                me.editor.insert_multi(&t);
                me.dirty = true;
            }
        }
        // 验证用：折叠前 N 个可折叠块。
        if let Ok(n) = std::env::var("NW_FOLD") {
            if let Ok(n) = n.parse::<usize>() {
                if me.open_path.is_some() {
                    me.fold_all(cx);
                    let keep: std::collections::BTreeSet<usize> =
                        me.folds.iter().copied().take(n).collect();
                    me.folds = keep;
                }
            }
        }
        // 验证用：打开编辑器并预置查找词。
        if let Ok(q) = std::env::var("NW_FIND") {
            if me.open_path.is_some() {
                me.open_find(cx);
                me.set_find_query(q, cx);
            }
        }
        // 自动化验证用：`NW_AUTO_APPROVE=1` 时自动批准写操作（仅供本机验证，正常使用别开）。
        if std::env::var("NW_AUTO_APPROVE").map(|v| v == "1").unwrap_or(false) {
            me.auto_approve = true;
        }
        // 用 NW_VIEW 直达同步页时也自动查一次版本。
        if me.settings_tab == SettingsTab::Sync && me.center == CenterView::Settings {
            me.check_update(cx);
        }
        cx.notify();
        if let Some(t) = auto_send {
            me.new_session(cx);
            me.chat.reset(t);
            me.send_message(cx);
        }
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
        let indent = self.indent_str();
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
                        indented.push_str(&indent);
                    }
                    indented.push_str(line);
                }
                self.editor.move_to(sel_start, false);
                self.editor.move_to(sel_end, true);
                self.editor.insert(&indented);
            }
            None => {
                self.editor.insert(&indent);
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
            Command::Settings => self.open_settings(SettingsTab::General, cx),
            Command::Skills => self.open_skills(cx),
        }
    }

    // ===== 键盘 =====

    fn buffer(&self, target: InputTarget) -> &TextBuffer {
        match target {
            InputTarget::Chat => &self.chat,
            InputTarget::Editor => &self.editor,
            InputTarget::Workspace => &self.ws_input,
            InputTarget::Field(_) => &self.field_buf,
        }
    }

    fn buffer_mut(&mut self, target: InputTarget) -> &mut TextBuffer {
        match target {
            InputTarget::Chat => &mut self.chat,
            InputTarget::Editor => &mut self.editor,
            InputTarget::Workspace => &mut self.ws_input,
            InputTarget::Field(_) => &mut self.field_buf,
        }
    }

    fn note_edit(&mut self, target: InputTarget) {
        if target == InputTarget::Editor {
            self.dirty = true;
            self.schedule_auto_save();
            self.reconcile_folds();
        }
    }

    /// 自动保存：开启时每次改动重新计时 800ms，超时后写盘。
    /// 每次新改动会推进 `auto_save_gen`，只有最后一次计时器真正落盘。
    pub(crate) fn schedule_auto_save(&mut self) {
        if !self.settings.auto_save || self.open_path.is_none() {
            return;
        }
        let gen = self.auto_save_gen.get() + 1;
        self.auto_save_gen.set(gen);
        self.auto_save_pending = true;
    }

    /// 由渲染侧的定时器调用：若期间没有新改动则保存。
    pub(crate) fn flush_auto_save(&mut self, cx: &mut Context<Self>) {
        if !self.auto_save_pending || !self.dirty {
            return;
        }
        self.auto_save_pending = false;
        if let Some(path) = self.open_path.clone() {
            if crate::filesystem::write_file(&path, self.editor.text()).is_ok() {
                self.dirty = false;
            }
        }
        cx.notify();
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

    /// 表单字段的按键处理：只走非文本按键，文本由 IME 注入 `field_buf`。
    pub(crate) fn on_field_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(field) = self.active_field else {
            return;
        };
        self.input_target = InputTarget::Field(field);
        let Some(action) = keymap::route(&ev.keystroke) else {
            // 可打印字符：IME 会调用 replace_text_in_range 写入 field_buf，
            // 写完后触发 notify（见 input_handler）。
            return;
        };
        match action {
            KeyAction::Enter => self.commit_field(cx),
            KeyAction::Escape => {
                self.active_field = None;
                self.action_msg = None;
                cx.notify();
            }
            KeyAction::Tab { .. } => self.commit_field(cx),
            other => {
                let buf = &mut self.field_buf;
                match other {
                    KeyAction::Motion { motion, extend } => match motion {
                        Motion::Left => buf.move_left(extend),
                        Motion::Right => buf.move_right(extend),
                        Motion::WordLeft => buf.move_word_left(extend),
                        Motion::WordRight => buf.move_word_right(extend),
                        Motion::Home | Motion::DocStart => buf.move_home(extend),
                        Motion::End | Motion::DocEnd => buf.move_end(extend),
                        Motion::Up => buf.move_up(extend),
                        Motion::Down => buf.move_down(extend),
                    },
                    KeyAction::Backspace => buf.backspace(),
                    KeyAction::Delete => buf.delete_forward(),
                    KeyAction::SelectAll => buf.select_all(),
                    KeyAction::Undo => {
                        buf.undo();
                    }
                    KeyAction::Redo => {
                        buf.redo();
                    }
                    KeyAction::Copy => {
                        if let Some(t) = buf.selected_text().map(|s| s.to_string()) {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(t));
                        }
                    }
                    KeyAction::Cut => {
                        if let Some(t) = buf.take_selection() {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(t));
                        }
                    }
                    KeyAction::Paste => {
                        if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
                            buf.insert(&t);
                        }
                    }
                    KeyAction::Save => self.save_file(cx),
            KeyAction::Find => self.open_find(cx),
            KeyAction::FoldAll => self.fold_all(cx),
            KeyAction::SelectNextOccurrence => self.select_next_occurrence(cx),
            KeyAction::UnfoldAll => self.unfold_all(cx),
            KeyAction::Replace => {
                self.open_find(cx);
                self.find.replace_focused = true;
                cx.notify();
            }
                    _ => {}
                }
                // 实时同步到模型，让输入框内容立刻可见。
                let value = self.field_buf.text().to_string();
                self.apply_field_live(field, value, cx);
                cx.notify();
            }
        }
    }

    /// 字段编辑过程中直接写回（不走落盘，避免每次按键都写文件）。
    pub(crate) fn apply_field_live(
        &mut self,
        field: FormField,
        value: String,
        cx: &mut Context<Self>,
    ) {
        match field {
            FormField::SkillSearch => {
                self.skill_search = value;
            }
            FormField::ImportPath => {
                self.import_path = value;
            }
            _ => {}
        }
        cx.notify();
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
                if target == InputTarget::Editor && self.editor.has_multi_cursor() {
                    self.editor.backspace_multi();
                } else {
                    self.buffer_mut(target).backspace();
                }
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
            KeyAction::Find => self.open_find(cx),
            KeyAction::FoldAll => self.fold_all(cx),
            KeyAction::SelectNextOccurrence => self.select_next_occurrence(cx),
            KeyAction::UnfoldAll => self.unfold_all(cx),
            KeyAction::Replace => {
                self.open_find(cx);
                self.find.replace_focused = true;
                cx.notify();
            }
            KeyAction::Enter => match target {
                InputTarget::Chat => self.send_message(cx),
                InputTarget::Editor => {
                    self.editor.newline();
                    self.dirty = true;
                    self.scroll_editor_to_cursor();
                    cx.notify();
                }
                InputTarget::Workspace => self.apply_workspace_input(cx),
                InputTarget::Field(_) => self.commit_field(cx),
            },
            KeyAction::Tab { shift } => {
                if target == InputTarget::Editor {
                    if shift {
                        // Shift+Tab：按设置里的缩进宽度去掉行首空白。
                        let indent = self.indent_str();
                        let start = self.editor.line_start_of(self.editor.cursor());
                        let text = self.editor.text().to_string();
                        let rest = &text[start..];
                        let w = indent.len();
                        let strip = if w > 0 && rest.starts_with(&indent) {
                            w
                        } else {
                            // 容忍实际缩进少于设置宽度的情况
                            rest.chars().take_while(|c| *c == ' ').count().min(w)
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
                if self.model_picker.is_some() {
                    self.close_model_picker(cx);
                } else if self.ws_prompt_open {
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
        let Some((provider_id, config)) = self.active_provider_config() else {
            self.notice = Some(
                "还没有可用的模型：打开「设置 → 模型与厂商」填入 API Key 即可开始对话".into(),
            );
            cx.notify();
            return;
        };

        let team = self
            .config
            .active_team()
            .cloned()
            .unwrap_or_else(neverwork_core::Team::default_team);

        self.chat.reset("");
        self.notice = None;
        self.pending = None;
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
        self.chat_scroll_dirty.set(true);
        cx.notify();

        // 开启工具 → 团队成员**串行**带工具干活：前一个角色的产出作为后一个的上下文，
        //   写操作逐个弹审批（并发写同一文件只会互相覆盖，所以不做并行）。
        // 关闭工具 → 多角色团队按原编排轮流发言（无工具、纯文本）。
        if self.tools_enabled {
            let mut roles = team.roles.clone();
            if roles.is_empty() {
                roles.push(neverwork_core::Role::new("agent", "Assistant", "A"));
            }
            // 模型与厂商一律以设置里的当前选择为准：role.model 只是展示标签。
            self.spawn_agents_chain(session_idx, text, roles, provider_id, config, cx);
        } else {
            self.spawn_team(session_idx, text, team, provider_id, config, cx);
        }
    }

    /// 带工具的 Agent 链：按角色顺序串行执行，事件经 flume 回到 UI 线程。
    ///
    /// 每个角色一次完整的「模型 → 工具 → 回填」循环；某个角色的产出会作为下一个角色的
    /// 上下文，这样流水线（规划 → 编码 → 审查）在带工具时依然成立。
    fn spawn_agents_chain(
        &mut self,
        session_idx: usize,
        text: String,
        roles: Vec<neverwork_core::Role>,
        provider: String,
        config: neverwork_agent::ProviderConfig,
        cx: &mut Context<Self>,
    ) {
        let cancel = self.cancel.clone();
        let root = self.root.clone();
        // 线程里用一份，UI 侧留一份（角色名/颜色要用来渲染气泡）
        let roles_for_ui = roles.clone();
        // (角色下标, 事件)
        let (tx, rx) = flume::unbounded::<(usize, neverwork_agent::AgentEvent)>();

        std::thread::spawn(move || {
            // 跨角色累积的对话上下文
            let mut history: Vec<neverwork_agent::ChatMessage> = Vec::new();
            for (ri, role) in roles.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let mut collected = String::new();
                {
                    let mut emit = |ev: neverwork_agent::AgentEvent| {
                        if let neverwork_agent::AgentEvent::TextDelta(d) = &ev {
                            collected.push_str(d);
                        }
                        let _ = tx.send((ri, ev));
                    };
                    neverwork_agent::run_agent(
                        &provider,
                        config.clone(),
                        &role.prompt,
                        history.clone(),
                        &text,
                        root.clone(),
                        &mut emit,
                        cancel.clone(),
                    );
                }
                if !collected.trim().is_empty() {
                    // 下一个角色能看到前面角色说了什么
                    history.push(neverwork_agent::ChatMessage::assistant(format!(
                        "[{}] {}",
                        role.name, collected
                    )));
                }
            }
        });

        cx.spawn(async move |weak, async_cx| {
            // 每个角色当前正在追加文本的 Agent 消息下标
            let mut agent_msg: std::collections::HashMap<usize, usize> =
                std::collections::HashMap::new();
            // 每个角色当前正在进行的工具卡片下标
            let mut tool_msg: std::collections::HashMap<usize, usize> =
                std::collections::HashMap::new();

            loop {
                let ev = rx.recv_async().await;
                let Ok((ri, ev)) = ev else { break };

                match ev {
                    neverwork_agent::AgentEvent::TextDelta(d) => {
                        let participant = roles_for_ui.get(ri).map(|r| {
                            neverwork_core::Participant {
                                role_id: r.id.clone(),
                                role_name: r.name.clone(),
                                color_index: r.color_index,
                                initial: r.initial.clone(),
                            }
                        });
                        let _ = weak.update(async_cx, |this, cx| {
                            let Some(p) = participant else { return };
                            let idx = match agent_msg.get(&ri).copied() {
                                Some(i) => i,
                                None => {
                                    let i = this.sessions[session_idx].messages.len();
                                    this.sessions[session_idx].messages.push(Message {
                                        kind: MessageKind::Agent { participant: p },
                                        body: String::new(),
                                        code: None,
                                    });
                                    agent_msg.insert(ri, i);
                                    i
                                }
                            };
                            if let Some(m) = this.sessions[session_idx].messages.get_mut(idx) {
                                m.body.push_str(&d);
                            }
                            this.chat_scroll_dirty.set(true);
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::ToolStarted {
                        call,
                        requires_approval,
                        preview,
                    } => {
                        let _ = weak.update(async_cx, |this, cx| {
                            let idx = this.sessions[session_idx].messages.len();
                            this.sessions[session_idx].messages.push(Message {
                                kind: MessageKind::ToolCall {
                                    name: call.name.clone(),
                                    path: call.path_arg(),
                                    summary: call.summary(),
                                    status: if requires_approval {
                                        ToolRunStatus::Awaiting
                                    } else {
                                        ToolRunStatus::Running
                                    },
                                    output: String::new(),
                                    diff: preview
                                        .iter()
                                        .map(|d| neverwork_core::DiffLineView {
                                            marker: d.marker().to_string(),
                                            text: d.text.clone(),
                                        })
                                        .collect(),
                                    added: 0,
                                    removed: 0,
                                },
                                body: String::new(),
                                code: None,
                            });
                            tool_msg.insert(ri, idx);
                            this.chat_scroll_dirty.set(true);
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::NeedApproval {
                        call,
                        respond,
                        ..
                    } => {
                        let tool = call.name.clone();
                        let summary = call.summary();
                        let tmsg = tool_msg.get(&ri).copied();
                        let _ = weak.update(async_cx, |this, cx| {
                            if this.auto_approve {
                                let _ = respond.send(true);
                            } else {
                                this.pending = Some(crate::state::PendingApproval {
                                    session_idx,
                                    msg_idx: tmsg.unwrap_or(usize::MAX),
                                    tool,
                                    summary,
                                    respond,
                                });
                            }
                            this.chat_scroll_dirty.set(true);
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::ToolFinished { result } => {
                        let tmsg = tool_msg.get(&ri).copied();
                        let _ = weak.update(async_cx, |this, cx| {
                            if let Some(idx) = tmsg {
                                if let Some(m) = this.sessions[session_idx].messages.get_mut(idx) {
                                    if let MessageKind::ToolCall {
                                        status,
                                        output,
                                        diff,
                                        added,
                                        removed,
                                        ..
                                    } = &mut m.kind
                                    {
                                        *status = if result.ok {
                                            ToolRunStatus::Ok
                                        } else {
                                            ToolRunStatus::Failed
                                        };
                                        *output = result.content.clone();
                                        *added = result.added as u32;
                                        *removed = result.removed as u32;
                                        if !result.diff.is_empty() {
                                            *diff = result
                                                .diff
                                                .iter()
                                                .map(|d| neverwork_core::DiffLineView {
                                                    marker: d.marker().to_string(),
                                                    text: d.text.clone(),
                                                })
                                                .collect();
                                        }
                                    }
                                }
                            }
                            this.pending = None;
                            this.refresh_tree(cx);
                            this.chat_scroll_dirty.set(true);
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::Denied { .. } => {
                        let tmsg = tool_msg.get(&ri).copied();
                        let _ = weak.update(async_cx, |this, cx| {
                            if let Some(idx) = tmsg {
                                if let Some(m) = this.sessions[session_idx].messages.get_mut(idx) {
                                    if let MessageKind::ToolCall { status, .. } = &mut m.kind {
                                        *status = ToolRunStatus::Denied;
                                    }
                                }
                            }
                            this.pending = None;
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::RoundLimit => {
                        let _ = weak.update(async_cx, |this, cx| {
                            this.notice =
                                Some("某个角色已达到单轮工具调用上限（8 轮），已停下".into());
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::Error(e) => {
                        let _ = weak.update(async_cx, |this, cx| {
                            this.notice = Some(format!("Agent 错误：{e}"));
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::Done => {}
                }
            }

            let _ = weak.update(async_cx, |this, cx| {
                this.streaming = false;
                this.pending = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// 多角色团队：沿用原来的轮流发言实现。
    fn spawn_team(
        &mut self,
        session_idx: usize,
        text: String,
        team: neverwork_core::Team,
        _provider_id: String,
        config: neverwork_agent::ProviderConfig,
        cx: &mut Context<Self>,
    ) {
        let cancel = self.cancel.clone();
        let (tx, rx) = flume::unbounded::<neverwork_agent::TeamEvent>();

        std::thread::spawn(move || {
            let mut emit = |ev: neverwork_agent::TeamEvent| {
                let _ = tx.send(ev);
            };
            neverwork_agent::run_team(&team, &text, config, &mut emit, cancel);
        });

        cx.spawn(async move |weak, async_cx| {
            let mut role_msg: std::collections::HashMap<usize, usize> =
                std::collections::HashMap::new();
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
                            this.chat_scroll_dirty.set(true);
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
                            this.chat_scroll_dirty.set(true);
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

    /// 本次会话里 Agent 改动过的文件汇总（按路径合并）。
    pub(crate) fn session_changes(&self) -> Vec<neverwork_core::FileChange> {
        match self.sessions.get(self.active_session) {
            Some(session) => neverwork_core::aggregate_changes(&session.messages),
            None => Vec::new(),
        }
    }

    /// 展开 / 收起一个工具步骤组。
    pub(crate) fn toggle_steps(&mut self, key: usize, cx: &mut Context<Self>) {
        if !self.expanded_steps.remove(&key) {
            self.expanded_steps.insert(key);
        }
        cx.notify();
    }

    // ===== 多光标 =====

    /// 给一段字节区间补上「额外光标」（Alt+点击用）。
    pub(crate) fn add_cursor_at(&mut self, pos: usize, cx: &mut Context<Self>) {
        if self.editor.has_selection() {
            self.editor.clear_extra_cursors();
        }
        self.editor.add_cursor(pos);
        cx.notify();
    }

    pub(crate) fn clear_multi_cursor(&mut self, cx: &mut Context<Self>) {
        if self.editor.has_multi_cursor() {
            self.editor.clear_extra_cursors();
            cx.notify();
        }
    }

    /// Ctrl+D：把光标处的词连同「下一个相同词」一起选中。
    ///
    /// 第一次按下选当前词，之后每按一次再选下一个出现位置。
    pub(crate) fn select_next_occurrence(&mut self, cx: &mut Context<Self>) {
        let text = self.editor.text().to_string();

        // 已经有选区：把它当作要复制的词
        let word = match self.editor.selected_text() {
            Some(w) if !w.is_empty() && !w.contains('\n') => w.to_string(),
            _ => {
                // 没有选区：选中光标所在的词
                let cursor = self.editor.cursor();
                let (s, e) = crate::text_buffer::word_bounds(&text, cursor);
                if s == e {
                    return;
                }
                self.editor.move_to(s, false);
                self.editor.move_to(e, true);
                cx.notify();
                return;
            }
        };

        // 找当前选区之后的下一个同词
        let from = self.editor.selection().map(|r| r.end).unwrap_or(0);
        let Some(pos) = text[from..].find(&word).map(|i| from + i) else {
            self.notice = Some(format!("没有更多「{word}」"));
            cx.notify();
            return;
        };
        if let Some(sel) = self.editor.selection() {
            // 把已有选区的起点作为额外光标锚点，再扩出新区间
            let _ = sel;
        }
        self.editor.add_cursor(pos);
        self.editor.move_to(pos + word.len(), true);
        self.notice = Some(format!("已选中 {} 处「{word}」", self.editor.cursor_count()));
        cx.notify();
    }

    // ===== 代码折叠 =====

    /// 折叠 / 展开第 `line` 行开始的块。
    pub(crate) fn toggle_fold(&mut self, line: usize, cx: &mut Context<Self>) {
        if !self.folds.remove(&line) {
            self.folds.insert(line);
        }
        cx.notify();
    }

    /// 全部折叠（只折最外层）。
    pub(crate) fn fold_all(&mut self, cx: &mut Context<Self>) {
        let text = self.editor.text().to_string();
        let lines: Vec<&str> = text.split('\n').collect();
        let starts = crate::folding::foldable_starts(&lines);
        self.folds = starts.into_iter().collect();
        cx.notify();
    }

    pub(crate) fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.folds.clear();
        cx.notify();
    }

    /// 内容变化后清理失效的折叠点。
    pub(crate) fn reconcile_folds(&mut self) {
        if self.folds.is_empty() {
            return;
        }
        let text = self.editor.text().to_string();
        let lines: Vec<&str> = text.split('\n').collect();
        crate::folding::reconcile(&mut self.folds, &lines);
    }

    // ===== 查找 / 替换 =====

    /// 打开查找栏（Ctrl+F 或按钮）。带上当前选区作为初始查找词。
    pub(crate) fn open_find(&mut self, cx: &mut Context<Self>) {
        self.find.open = true;
        self.find.replace_focused = false;
        self.active_field = None;
        self.input_target = InputTarget::Field(FormField::FindQuery);
        if let Some(sel) = self.editor.selected_text() {
            if !sel.contains('\n') && sel.chars().count() <= 80 {
                self.find.query = sel.to_string();
            }
        }
        self.find.refresh(self.editor.text());
        self.focus_current_match();
        self.find_query_buf.reset(self.find.query.clone());
        self.find_replace_buf.reset(self.find.replace.clone());
        cx.notify();
    }

    /// 点进查找框 / 替换框。
    pub(crate) fn begin_find_field(&mut self, field: FormField, cx: &mut Context<Self>) {
        self.active_field = Some(field);
        self.input_target = InputTarget::Field(field);
        self.marked_range = None;
        match field {
            FormField::FindQuery => {
                self.find_query_buf.reset(self.find.query.clone());
                self.find_query_buf.move_doc_end(false);
            }
            FormField::FindReplace => {
                self.find.replace_focused = true;
                self.find_replace_buf.reset(self.find.replace.clone());
                self.find_replace_buf.move_doc_end(false);
            }
            _ => {}
        }
        cx.notify();
    }

    /// 查找框 / 替换框的键盘处理。
    pub(crate) fn on_find_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let shift = ev.keystroke.modifiers.shift;
        match key {
            "escape" => {
                self.close_find(cx);
                return;
            }
            "enter" => {
                if shift {
                    self.find_prev(cx);
                } else {
                    self.find_next(cx);
                }
                return;
            }
            _ => {}
        }

        let Some(field) = self.active_field else { return };
        let Some(action) = keymap::route(&ev.keystroke) else {
            // 可打印字符由 IME 写入 field_buf，写完后同步到 find 状态
            return;
        };
        let buf = match field {
            FormField::FindQuery => &mut self.find_query_buf,
            FormField::FindReplace => &mut self.find_replace_buf,
            _ => return,
        };
        match action {
            KeyAction::Motion { motion, extend } => match motion {
                Motion::Left => buf.move_left(extend),
                Motion::Right => buf.move_right(extend),
                Motion::Home | Motion::DocStart => buf.move_home(extend),
                Motion::End | Motion::DocEnd => buf.move_end(extend),
                _ => {}
            },
            KeyAction::Backspace => buf.backspace(),
            KeyAction::Delete => buf.delete_forward(),
            KeyAction::SelectAll => buf.select_all(),
            KeyAction::Undo => {
                buf.undo();
            }
            KeyAction::Paste => {
                if let Some(t) = cx.read_from_clipboard().and_then(|i| i.text()) {
                    buf.insert(&t);
                }
            }
            _ => {}
        }
        self.sync_find_bufs(field, cx);
    }

    /// 把输入框内容同步到查找状态。
    pub(crate) fn sync_find_bufs(&mut self, field: FormField, cx: &mut Context<Self>) {
        match field {
            FormField::FindQuery => {
                let q = self.find_query_buf.text().to_string();
                self.set_find_query(q, cx);
            }
            FormField::FindReplace => {
                self.find.replace = self.find_replace_buf.text().to_string();
                cx.notify();
            }
            _ => {}
        }
    }

    pub(crate) fn close_find(&mut self, cx: &mut Context<Self>) {
        self.find.open = false;
        self.find.matches.clear();
        cx.notify();
    }

    /// 跳到下一个 / 上一个命中，并把选区落上去。
    pub(crate) fn find_next(&mut self, cx: &mut Context<Self>) {
        self.find.next();
        self.focus_current_match();
        cx.notify();
    }

    pub(crate) fn find_prev(&mut self, cx: &mut Context<Self>) {
        self.find.prev();
        self.focus_current_match();
        cx.notify();
    }

    fn focus_current_match(&mut self) {
        if let Some(r) = self.find.current_range() {
            crate::find::select_match(&mut self.editor, &r);
            self.scroll_editor_to_cursor();
        }
    }

    /// 查找词变化后重算命中。
    pub(crate) fn set_find_query(&mut self, q: String, cx: &mut Context<Self>) {
        self.find.query = q;
        self.find.refresh(self.editor.text());
        self.focus_current_match();
        cx.notify();
    }

    pub(crate) fn replace_current_match(&mut self, cx: &mut Context<Self>) {
        if crate::find::replace_current(&mut self.editor, &mut self.find) {
            self.dirty = true;
            self.schedule_auto_save();
            self.focus_current_match();
        }
        cx.notify();
    }

    pub(crate) fn replace_all_matches(&mut self, cx: &mut Context<Self>) {
        let n = crate::find::replace_all(&mut self.editor, &mut self.find);
        if n > 0 {
            self.dirty = true;
            self.schedule_auto_save();
            self.notice = Some(format!("已替换 {n} 处"));
        } else {
            self.notice = Some("没有可替换的匹配".into());
        }
        cx.notify();
    }

    /// 收起 / 展开右侧栏。
    pub(crate) fn toggle_right_panel(&mut self, cx: &mut Context<Self>) {
        self.right_collapsed = !self.right_collapsed;
        cx.notify();
    }

    /// 用户对写操作的决定。
    pub(crate) fn answer_approval(&mut self, allow: bool, cx: &mut Context<Self>) {
        if let Some(p) = self.pending.take() {
            let _ = p.respond.send(allow);
            if !allow {
                if let Some(m) = self.sessions[p.session_idx].messages.get_mut(p.msg_idx) {
                    if let MessageKind::ToolCall { status, .. } = &mut m.kind {
                        *status = ToolRunStatus::Denied;
                    }
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn stop_streaming(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Relaxed);
        // 解开可能挂起的审批，让 Agent 线程立刻退出而不是等到超时。
        if let Some(p) = self.pending.take() {
            let _ = p.respond.send(false);
        }
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
        self.layout_w.set(visible_w);
        let hide_sides = self.focus_mode;
        // 设置页与技能市场是"全页"视图，会话列表和右栏都让出来。
        let full_page = matches!(self.center, CenterView::Settings | CenterView::Skills);
        let hide_right = visible_w < 1080.0 || full_page;
        let hide_list = visible_w < 620.0 || full_page;
        if std::env::var("NW_LAYOUT_DEBUG").is_ok() {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| {
                tracing::info!(
                    "layout: w={visible_w:.0} scale={:.1} center={:?} hide_sides={hide_sides} hide_right={hide_right} hide_list={hide_list}",
                    window.scale_factor(),
                    self.center
                );
            });
        }
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
