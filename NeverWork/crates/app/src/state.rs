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
    /// Git 面板：分支、改动文件、提交历史。
    Git,
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
    /// 给当前会话设置分类（参数是分类编号，见 `Command::all`）。
    TagSession(u8),
    /// 置顶 / 取消置顶当前会话。
    TogglePin,
    /// 在「单个助手」和「团队流水线」之间切换。
    ToggleAgentMode,
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
            (Command::Changes, "查看 Git 改动", ""),
            (Command::Preview, "预览当前文件", ""),
            (Command::SwitchWorkspace, "切换工作区…", ""),
            (Command::FocusMode, "专注模式（隐藏侧栏）", ""),
            // 当前会话的分类：直接列出所有分类，点了就设上（比在卡片上连点快）
            (Command::TogglePin, "置顶 / 取消置顶当前会话", ""),
            (Command::ToggleAgentMode, "切换：单个助手 / 团队流水线", ""),
            (Command::TagSession(0), "分类：清除标记", ""),
            (Command::TagSession(1), "分类：功能", ""),
            (Command::TagSession(2), "分类：修复", ""),
            (Command::TagSession(3), "分类：重构", ""),
            (Command::TagSession(4), "分类：文档", ""),
            (Command::TagSession(5), "分类：调试", ""),
            (Command::TagSession(6), "分类：实验", ""),
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
    /// Git 仓库状态（进入 Git 面板或手动刷新时更新）。
    pub git: crate::git::RepoStatus,
    /// Git 面板里当前选中的文件（用于显示 diff）。
    pub git_selected: Option<String>,
    /// 当前目录是否是一个 git 仓库（false 时显示提示而不是空列表）。
    pub git_is_repo: bool,
    /// 工作区里发现的所有 git 仓库：(显示名, 绝对路径)。
    ///
    /// 工作区常常是「容器目录」（如 `E:\go`），其下每个项目各自是仓库。
    /// Git 面板可以独立切换要操作的仓库，文件树仍显示整个工作区。
    pub git_repos: Vec<(String, PathBuf)>,
    /// 当前 Git 面板操作的仓库路径；`None` 表示工作区根本身。
    pub git_active: Option<PathBuf>,
    /// 提交信息输入框。
    pub git_commit_msg: TextBuffer,
    /// 分支操作输入框（新建 / 切换）。
    pub git_branch_input: TextBuffer,
    /// 等待二次确认的破坏性操作（丢弃改动）。
    pub git_confirm: Option<crate::git_ops::GitOp>,
    /// Git 面板是否展开分支列表。
    pub git_show_branches: bool,
    /// 会话列表的分类筛选；`None` 表示「全部」。
    pub tag_filter: Option<neverwork_core::SessionTag>,

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
    /// 本轮生成的开始时刻，用来在回答末尾显示耗时。
    pub turn_started: Option<std::time::Instant>,
    pub tools_enabled: bool,
    /// 是否走「多角色团队流水线」。
    ///
    /// 默认关闭 = 单个助手。之前默认就把每条输入都塞进
    /// Planner → Coder → Reviewer 三个角色串行处理，问一句「你是谁」
    /// 也要跑三个模型，而且每个角色的职责都写死成改代码，
    /// 于是问答类诉求必然答得别扭。
    pub team_mode: bool,
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

/// 决定启动时打开哪个工作区。
///
/// 优先级：`NW_ROOT` → 上次用过的（`settings.last_workspace`）→ 内置默认 `E:\go`
/// → 从当前目录向上找 `Cargo.toml` → 当前目录。
///
/// 内置默认是**别人机器上可能不存在**的绝对路径，所以每一步都要校验存在性再采用，
/// 否则会出现「打开一个空目录还以为 IDE 坏了」。
fn resolve_root(saved: &str) -> PathBuf {
    if let Ok(p) = std::env::var("NW_ROOT") {
        let path = PathBuf::from(p);
        if path.is_dir() {
            return path;
        }
    }
    if !saved.trim().is_empty() {
        let path = PathBuf::from(saved.trim());
        if path.is_dir() {
            return path;
        }
    }
    let default = PathBuf::from(neverwork_core::DEFAULT_WORKSPACE);
    if default.is_dir() {
        return default;
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
        let settings = neverwork_core::config::load_settings();
        let root = resolve_root(&settings.last_workspace);
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
            git: crate::git::RepoStatus::default(),
            git_selected: None,
            git_is_repo: false,
            git_repos: Vec::new(),
            git_active: None,
            git_commit_msg: TextBuffer::single_line(),
            git_branch_input: TextBuffer::single_line(),
            git_confirm: None,
            git_show_branches: false,
            tag_filter: None,
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
            turn_started: None,
            tools_enabled: true,
            team_mode: false,
            pending: None,
            auto_approve: false,
            expanded_steps: std::collections::HashSet::new(),
            // 右侧文件/Git 面板默认收起：把宽度让给对话区，
            // 需要时点一下窄条展开（`NW_RIGHT_COLLAPSED=0` 可强制展开用于验证）。
            right_collapsed: std::env::var("NW_RIGHT_COLLAPSED")
                .map(|v| v != "0")
                .unwrap_or(true),
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
        // 验证用：启动时执行一次 Git 写操作，走与界面完全相同的代码路径。
        if let Ok(spec) = std::env::var("NW_GIT_OP") {
            me.refresh_git(cx);
            // NW_GIT_REPO=<子目录名> 先切到该仓库
            if let Ok(name) = std::env::var("NW_GIT_REPO") {
                let p = me.root.join(name);
                me.git_active = Some(p);
                me.git_selected = None;
                me.refresh_git(cx);
            }
            let (kind, arg) = match spec.split_once(':') {
                Some((k, a)) => (k.to_string(), a.to_string()),
                None => (spec.clone(), String::new()),
            };
            match kind.as_str() {
                "stage-all" => me.run_git_op(crate::git_ops::GitOp::StageAll, cx),
                "commit" => me.run_git_op(crate::git_ops::GitOp::Commit(arg), cx),
                "discard" => me.run_git_op(crate::git_ops::GitOp::Discard(arg), cx),
                "branch" => me.run_git_op(crate::git_ops::GitOp::CreateBranch(arg), cx),
                "switch" => me.run_git_op(crate::git_ops::GitOp::SwitchBranch(arg), cx),
                _ => {}
            }
            // NW_GIT_CONFIRM=1 时把挂起的破坏性操作也执行掉（仅验证用）
            if std::env::var("NW_GIT_CONFIRM").map(|v| v == "1").unwrap_or(false)
                && me.git_confirm.is_some()
            {
                me.confirm_git_op(cx);
            }
            tracing::info!("NW_GIT_OP={spec} -> {:?}", me.notice);
        }
        // 验证用：直接打开右侧某个 Tab（files|git|preview）。
        if let Ok(t) = std::env::var("NW_RIGHT_TAB") {
            match t.to_lowercase().as_str() {
                "git" => me.set_right_tab(RightTab::Git, cx),
                "preview" => me.set_right_tab(RightTab::Preview, cx),
                "files" => me.set_right_tab(RightTab::Files, cx),
                _ => {}
            }
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
            tag: neverwork_core::SessionTag::None,
            updated_at: 0,
            pinned: false,
        });
        // 新会话的活跃时间设为当前，列表按它倒序时会自动排到最前
        if let Some(last) = self.sessions.last_mut() {
            last.touch();
        }
        self.active_session = self.sessions.len() - 1;
        self.center = CenterView::Chat;
        self.palette_open = false;
        cx.notify();
    }

    pub(crate) fn select_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.sessions.len() {
            self.active_session = idx;
            self.center = CenterView::Chat;
            // 注意：这里**不** touch。
            // 单纯「点开看一眼」不该把会话顶到最前，否则列表会随着浏览乱跳；
            // 只有真正发消息（send_message）才算活跃。
            cx.notify();
        }
    }

    /// 按「最近活跃优先」排序后的会话下标。
    pub(crate) fn ordered_sessions(&self) -> Vec<usize> {
        neverwork_core::sessions_by_recent(&self.sessions)
    }

    /// 给当前会话打分类标记（命令面板用，可直接跳到某个分类）。
    #[allow(dead_code)]
    pub(crate) fn set_session_tag(
        &mut self,
        idx: usize,
        tag: neverwork_core::SessionTag,
        cx: &mut Context<Self>,
    ) {
        if let Some(s) = self.sessions.get_mut(idx) {
            s.tag = tag;
            cx.notify();
        }
    }

    /// 在「单个助手」和「团队流水线」之间切换。
    pub(crate) fn toggle_agent_mode(&mut self, cx: &mut Context<Self>) {
        self.team_mode = !self.team_mode;
        self.notice = Some(if self.team_mode {
            "已切到团队流水线：多角色按顺序协作，适合分阶段的大任务".into()
        } else {
            "已切到单个助手：一个助手同时负责问答和改代码".into()
        });
        cx.notify();
    }

    /// 切换置顶。置顶的会话永远排在最前。
    pub(crate) fn toggle_pin(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(s) = self.sessions.get_mut(idx) {
            s.pinned = !s.pinned;
            let pinned = s.pinned;
            let title = s.title.clone();
            let short: String = title.chars().take(16).collect();
            self.notice = Some(if pinned {
                format!("已置顶「{short}」")
            } else {
                format!("已取消置顶「{short}」")
            });
            cx.notify();
        }
    }

    /// 点一下循环切换分类。
    pub(crate) fn cycle_session_tag(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(s) = self.sessions.get_mut(idx) {
            s.tag = s.tag.next();
            cx.notify();
        }
    }

    /// 当前分类筛选；`None` 表示全部。
    pub(crate) fn set_tag_filter(
        &mut self,
        tag: Option<neverwork_core::SessionTag>,
        cx: &mut Context<Self>,
    ) {
        self.tag_filter = tag;
        cx.notify();
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

    /// 当前 Git 面板操作的目录（选中的子仓库，或工作区根）。
    pub(crate) fn git_dir(&self) -> PathBuf {
        self.git_active.clone().unwrap_or_else(|| self.root.clone())
    }

    /// 重新发现工作区里的仓库。
    ///
    /// 规则：工作区根本身是仓库 → 只列它；否则列直接子目录里的仓库。
    /// 不递归深层找，避免在 node_modules 之类目录里翻出几百个仓库。
    fn discover_git_repos(&mut self) {
        let mut repos: Vec<(String, PathBuf)> = Vec::new();
        if crate::git::repo_root(&self.root).is_some() {
            repos.push((".".to_string(), self.root.clone()));
        } else {
            for name in crate::git::child_repos(&self.root) {
                repos.push((name.clone(), self.root.join(&name)));
            }
        }
        self.git_repos = repos;

        // 当前选中的仓库若已不存在就清掉
        if let Some(active) = &self.git_active {
            if !self.git_repos.iter().any(|(_, p)| p == active) {
                self.git_active = None;
            }
        }
        // **只有一个仓库时才自动选中**。
        // 多个仓库时保持未选中：暂存/提交是写操作，作用到用户没点选的仓库
        // 是不可接受的（曾经默认选第一个，导致 `git add -A` 加到别的项目里）。
        if self.git_active.is_none() && self.git_repos.len() == 1 {
            self.git_active = self.git_repos.first().map(|(_, p)| p.clone());
        }
    }

    /// 切换 Git 面板操作的仓库。
    pub(crate) fn select_git_repo(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.git_active = Some(path);
        self.git_selected = None;
        self.git_commit_msg.reset("");
        self.refresh_git(cx);
    }

    /// 刷新 Git 状态。
    pub(crate) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        self.discover_git_repos();
        let dir = self.git_dir();
        match crate::git::status(&dir) {
            crate::git::RepoQuery::Ok(st) => {
                self.git_is_repo = true;
                self.git = st;
                // 选中的文件若已不在改动列表里，清掉选择
                if let Some(sel) = &self.git_selected {
                    if !self.git.changes.iter().any(|c| &c.path == sel) {
                        self.git_selected = None;
                    }
                }
                if self.git_selected.is_none() {
                    self.git_selected = self.git.changes.first().map(|c| c.path.clone());
                }
            }
            crate::git::RepoQuery::NotARepo(msg) => {
                self.git_is_repo = false;
                self.git = crate::git::RepoStatus::default();
                self.git_selected = None;
                // 只有在「连子仓库都没有」时才提示不是仓库；
                // 多仓库工作区里这是正常状态，不该报错。
                if self.git_repos.is_empty() {
                    self.notice = Some(msg);
                }
            }
        }
        cx.notify();
    }

    /// 选中 Git 面板里的某个文件。
    pub(crate) fn select_git_file(&mut self, path: String, cx: &mut Context<Self>) {
        self.git_selected = Some(path);
        cx.notify();
    }

    /// 是否已明确选好要操作的仓库。
    ///
    /// 多仓库且未选中时为 `false`，此时禁止写操作——避免误改别的项目。
    pub(crate) fn git_repo_chosen(&self) -> bool {
        self.git_active.is_some() || self.git_repos.len() <= 1
    }

    /// 执行一次 Git 写操作。破坏性操作先挂起等确认。
    pub(crate) fn run_git_op(&mut self, op: crate::git_ops::GitOp, cx: &mut Context<Self>) {
        if !self.git_repo_chosen() {
            self.notice = Some("工作区内有多个仓库，先在 Git 面板选一个再操作".into());
            cx.notify();
            return;
        }
        // 破坏性操作必须二次确认，不能直接执行
        if op.is_destructive() {
            self.git_confirm = Some(op);
            cx.notify();
            return;
        }
        self.exec_git_op(op, cx);
    }

    /// 真正执行（已确认或非破坏性）。
    fn exec_git_op(&mut self, op: crate::git_ops::GitOp, cx: &mut Context<Self>) {
        let dir = self.git_dir();
        let is_commit = matches!(op, crate::git_ops::GitOp::Commit(_));

        let r = crate::git_ops::apply(&dir, &op);
        self.notice = Some(r.message.clone());

        if r.ok {
            if is_commit {
                self.git_commit_msg.reset("");
            }
            // 改动文件可能被删/改名，清掉失效的选中项
            self.git_selected = None;
            self.refresh_git(cx);
        } else {
            cx.notify();
        }
    }

    /// 用户确认了破坏性操作。
    pub(crate) fn confirm_git_op(&mut self, cx: &mut Context<Self>) {
        if let Some(op) = self.git_confirm.take() {
            self.exec_git_op(op, cx);
        }
        cx.notify();
    }

    pub(crate) fn cancel_git_op(&mut self, cx: &mut Context<Self>) {
        self.git_confirm = None;
        cx.notify();
    }

    /// 用输入框里的内容提交。
    pub(crate) fn commit_from_input(&mut self, cx: &mut Context<Self>) {
        let msg = self.git_commit_msg.text().to_string();
        if msg.trim().is_empty() {
            self.notice = Some("提交信息不能为空".into());
            cx.notify();
            return;
        }
        self.exec_git_op(crate::git_ops::GitOp::Commit(msg), cx);
    }

    /// 用输入框里的内容新建分支。
    pub(crate) fn create_branch_from_input(&mut self, cx: &mut Context<Self>) {
        let name = self.git_branch_input.text().to_string();
        if name.trim().is_empty() {
            self.notice = Some("分支名不能为空".into());
            cx.notify();
            return;
        }
        self.git_branch_input.reset("");
        self.exec_git_op(crate::git_ops::GitOp::CreateBranch(name), cx);
    }


    pub(crate) fn set_right_tab(&mut self, tab: RightTab, cx: &mut Context<Self>) {
        self.right_tab = tab;
        if tab == RightTab::Git {
            self.refresh_git(cx);
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

    /// 切换到一个已知存在的目录作为工作区（子仓库入口用）。
    pub(crate) fn switch_workspace(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() {
            self.notice = Some(format!("目录不存在：{}", path.display()));
            cx.notify();
            return;
        }
        self.root = path;
        self.settings.last_workspace = self.root.to_string_lossy().to_string();
        let _ = neverwork_core::config::save_settings(&self.settings);
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
        self.folds.clear();
        self.center = CenterView::Chat;
        self.notice = Some(format!("已切换到 {}", self.root.display()));
        self.refresh_git(cx);
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
        self.settings.last_workspace = self.root.to_string_lossy().to_string();
        let _ = neverwork_core::config::save_settings(&self.settings);
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
            Command::Changes => self.set_right_tab(RightTab::Git, cx),
            Command::Preview => self.set_right_tab(RightTab::Preview, cx),
            Command::SwitchWorkspace => self.open_workspace_prompt(cx),
            Command::FocusMode => {
                self.focus_mode = !self.focus_mode;
                cx.notify();
            }
            Command::ToggleAgentMode => self.toggle_agent_mode(cx),
            Command::TogglePin => {
                let idx = self.active_session;
                self.toggle_pin(idx, cx);
            }
            Command::TagSession(n) => {
                let tag = neverwork_core::SessionTag::all()
                    .get(n as usize)
                    .copied()
                    .unwrap_or(neverwork_core::SessionTag::None);
                let idx = self.active_session;
                self.set_session_tag(idx, tag, cx);
                self.notice = Some(if tag == neverwork_core::SessionTag::None {
                    "已清除分类标记".into()
                } else {
                    format!("已标记为「{}」", tag.label())
                });
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
                elapsed_ms: 0,
            });
            session.team_name = team.name.clone();
            if session.title.is_empty() || session.title == "新会话" {
                session.title = text.chars().take(16).collect();
            }
            // 真正产生内容才算活跃：刷新时间戳让它在列表里上浮
            session.touch();
        }

        self.streaming = true;
        self.turn_started = Some(std::time::Instant::now());
        self.cancel.store(false, Ordering::Relaxed);
        self.chat_scroll_dirty.set(true);
        cx.notify();

        // 三种走法，按「用户想要什么」分：
        //
        // 1. 默认（`team_mode = false`）：**一个**助手带工具。
        //    它能问答、能读文件、能改代码，由模型自己判断这次要不要动工具。
        //    绝大多数输入都走这里。
        // 2. 团队模式（`team_mode = true`）+ 开工具：多角色串行带工具，
        //    前一个角色的产出作为后一个的上下文，写操作逐个弹审批。
        //    适合「规划 → 编码 → 审查」这种明确的多阶段任务。
        // 3. 团队模式 + 关工具：多角色纯文本轮流发言。
        if self.team_mode && self.tools_enabled {
            let mut roles = team.roles.clone();
            if roles.is_empty() {
                roles.push(neverwork_core::Role::new("agent", "Assistant", "A"));
            }
            // 模型与厂商一律以设置里的当前选择为准：role.model 只是展示标签。
            self.spawn_agents_chain(session_idx, text, roles, provider_id, config, cx);
        } else if self.team_mode {
            self.spawn_team(session_idx, text, team, provider_id, config, cx);
        } else {
            // 单个助手：不套任何角色 prompt，避免「规划者 / 编码」这类
            // 与具体诉求无关的角色约束把问答带偏。
            let solo = neverwork_core::Role::new("agent", "Assistant", "A");
            self.spawn_agents_chain(session_idx, text, vec![solo], provider_id, config, cx);
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
        // 关工具时把 false 传下去，模型连工具清单都看不到。
        let with_tools = self.tools_enabled;
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
                        with_tools,
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
                                        elapsed_ms: 0,
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
                                elapsed_ms: 0,
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
                // 把本轮耗时写到最后一条回答上，渲染成 OpenCode 那样的页脚。
                if let Some(started) = this.turn_started.take() {
                    let ms = started.elapsed().as_millis() as u64;
                    if let Some(session) = this.sessions.get_mut(session_idx) {
                        if let Some(m) = session
                            .messages
                            .iter_mut()
                            .rev()
                            .find(|m| matches!(m.kind, MessageKind::Agent { .. }))
                        {
                            m.elapsed_ms = ms;
                        }
                    }
                }
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
                                        elapsed_ms: 0,
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
        // 立刻复位 streaming：只置取消标志的话，要等 Agent 线程跑完当前工具
        // 才收尾，界面上「生成中」和停止按钮会一直挂着（长工具调用可达几十秒），
        // 用户会以为卡死、也无法继续输入下一条。
        self.streaming = false;
        // 正在执行/待批准的卡片不会再返回结果，直接标成已取消，避免永远转圈。
        let idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        if let Some(session) = self.sessions.get_mut(idx) {
            for m in session.messages.iter_mut() {
                if let MessageKind::ToolCall { status, .. } = &mut m.kind {
                    if matches!(status, ToolRunStatus::Running | ToolRunStatus::Awaiting) {
                        *status = ToolRunStatus::Cancelled;
                    }
                }
            }
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
        // ---- 可见尺寸 ----
        // 坑：某些环境（远程桌面 / 特殊 DPI 缩放）下窗口按 1:1 物理像素创建，
        // 但 `bounds()` 返回的是物理像素，渲染却按 scale 放大 —— 于是内容被放大
        // 一倍，右侧和底部整块跑到窗口外。
        //
        // 判据：**窗口不可能比屏幕还宽**。当 `bounds().width > display.width` 时，
        // 说明两者单位不一致，`bounds()` 是物理值，需要除以 scale 才是逻辑尺寸。
        // 正常的 200% 缩放机器上 bounds 已是逻辑值（小于屏幕），不会触发这条分支。
        let scale = window.scale_factor().max(1.0);
        let raw_w: f32 = window.bounds().size.width.into();
        let raw_h: f32 = window.bounds().size.height.into();
        let screen_w: f32 = window
            .display(cx)
            .map(|d| d.bounds().size.width.into())
            .unwrap_or(raw_w);
        let mismatch = raw_w > screen_w * 1.02;
        let (visible_w, visible_h) = if mismatch {
            (raw_w / scale, raw_h / scale)
        } else {
            (raw_w, raw_h)
        };
        // 尺寸变了就再请求一帧。拖动窗口时 GPUI 在 Windows 的 modal resize
        // 循环里偶尔会漏掉一次重绘，导致内容停在旧尺寸（观感就是「不跟随」）。
        // 这里只在尺寸真的变化时补一帧，不会自激成循环。
        if (self.layout_w.get() - visible_w).abs() > 0.5 {
            cx.notify();
        }
        self.layout_w.set(visible_w);
        let hide_sides = self.focus_mode;
        // 设置页与技能市场是"全页"视图，会话列表和右栏都让出来。
        let full_page = matches!(
            self.center,
            CenterView::Settings | CenterView::Skills
        );
        let hide_right = visible_w < 1080.0 || full_page;
        let hide_list = visible_w < 620.0 || full_page;
        if std::env::var("NW_LAYOUT_DEBUG").is_ok() {
            // 尺寸变化时才打印。拖动窗口时能连续看到断点变化，便于确认跟随是否正常。
            static LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let key = (visible_w as u32) ^ ((visible_h as u32) << 16);
            if LAST.swap(key, std::sync::atomic::Ordering::Relaxed) != key {
                tracing::info!(
                    "layout: w={visible_w:.0} h={visible_h:.0} scale={:.1} hide_right={hide_right} hide_list={hide_list}",
                    window.scale_factor()
                );
            }
        }
        // 焦点始终跟着「当前输入目标」走。
        //
        // 之前只在启动时聚焦一次：之后任何一次点击落到没有 `track_focus` 的元素上
        // （文件树、会话卡片、标题栏、Git 面板…），焦点就被清空，
        // 输入桥也不再注册输入处理器 —— 键盘和输入法事件全部丢弃，
        // 表现就是「明明有光标，怎么打字都没反应」。
        // 现在每帧检查一次，掉焦就补回来；用户无需先点一下输入框。
        {
            let want = match self.input_target {
                InputTarget::Editor => Some(self.editor_focus.clone()),
                InputTarget::Workspace => Some(self.ws_focus.clone()),
                InputTarget::Chat | InputTarget::Field(_) => Some(self.focus_handle.clone()),
            };
            if let Some(handle) = want {
                if !handle.is_focused(window) {
                    // 渲染期间不能直接改焦点，推迟到本帧结束
                    window.defer(cx, move |window, _cx| {
                        window.focus(&handle);
                    });
                }
            }
        }

        let mut root = div()
            .relative()
            // 用 size_full() 铺满窗口，而不是写死 `px(visible_w)`。
            // 写死会有两个后果：拖动窗口时内容先停在旧尺寸（观感就是
            // 「界面不跟随」），以及尺寸读到中间值时比窗口小一圈留出黑边。
            // `visible_w` 只用来判断断点，不再用来定尺寸。
            .size_full()
            .overflow_hidden()
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
                    // 窄窗口不整块隐藏右栏，而是收成窄条：
                    // 否则用户再也点不回来（折叠按钮也一起消失）。
                    row.child(self.right_panel(hide_right, cx))
                }
            })
            .child(input_bridge(
                cx,
                self.input_target,
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
    target: InputTarget,
    chat_focus: FocusHandle,
    editor_focus: FocusHandle,
    ws_focus: FocusHandle,
) -> impl IntoElement {
    let entity = cx.entity();
    // 绑定哪个 handle 只看「当前输入目标」，不再靠 `is_focused` 三选一。
    //
    // 之前是按「谁此刻持有焦点」挑，一旦焦点掉到没有 handle 的元素上
    // （点一下文件树、会话卡片、标题栏），三个分支全不命中，`handle_input`
    // 就不会被注册 —— 键盘和输入法事件直接丢弃，表现就是「打字没反应」。
    // 现在输入目标由点击逻辑决定（`input_target`），焦点始终跟着它走。
    let handle = match target {
        InputTarget::Editor => editor_focus.clone(),
        InputTarget::Workspace => ws_focus.clone(),
        // 表单字段与对话共用同一个输入宿主
        InputTarget::Chat | InputTarget::Field(_) => chat_focus.clone(),
    };
    gpui::canvas(
        |_bounds, _window, _cx| {},
        move |bounds, _state, window, cx| {
            window.handle_input(
                &handle,
                ElementInputHandler::new(bounds, entity.clone()),
                cx,
            );
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
