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
    /// 上次会话落盘时间（流式期间节流用，避免每个 delta 都写盘）。
    sessions_saved_at: std::time::Instant,
    /// 验证用：本轮答完后自动追问的第二条（`NW_SEND2`）。
    /// 无人值守时用它确认模型真的看得到上一轮对话，而不是每次都重新开始。
    follow_up: Option<String>,

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
    /// 展开的交接卡片（键 = 消息下标），不与 `expanded_steps` 混用。
    pub expanded_handoff: std::collections::HashSet<usize>,
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
    /// 中栏（对话 / 编辑器）的实际可用宽度，render 时写入。
    ///
    /// 正文折行必须按这个值算：写死常量（如 `CHAT_MAX_W`）在三栏全开时
    /// 比可用宽度还宽，文字二次折行后右边被裁；右栏收起时又窄一截留白。
    pub chat_w: f32,
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
        "file" | "editor" => CenterView::File,
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

/// 「一轮模型回复对应一个消息气泡」的簿记。
///
/// 之前只用 `HashMap<角色, 当前气泡下标>`，工具调用结束后也不清空：第 2 轮的
/// 正文会继续追加到第 1 轮的气泡里 —— 最终回答被拼到工具步骤**上方**那段
/// 前言后面，读起来顺序全错。把规则收成这个小结构，才能单测。
#[derive(Default)]
pub(crate) struct RoundBubbles {
    current: std::collections::HashMap<usize, usize>,
}

impl RoundBubbles {
    /// 取本轮该写入的气泡下标；还没有就调 `create` 建一个。
    pub(crate) fn bubble(&mut self, role: usize, create: impl FnOnce() -> usize) -> usize {
        if let Some(i) = self.current.get(&role) {
            return *i;
        }
        let i = create();
        self.current.insert(role, i);
        i
    }

    /// 本轮正文结束（模型开始调工具）：下一段文字要落进新气泡。
    pub(crate) fn end_round(&mut self, role: usize) {
        self.current.remove(&role);
    }
}

/// 决定启动时打开哪个工作区。
///
/// 优先级：`NW_ROOT` → 上次用过的（`settings.last_workspace`）→ `~/.neverwork/workspace`。
///
/// 最后一级**不能**回落到「当前目录 / 向上找 `Cargo.toml`」：从 NeverWork 自己的
/// 源码目录启动时，那正好把 IDE 的开发仓库当成用户工作区，看到的是本项目代码，
/// 既不是用户选的也没有任何提示。改为自动创建的中立空目录。
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
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    let ws = PathBuf::from(home).join(".neverwork").join("workspace");
    let _ = std::fs::create_dir_all(&ws);
    ws
}

/// 造一个空白会话。
///
/// id 带时间戳：只用 `s{len+1}` 的话，删掉一个再新建就会撞号，
/// 而 id 是会话在磁盘上的唯一标识（`active_id` 靠它恢复）。
fn blank_session(seq: usize, config: &Config) -> Session {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Session {
        id: format!("s{now}-{seq}"),
        title: "新会话".into(),
        subtitle: "空白会话".into(),
        team_name: config
            .active_team()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "默认协作团队".into()),
        messages: Vec::new(),
        updated_label: "刚刚".into(),
        preview: "空白会话，输入第一条指令开始".into(),
        status: neverwork_core::SessionStatus::Idle,
        meta: String::new(),
        accent: seq % 6,
        icon: "＋".into(),
        badge: String::new(),
        action: neverwork_core::SessionAction::None,
        tag: neverwork_core::SessionTag::None,
        updated_at: now,
        pinned: false,
        stage: neverwork_core::TaskStage::Idle,
        resume_prompt: String::new(),
    }
}

/// 会话卡片第二行：把最后一条有内容的消息压成一行。
///
/// 工具卡片优先用「工具名 + 路径」，比正文更说明「刚才在干什么」。
fn last_preview(session: &Session) -> String {
    for m in session.messages.iter().rev() {
        if let MessageKind::ToolCall { name, path, .. } = &m.kind {
            return if path.is_empty() {
                name.clone()
            } else {
                format!("{name} {path}")
            };
        }
        if let Some(line) = m.body.lines().find(|l| !l.trim().is_empty()) {
            return line.trim().chars().take(28).collect();
        }
    }
    String::new()
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
        // MCP 工具在启动时配置一次。连接是惰性的（第一次真要用工具才起进程），
        // 没配 MCP 的用户零开销；改完配置需重启生效。
        let mcp_servers = neverwork_agent::mcp::servers_from_env()
            .unwrap_or_else(|| settings.mcp_servers.clone());
        neverwork_agent::mcp::configure(mcp_servers);
        let ai_ready = settings.usable_provider_count() > 0
            || neverwork_agent::config_from_env().is_some();
        // 会话来自磁盘，不再是演示数据：重启后要能看到上次聊到哪。
        // `load_with_recovery` 会把崩溃残留的 Running/AwaitingApproval 降级成
        // `Interrupted`，并告诉这里降级了几条、丢弃了几条。
        let (store, report) = neverwork_core::sessions::load_with_recovery(&root);
        let mut sessions = store.sessions;
        if sessions.is_empty() {
            sessions.push(blank_session(0, &config));
        }
        // 文件里存的「3分钟前」是写盘那一刻的快照，按真实时间戳重算：
        // 否则隔一周打开，列表还写着「3分钟前」。
        for s in &mut sessions {
            s.updated_label = neverwork_core::relative_label(s.updated_at);
        }
        // 恢复到上次激活的那个会话，而不是永远跳到第一条。
        let active_session = sessions
            .iter()
            .position(|s| s.id == store.active_id)
            .unwrap_or(0);

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
            active_session,
            // 减 5 秒：保证启动后第一次保存不会被节流档住
            sessions_saved_at: std::time::Instant::now()
                - std::time::Duration::from_secs(5),
            follow_up: std::env::var("NW_SEND2")
                .ok()
                .filter(|t| !t.trim().is_empty()),
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
            // 默认单助手；`NW_TEAM=1` 供自动化验收直接以团队模式启动。
            team_mode: std::env::var("NW_TEAM").map(|v| v == "1").unwrap_or(false),
            pending: None,
            auto_approve: false,
            expanded_steps: std::collections::HashSet::new(),
            expanded_handoff: std::collections::HashSet::new(),
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
            chat_w: 1200.0,
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
        // 上次退出时还在跑的任务：告诉用户可以续跑，
        // 否则只会觉得「任务凭空消失了」。
        if report.recovered > 0 {
            me.notice = Some(format!(
                "{} 个任务在退出时被中断，点会话卡片右上角的 ▶ 可从中断处继续",
                report.recovered
            ));
        }
        // 有记录解析不了（旧版本/新版本字段、手改坏了）：不让它们拖垮
        // 整个库，但要让用户知道丢过东西，而不是以为历史自己蒸发了。
        if report.dropped > 0 {
            me.notice = Some(format!(
                "{} 条会话记录已损坏被跳过，其余历史不受影响",
                report.dropped
            ));
        }
        cx.notify();
        if let Some(t) = auto_send {
            me.new_session(cx);
            me.chat.reset(t);
            me.send_message(cx);
        } else if std::env::var("NW_RESUME").is_ok() {
            // 验证用：启动后直接续跑第一条可续跑的会话（等同点击卡片右上角的 ▶）
            if let Some(i) = me
                .sessions
                .iter()
                .position(|s| s.stage.can_resume() && !s.resume_prompt.is_empty())
            {
                me.resume_session(i, cx);
            }
        }
        me
    }

    // ===== 会话 =====

    pub fn session(&self) -> &Session {
        &self.sessions[self.active_session.min(self.sessions.len().saturating_sub(1))]
    }

    /// 把会话写回磁盘。失败不中断对话，只提示一次。
    fn save_sessions(&mut self) {
        // `action` 只是 `stage` 的显示投影：每次落盘顺手对齐，
        // 避免两套状态各写各的（圆圈说「已完成」、实际还停在审批上）。
        for s in &mut self.sessions {
            s.sync_action();
        }
        let store = neverwork_core::SessionStore {
            active_id: self
                .sessions
                .get(self.active_session)
                .map(|s| s.id.clone())
                .unwrap_or_default(),
            sessions: self.sessions.clone(),
        };
        if let Err(e) = neverwork_core::sessions::save(&self.root, &store) {
            self.notice = Some(format!("会话保存失败：{e}"));
        }
        self.sessions_saved_at = std::time::Instant::now();
    }

    /// 流式输出时每个 delta 都写盘太重，这里节流 2 秒；
    /// 关键节点（发消息、工具结束、本轮结束、切会话）仍走 `save_sessions` 立即落盘。
    fn autosave_sessions(&mut self) {
        if self.sessions_saved_at.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        self.save_sessions();
    }

    pub(crate) fn new_session(&mut self, cx: &mut Context<Self>) {
        let s = blank_session(self.sessions.len(), &self.config);
        self.sessions.push(s);
        self.active_session = self.sessions.len() - 1;
        self.center = CenterView::Chat;
        self.palette_open = false;
        self.save_sessions();
        cx.notify();
    }

    pub(crate) fn select_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.sessions.len() {
            self.active_session = idx;
            self.center = CenterView::Chat;
            // 注意：这里**不** touch。
            // 单纯「点开看一眼」不该把会话顶到最前，否则列表会随着浏览乱跳；
            // 只有真正发消息（send_message）才算活跃。
            // 但要记住「上次看的是哪个」，重启后回到这里。
            self.save_sessions();
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
            self.save_sessions();
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
        self.send_text(text, None, cx);
    }

    /// 真正发一条指令。
    ///
    /// `send_message` 从输入框取文本，续跑（`resume_session`）直接调这里：
    /// 两条路径共用同一套记账、落盘与状态机迁移，不会一边有 stage 一边没有。
    ///
    /// `keep_prompt`：续跑时传原始任务，避免 `resume_prompt` 被「接着做」
    /// 提示覆盖——否则第二次中断就再也想不起原来要做什么了。
    fn send_text(&mut self, text: String, keep_prompt: Option<String>, cx: &mut Context<Self>) {
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
                tokens_in: 0,
                tokens_out: 0,
            });
            session.team_name = team.name.clone();
            if session.title.is_empty() || session.title == "新会话" {
                session.title = text.chars().take(16).collect();
                // 图标跟标题一起换成首字：列表里比「＋」好认
                session.icon = text
                    .chars()
                    .next()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "◈".into());
            }
            session.preview = text.chars().take(24).collect();
            session.status = neverwork_core::SessionStatus::Working;
            // 状态机：这一轮开始了。磁盘上留着 Running 就是崩溃过的证据。
            session.stage = neverwork_core::TaskStage::Running;
            session.resume_prompt = keep_prompt.unwrap_or_else(|| text.clone());
            session.sync_action();
            // 真正产生内容才算活跃：刷新时间戳让它在列表里上浮
            session.touch();
        }
        // 用户提问立刻落盘：即便下一秒崩了，已发出的指令也不会丢。
        self.save_sessions();

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
            match neverwork_core::parse_role_mention(&text, &roles) {
                // 点名角色：以它为入口，后续由 handoff 自动接力。
                Some(entry) => {
                    let mut team = team.clone();
                    if team.roles.is_empty() {
                        team.roles = roles.clone();
                    }
                    self.spawn_agents_chain(
                        session_idx,
                        text,
                        roles,
                        Some((team, entry)),
                        provider_id,
                        config,
                        cx,
                    );
                }
                // 没点名：保持原有「全员按顺序跑」的行为。
                None => self.spawn_agents_chain(
                    session_idx, text, roles, None, provider_id, config, cx,
                ),
            }
        } else if self.team_mode {
            // 关工具：@ 入口仍生效，但只跑被点名角色，不自动交接。
            match neverwork_core::parse_role_mention(&text, &team.roles) {
                Some(entry) => {
                    let role = team.roles[entry].clone();
                    self.spawn_agents_chain(
                        session_idx,
                        text,
                        vec![role],
                        None,
                        provider_id,
                        config,
                        cx,
                    );
                }
                None => self.spawn_team(session_idx, text, team, provider_id, config, cx),
            }
        } else {
            // 单个助手：不套任何角色 prompt，避免「规划者 / 编码」这类
            // 与具体诉求无关的角色约束把问答带偏。
            let solo = neverwork_core::Role::new("agent", "Assistant", "A");
            self.spawn_agents_chain(
                session_idx,
                text,
                vec![solo],
                None,
                provider_id,
                config,
                cx,
            );
        }
    }

    /// 续跑一条被中断 / 失败 / 已停止的会话。
    ///
    /// 不重发原始指令：历史里已经有一条同样的 user 消息，重发等于让模型
    /// 以为用户又说了一遍。改为发一条「接着做」的提示；已经写过的文件
    /// 靠工具层的幂等键（内容相同 → ok）保证重做不破坏。
    pub(crate) fn resume_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.sessions.len() {
            return;
        }
        if self.streaming {
            self.notice = Some("还在生成中，等这一轮结束再继续".into());
            cx.notify();
            return;
        }
        let stage = self.sessions[idx].stage;
        if !stage.can_resume() {
            self.notice = Some("这条会话没有需要续跑的进度".into());
            cx.notify();
            return;
        }
        let original = self.sessions[idx].resume_prompt.clone();
        self.active_session = idx;
        self.center = CenterView::Chat;
        self.notice = Some("正在从中断处继续…".into());
        let hint = neverwork_core::resume_instruction(&original);
        // 传 `Some(original)`：续跑期间再被中断，`resume_prompt` 仍是原始任务
        self.send_text(hint, Some(original), cx);
    }

    /// 带工具的 Agent 链：按角色顺序串行执行，事件经 flume 回到 UI 线程。
    ///
    /// 传 `handoff = Some((team, entry))` 时进入「@ 入口」模式：只从被点名
    /// 角色开始，由 `handoff` 工具决定下一跳；`None` 保持原有按 roles 顺序
    /// 全部跑一遍的行为。两条路径共用同一套 UI 事件泵。
    fn spawn_agents_chain(
        &mut self,
        session_idx: usize,
        text: String,
        roles: Vec<neverwork_core::Role>,
        handoff: Option<(neverwork_core::Team, usize)>,
        provider: String,
        config: neverwork_agent::ProviderConfig,
        cx: &mut Context<Self>,
    ) {
        // 关工具时把 false 传下去，模型连工具清单都看不到。
        let with_tools = self.tools_enabled;
        // 单次运行的硬上限（三项都为 0 = 不限），随设置走。
        let limits = neverwork_agent::RunLimits {
            tokens: self.settings.max_tokens_per_run,
            rounds: self.settings.max_rounds_per_run,
            seconds: self.settings.max_run_seconds,
        };
        let cancel = self.cancel.clone();
        let root = self.root.clone();
        // 线程里用一份，UI 侧留一份（角色名/颜色要用来渲染气泡）
        // 本会话之前聊过的内容。不喂给模型它就「失忆」：第二轮问
        // 「刚才让你读的是哪个文件」必然答不出，也看不出用户在追问。
        let seeded = history_for_model(&self.sessions[session_idx].messages, &text);
        let roles_for_ui = roles.clone();
        // (角色下标, 事件)
        let (tx, rx) = flume::unbounded::<(usize, neverwork_agent::AgentEvent)>();

        std::thread::spawn(move || {
            // 跨角色累积的对话上下文，起手带上会话历史
            let mut history: Vec<neverwork_agent::ChatMessage> = seeded;
            // 老路径：按 roles 顺序全部跑一遍；
            // @ 入口模式：从被点名角色开始，由 handoff 决定下一跳。
            let handoff_mode = handoff.is_some();
            let mut next: Option<usize> = handoff.as_ref().map(|(_, i)| *i);
            let mut seq = 0usize;
            let mut hops = 0usize;
            let mut input = text;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let ri = match next {
                    Some(i) => i,
                    None => {
                        if seq >= roles.len() {
                            break;
                        }
                        seq
                    }
                };
                let Some(role) = roles.get(ri) else { break };
                let mut collected = String::new();
                let mut hop: Option<neverwork_agent::HandoffPayload> = None;
                {
                    let mut emit = |ev: neverwork_agent::AgentEvent| {
                        if let neverwork_agent::AgentEvent::TextDelta(d) = &ev {
                            collected.push_str(d);
                        }
                        if let neverwork_agent::AgentEvent::HandoffRequest { payload, .. } = &ev {
                            hop = Some(payload.clone());
                        }
                        let _ = tx.send((ri, ev));
                    };
                    match &handoff {
                        // 团队交接链：用角色身份跑，工具清单里带 handoff。
                        Some((team, _)) => neverwork_agent::run_agent_in_team(
                            &provider,
                            config.clone(),
                            history.clone(),
                            &input,
                            root.clone(),
                            with_tools,
                            limits,
                            team,
                            ri,
                            &mut emit,
                            cancel.clone(),
                        ),
                        // 单 Agent / 团队顺序路径：不带 handoff 工具。
                        None => neverwork_agent::run_agent(
                            &provider,
                            config.clone(),
                            &role.prompt,
                            history.clone(),
                            &input,
                            root.clone(),
                            with_tools,
                            limits,
                            &mut emit,
                            cancel.clone(),
                        ),
                    }
                }
                if !collected.trim().is_empty() {
                    // 下一个角色能看到前面角色说了什么
                    history.push(neverwork_agent::ChatMessage::assistant(format!(
                        "[{}] {}",
                        role.name, collected
                    )));
                }
                match hop {
                    Some(payload) if handoff_mode => {
                        if !can_handoff(hops) {
                            let _ = tx.send((ri, neverwork_agent::AgentEvent::HandoffLimit));
                            break;
                        }
                        let Some((team, _)) = &handoff else { break };
                        let Some(target) = team.role_by_name(&payload.to) else {
                            break;
                        };
                        input = render_handoff(&role.name, &payload);
                        next = Some(target);
                        hops += 1;
                    }
                    _ => {
                        // 入口模式跑完即结束；老路径继续下一个角色。
                        if handoff_mode {
                            break;
                        }
                        seq += 1;
                        next = None;
                    }
                }
            }
        });

        cx.spawn(async move |weak, async_cx| {
            // 每一轮模型回复对应一个气泡：工具调用一结束就要换新的，
            // 否则下一轮正文会追加到上一轮那段前言里（顺序全错）。
            let mut bubbles = RoundBubbles::default();
            // 正在执行的工具卡片：**按调用 id** 记录，不能用角色作键。
            // 一轮里模型可能同时发起多个工具调用（如一次读 3 个文件），
            // 用角色作键会互相覆盖：结果全写到最后一个卡片，
            // 前面的永远停在「执行中」转圈。
            let mut tool_msg: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            // 本轮真实用量（Provider 报的输入/输出 token），收尾时记到消息上。
            let mut turn_usage = (0u64, 0u64);

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
                            let idx = bubbles.bubble(ri, || {
                                let i = this.sessions[session_idx].messages.len();
                                this.sessions[session_idx].messages.push(Message {
                                    kind: MessageKind::Agent { participant: p },
                                    body: String::new(),
                                    code: None,
                                    elapsed_ms: 0,
                                    tokens_in: 0,
                                    tokens_out: 0,
                                });
                                i
                            });
                            if let Some(m) = this.sessions[session_idx].messages.get_mut(idx) {
                                m.body.push_str(&d);
                            }
                            this.chat_scroll_dirty.set(true);
                            // 流式期间节流保存（2 秒），别每个 token 都写盘
                            this.autosave_sessions();
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::ToolStarted {
                        call,
                        requires_approval,
                        preview,
                    } => {
                        let _ = weak.update(async_cx, |this, cx| {
                            // 本轮正文到此为止：工具跑完后的那段回答要落到新气泡里
                            bubbles.end_round(ri);
                            let idx = this.sessions[session_idx].messages.len();
                            this.sessions[session_idx].messages.push(Message {
                                kind: MessageKind::ToolCall {
                                    name: call.name.clone(),
                                    // 存 id 与参数：下一轮追问时要把这次调用
                                    // 连结果一起回放给模型，否则它只能重读
                                    call_id: call.id.clone(),
                                    arguments: history_arguments(&call),
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
                                tokens_in: 0,
                                tokens_out: 0,
                            });
                            tool_msg.insert(call.id.clone(), idx);
                            this.chat_scroll_dirty.set(true);
                            // 工具节点是明确的阶段边界，立即落盘
                            this.save_sessions();
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
                        let tmsg = tool_msg.get(&call.id).copied();
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
                                // 卡在等人：磁盘上留 `AwaitingApproval`，
                                // 下次启动才会提示「这个任务被打断了」。
                                if let Some(s) = this.sessions.get_mut(session_idx) {
                                    s.stage = neverwork_core::TaskStage::AwaitingApproval;
                                    s.sync_action();
                                }
                                this.save_sessions();
                            }
                            this.chat_scroll_dirty.set(true);
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::ToolFinished { result } => {
                        // 用完就清：同一 id 不会再来第二次，留着只会让表无限增长
                        let tmsg = tool_msg.remove(&result.call_id);
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
                            // 审批/执行结束后继续跑：离开「等人」状态
                            if let Some(s) = this.sessions.get_mut(session_idx) {
                                if s.stage == neverwork_core::TaskStage::AwaitingApproval {
                                    s.stage = neverwork_core::TaskStage::Running;
                                    s.sync_action();
                                }
                            }
                            this.chat_scroll_dirty.set(true);
                            this.save_sessions();
                            cx.notify();
                        });
                    }
                    // 交接完成：落一张卡片，下一段角色正文会开新气泡。
                    neverwork_agent::AgentEvent::HandoffRequest { from, payload } => {
                        let _ = weak.update(async_cx, |this, cx| {
                            bubbles.end_round(ri);
                            let action = if payload.verdict == "needs_change" {
                                format!("需要修改 {} 处", payload.changes.len())
                            } else {
                                "验证通过".to_string()
                            };
                            let detail = render_handoff(&from, &payload);
                            this.sessions[session_idx].messages.push(Message {
                                kind: MessageKind::Handoff {
                                    from: from.clone(),
                                    to: payload.to.clone(),
                                    action,
                                    detail: detail.clone(),
                                },
                                // 历史回放用 body：下一轮追问时模型看得到
                                // 「上次谁把什么交给了谁、要求改哪些」。
                                body: detail,
                                code: None,
                                elapsed_ms: 0,
                                tokens_in: 0,
                                tokens_out: 0,
                            });
                            this.chat_scroll_dirty.set(true);
                            // 交接卡片是明确的阶段边界，立即落盘
                            this.save_sessions();
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::HandoffLimit => {
                        let _ = weak.update(async_cx, |this, cx| {
                            // 正常收尾，不标 Failed：链路已经跑完能跑的部分。
                            this.notice = Some(format!(
                                "已达交接上限（{} 跳），链路正常结束。",
                                neverwork_agent::MAX_HANDOFF_STEPS
                            ));
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::Denied { call } => {
                        let tmsg = tool_msg.remove(&call.id);
                        let _ = weak.update(async_cx, |this, cx| {
                            if let Some(idx) = tmsg {
                                if let Some(m) = this.sessions[session_idx].messages.get_mut(idx) {
                                    if let MessageKind::ToolCall { status, .. } = &mut m.kind {
                                        *status = ToolRunStatus::Denied;
                                    }
                                }
                            }
                            this.pending = None;
                            // 拒绝后 agent 会换个办法继续，不能停在「等人」
                            if let Some(s) = this.sessions.get_mut(session_idx) {
                                if s.stage == neverwork_core::TaskStage::AwaitingApproval {
                                    s.stage = neverwork_core::TaskStage::Running;
                                    s.sync_action();
                                }
                            }
                            this.save_sessions();
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::RoundLimit => {
                        let _ = weak.update(async_cx, |this, cx| {
                            // 不是错误：只是不再继续读，接下来会基于已读内容作答。
                            // 原文案「已达到上限（8 轮），已停下」看起来像崩了。
                            this.notice = Some(
                                "已达到本轮工具调用上限，不再继续读取，正在基于已有信息作答…"
                                    .into(),
                            );
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::TimeLimit => {
                        let _ = weak.update(async_cx, |this, cx| {
                            // 同样是「停下并收尾」，不是失败。
                            this.notice = Some(
                                "这次运行已到时间上限，不再继续调用工具，正在基于已读内容作答…"
                                    .into(),
                            );
                            cx.notify();
                        });
                    }
                    // 用量已由 agent 写进 trace（~/.neverwork/traces/）；
                    // 这里把它记到本轮回答上，页脚才看得到花了多少。
                    neverwork_agent::AgentEvent::Usage {
                        input_tokens,
                        output_tokens,
                        ..
                    } => {
                        turn_usage = (input_tokens, output_tokens);
                    }
                    // 上下文超预算，多一次轻量摘要调用；给用户一个提示，
                    // 否则看起来就是卡住了。
                    neverwork_agent::AgentEvent::Compacting => {
                        let _ = weak.update(async_cx, |this, cx| {
                            this.notice =
                                Some("上下文已超预算，正在把更早的对话压成摘要…".into());
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::CostLimit { used, limit } => {
                        let _ = weak.update(async_cx, |this, cx| {
                            // 和轮数/时间上限同性质：停下来收尾，不是报错。
                            // 带上真实数字，用户才知道是预算设小了还是任务真失控了。
                            this.notice = Some(format!(
                                "这次运行已用完 token 预算（{used} / {limit}），不再继续调用工具，正在基于已读内容作答…"
                            ));
                            cx.notify();
                        });
                    }
                    neverwork_agent::AgentEvent::Error(e) => {
                        // provider 的原始错误往往是一大段 JSON（如 context_length_exceeded），
                        // 直接甩给用户只会让人以为程序坏了。把最常见的几种翻成人话，
                        // 其余保留原文（排查时还需要它）。
                        let low = e.to_lowercase();
                        let msg = if low.contains("context_length")
                            || low.contains("maximum context length")
                            || low.contains("too many tokens")
                            || low.contains("context window")
                        {
                            "这次读到的内容超出了模型的上下文上限，已停下。可以新开一个会话，或直接说清要改哪个文件。"
                                .to_string()
                        } else if low.contains("invalid api key")
                            || low.contains("unauthorized")
                            || low.contains("incorrect api key")
                        {
                            "API Key 无效或已过期，去设置里检查一下。".to_string()
                        } else if low.contains("rate limit") || low.contains("429") {
                            "请求太频繁被限流了，稍等几秒再试。".to_string()
                        } else {
                            format!("Agent 错误：{e}")
                        };
                        let _ = weak.update(async_cx, |this, cx| {
                            this.notice = Some(msg);
                            // 真错误：记成 `Failed`，卡片给「可续跑」的圆圈。
                            // 收尾逻辑会避开 Failed，不会被后面的 Done 盖掉。
                            if let Some(s) = this.sessions.get_mut(session_idx) {
                                s.stage = neverwork_core::TaskStage::Failed;
                                s.sync_action();
                            }
                            cx.notify();
                        });
                    }
                    // 每个角色跑完都要收气泡：同一角色被交回时才能开新气泡。
                    neverwork_agent::AgentEvent::Done => {
                        bubbles.end_round(ri);
                    }
                }
            }

            let _ = weak.update(async_cx, |this, cx| {
                this.streaming = false;
                this.pending = None;
                // 先把取消标志取出来，后面要可变借用 sessions
                let cancelled = this.cancel.load(Ordering::Relaxed);
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
                            m.tokens_in = turn_usage.0;
                            m.tokens_out = turn_usage.1;
                        }
                        // 本轮收尾：状态与摘要归位，左侧卡片才反映真实进度
                        session.status = neverwork_core::SessionStatus::Done;
                        // `Failed` 是真实失败，不能被「正常收尾」覆盖；
                        // 用户点过停止则是 `Cancelled`（可续跑），其余算跑完。
                        if session.stage != neverwork_core::TaskStage::Failed {
                            session.stage = if cancelled {
                                neverwork_core::TaskStage::Cancelled
                            } else {
                                neverwork_core::TaskStage::Done
                            };
                        }
                        session.sync_action();
                        let preview = last_preview(session);
                        session.preview = preview;
                        session.touch();
                    }
                }
                // 本轮结束强制落盘：下次启动打开就是这个瞬间的内容
                this.save_sessions();
                // 验证用：本轮答完自动追问下一条（`NW_SEND2`）
                if let Some(q) = this.follow_up.take() {
                    this.chat.reset(q);
                    this.send_message(cx);
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
        // 纯文本多角色也一样会「失忆」：同一会话的前文要带上
        let seeded = history_for_model(&self.sessions[session_idx].messages, &text);
        let (tx, rx) = flume::unbounded::<neverwork_agent::TeamEvent>();

        std::thread::spawn(move || {
            let mut emit = |ev: neverwork_agent::TeamEvent| {
                let _ = tx.send(ev);
            };
            neverwork_agent::run_team_with_history(&team, seeded, &text, config, &mut emit, cancel);
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
                                        tokens_in: 0,
                                        tokens_out: 0,
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
                            this.autosave_sessions();
                            cx.notify();
                        });
                    }
                    Ok(neverwork_agent::TeamEvent::RoleError { error, .. }) => {
                        let _ = weak.update(async_cx, |this, cx| {
                            this.notice = Some(format!("Agent 错误：{error}"));
                            if let Some(s) = this.sessions.get_mut(session_idx) {
                                s.stage = neverwork_core::TaskStage::Failed;
                                s.sync_action();
                            }
                            this.save_sessions();
                            cx.notify();
                        });
                    }
                    Ok(neverwork_agent::TeamEvent::RoleDone { .. }) => {}
                    Ok(neverwork_agent::TeamEvent::AllDone) | Err(_) => {
                        let _ = weak.update(async_cx, |this, cx| {
                            this.streaming = false;
                            let cancelled = this.cancel.load(Ordering::Relaxed);
                            if let Some(session) = this.sessions.get_mut(session_idx) {
                                session.status = neverwork_core::SessionStatus::Done;
                                if session.stage != neverwork_core::TaskStage::Failed {
                                    session.stage = if cancelled {
                                        neverwork_core::TaskStage::Cancelled
                                    } else {
                                        neverwork_core::TaskStage::Done
                                    };
                                }
                                session.sync_action();
                                let preview = last_preview(session);
                                session.preview = preview;
                                session.touch();
                            }
                            this.save_sessions();
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
            // 允许后立刻回到「执行中」：否则卡片会一直停在「待确认」，按钮却已经
            // 消失，看起来像卡住了（大文件写入要等几秒）。
            if let Some(m) = self
                .sessions
                .get_mut(p.session_idx)
                .and_then(|s| s.messages.get_mut(p.msg_idx))
            {
                if let MessageKind::ToolCall { status, .. } = &mut m.kind {
                    *status = if allow {
                        ToolRunStatus::Running
                    } else {
                        ToolRunStatus::Denied
                    };
                }
            }
            // 保持展开：`awaiting` 是强制展开的条件，状态一回 `Running` 卡片就会
            // 收起，用户看不到随后的写入结果和 diff。
            self.expanded_steps.insert(p.msg_idx);
            // 决定已下：离开「等人」，回到执行中（下一次落盘才不会被当成崩溃）
            if let Some(s) = self.sessions.get_mut(p.session_idx) {
                if s.stage == neverwork_core::TaskStage::AwaitingApproval {
                    s.stage = neverwork_core::TaskStage::Running;
                    s.sync_action();
                }
            }
            self.save_sessions();
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
            // 用户主动停止：可续跑，但不是失败也不是完成
            session.stage = neverwork_core::TaskStage::Cancelled;
            session.sync_action();
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
        // 编辑器是「单文件」视图：会话列表让给正文，但右栏文件树留着
        // （切文件靠它，一起隐藏了就没法在这页换文件）。
        let hide_list = visible_w < 620.0
            || full_page
            || matches!(self.center, CenterView::File);
        // 中栏可用宽度：左栏（图标栏 + 会话列表）与右栏（面板或 16px 窄条）
        // 都是定宽，减掉就是正文真实可用的宽度（`chat_pane` 据此折行）。
        let left_w = if hide_sides {
            0.0
        } else if hide_list {
            crate::views::sidebar::RAIL_W
        } else {
            crate::views::sidebar::RAIL_W + crate::views::sidebar::SESSION_W
        };
        let right_w = if hide_sides {
            0.0
        } else if hide_right || self.right_collapsed {
            16.0
        } else {
            theme::FILES_W
        };
        self.chat_w = (visible_w - left_w - right_w).max(240.0);
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

/// 单个会话送进模型的历史条数上限。
///
/// 一条工具对（`assistant.tool_calls` + `tool`）至少占 2 条，
/// 「介绍这个项目」动辄读几十个文件，上限太小会在第一轮结束时就
/// 把观察全挤掉，模型在追问时只能把项目重读一遍。
const HISTORY_MAX_MSGS: usize = 90;
/// 历史正文的总字符预算。
///
/// 故意留得比 `agent::CONTEXT_BUDGET_CHARS`（160k）小：agent 侧还要加
/// system prompt 与本轮新产生的工具结果，历史把窗口吃满就等于逼它截新内容。
const HISTORY_BUDGET_CHARS: usize = 120_000;
/// 历史里单条工具结果保留的开头长度。
///
/// 超预算时先截到这里而不是整条丢：保住「哪个文件 + 它怎么开头」
/// 才能让模型判断要不要重读，否则它对上一轮的观察一无所知。
const HISTORY_TOOL_HEAD: usize = 3_000;
/// 截断后补在最前面的占位，告诉模型「上文不完整」。
const HISTORY_OMITTED: &str = "（更早的对话已省略）";
/// 存进历史的参数上限。超过就退化成只有 path：`write_file` 的参数
/// 含整份文件内容，原样存一遍会让会话文件翻倍，而那些内容已经在磁盘上。
const ARGS_KEEP_CHARS: usize = 4_000;

/// 回放历史时记进 `assistant.tool_calls` 的参数。
fn history_arguments(call: &neverwork_agent::ToolCall) -> String {
    let raw = call.arguments.trim();
    if raw.chars().count() <= ARGS_KEEP_CHARS {
        return raw.to_string();
    }
    let path = call.path_arg();
    if path.is_empty() {
        "{}".into()
    } else {
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        format!(r#"{{"path":"{escaped}"}}"#)
    }
}

/// 工具结果为空时的占位。
///
/// `tool` 消息不能是空内容（部分 provider 直接报错），更不能让模型
/// 以为自己拿到了结果——拒绝、失败、中断各自要说清楚。
fn tool_placeholder(status: neverwork_core::ToolRunStatus) -> &'static str {
    use neverwork_core::ToolRunStatus as S;
    match status {
        S::Denied => "[用户拒绝了这次调用，没有执行]",
        S::Failed => "[调用失败，没有输出]",
        S::Cancelled => "[用户中断了生成，这次调用没有结果]",
        _ => "[没有输出]",
    }
}

/// 把连续的工具卡片压成一条摘要。
///
/// 只给**旧记录**用：本功能上线前写下的卡片没有 `call_id`，配不成
/// `assistant.tool_calls` + `tool` 对，硬凑就是伪造协议结构（整轮 400）。
/// 新记录走 `emit_tool_block`，带真实输出回放。
///
/// 文案写成「会话记录」而不是第一人称：这条以 assistant 承载只是为了协议上安全
/// （工具消息必须有 `call_id` 配对），模型不该把它当成自己说过的话去引用。
fn tool_digest(cards: &[&neverwork_core::Message]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    // cards 是倒序攒的，反向走才是实际发生顺序
    for m in cards.iter().rev() {
        let MessageKind::ToolCall {
            name,
            path,
            status,
            output,
            ..
        } = &m.kind
        else {
            continue;
        };
        let verdict = match status {
            ToolRunStatus::Ok => format!("成功，返回 {} 字符", output.chars().count()),
            ToolRunStatus::Denied => "被用户拒绝".to_string(),
            ToolRunStatus::Failed => "失败".to_string(),
            // 没跑完的调用不构成「做过的事」
            _ => continue,
        };
        let target = if path.is_empty() {
            String::new()
        } else {
            format!(" {path}")
        };
        parts.push(format!("{name}{target} → {verdict}"));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("（会话记录 · 上一轮调用过工具：{}）", parts.join("；")))
}

/// 历史里的一块。
///
/// 工具卡片必须成块处理：`assistant.tool_calls` 与它的 `tool` 结果
/// 同生同死——少一条配对，OpenAI 协议下整轮 400。
enum HistoryBlock<'a> {
    /// 一条正文（用户提问 / 助手回答）。不截断：截了就是伪造。
    Text(&'a neverwork_core::Message),
    /// 一段连续的工具卡片。
    Tools {
        cards: Vec<&'a neverwork_core::Message>,
        /// 预算不够时只回放每条结果的开头，而不是整块丢掉。
        head_only: bool,
    },
}

impl HistoryBlock<'_> {
    /// 全文成本（字符）。
    fn chars(&self) -> usize {
        match self {
            Self::Text(m) => m.body.trim().chars().count(),
            Self::Tools { cards, head_only } => tool_block_chars(cards, *head_only),
        }
    }

    /// 占用的模型消息条数（一块工具是 1 条 `assistant.tool_calls` + N 条结果）。
    fn msgs(&self) -> usize {
        match self {
            Self::Text(_) => 1,
            Self::Tools { cards, .. } => 1 + cards.len(),
        }
    }
}

/// 一块工具卡片在历史里的字符成本。
fn tool_block_chars(cards: &[&neverwork_core::Message], head_only: bool) -> usize {
    cards
        .iter()
        .map(|m| match &m.kind {
            MessageKind::ToolCall { output, .. } => {
                let body = output.trim();
                if head_only {
                    keep_head(body, HISTORY_TOOL_HEAD).chars().count()
                } else {
                    body.chars().count()
                }
            }
            _ => 0,
        })
        .sum()
}

/// 把一块工具结果截到开头，返回省下的字符数。
fn trim_tool_block(b: &mut HistoryBlock) -> usize {
    let HistoryBlock::Tools { cards, head_only } = b else {
        return 0;
    };
    if *head_only {
        return 0;
    }
    let before = tool_block_chars(cards, false);
    *head_only = true;
    before.saturating_sub(tool_block_chars(cards, true))
}

/// 一段连续工具卡片 → `assistant.tool_calls` + 若干 `tool` 结果。
///
/// 这是「第二轮不再重读项目」的关键：把上一轮真实读到的内容带回去，
/// 模型只需要看摘要里没有的细节，而不是把整个项目重新读一遍。
fn emit_tool_block(
    seg: &[&neverwork_core::Message],
    head_only: bool,
    out: &mut Vec<neverwork_agent::ChatMessage>,
) {
    /// 把攒下的调用一次性发出去，保持与 tool 结果的顺序一致
    fn flush(
        calls: &mut Vec<neverwork_agent::ToolCall>,
        results: &mut Vec<neverwork_agent::ChatMessage>,
        out: &mut Vec<neverwork_agent::ChatMessage>,
    ) {
        if calls.is_empty() {
            return;
        }
        out.push(neverwork_agent::ChatMessage::assistant_with_tools(
            "",
            std::mem::take(calls),
        ));
        out.append(results);
        results.clear();
    }

    let mut calls: Vec<neverwork_agent::ToolCall> = Vec::new();
    let mut results: Vec<neverwork_agent::ChatMessage> = Vec::new();
    // 本功能上线前写的记录没有 call_id，配不成对，只能退化成摘要
    let mut legacy: Vec<&neverwork_core::Message> = Vec::new();
    for m in seg {
        let MessageKind::ToolCall {
            name,
            call_id,
            arguments,
            status,
            output,
            ..
        } = &m.kind
        else {
            continue;
        };
        // 没跑完的调用不构成观察（中断留下的 Running、等审批的 Awaiting）：
        // 回放成「已执行」就是伪造，模型会拿它当既成事实往下推。
        if status.is_pending() {
            continue;
        }
        if call_id.is_empty() {
            flush(&mut calls, &mut results, out);
            legacy.push(m);
            continue;
        }
        if !legacy.is_empty() {
            if let Some(d) = tool_digest(&legacy) {
                out.push(neverwork_agent::ChatMessage::assistant(d));
            }
            legacy.clear();
        }
        calls.push(neverwork_agent::ToolCall {
            id: call_id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
        });
        let body = output.trim();
        let text = if body.is_empty() {
            tool_placeholder(*status).to_string()
        } else if head_only && body.chars().count() > HISTORY_TOOL_HEAD {
            format!(
                "{}\n…（已截断，原文 {} 字符；需要细节请重新读取）",
                keep_head(body, HISTORY_TOOL_HEAD),
                body.chars().count()
            )
        } else {
            output.clone()
        };
        results.push(neverwork_agent::ChatMessage::tool_result(
            call_id.clone(),
            text,
        ));
    }
    if !legacy.is_empty() {
        if let Some(d) = tool_digest(&legacy) {
            out.push(neverwork_agent::ChatMessage::assistant(d));
        }
    }
    flush(&mut calls, &mut results, out);
}

/// 取前 `max` 个字符（按 UTF-8 边界切，不会 panic）。
fn keep_head(s: &str, max: usize) -> &str {
    if s.chars().count() <= max {
        return s;
    }
    let end = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
    &s[..end]
}

/// 还能不能再交接一跳（上限 6 跳，不含入口角色的首次运行）。
fn can_handoff(hops: usize) -> bool {
    hops < neverwork_agent::MAX_HANDOFF_STEPS
}

/// 把交接载荷渲染成目标角色读得懂的指令。
///
/// 目标角色拿到的是一段 user 文本，所以必须自带上下文：
/// 谁交过来的、结论是什么、具体改哪里、怎么算改完。
fn render_handoff(from: &str, p: &neverwork_agent::HandoffPayload) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "【团队交接】「{from}」把任务交给了你（「{}」）。\n\n",
        p.to
    ));
    s.push_str(&format!("结论：{}\n", p.summary));
    s.push_str(&format!(
        "验证结论：{}\n",
        if p.verdict == "pass" {
            "通过（pass）"
        } else {
            "需要修改（needs_change）"
        }
    ));
    if !p.changes.is_empty() {
        s.push_str("\n需要修改的点：\n");
        for (i, c) in p.changes.iter().enumerate() {
            let file = c.file.trim();
            let loc = c.location.trim();
            let where_ = match (file.is_empty(), loc.is_empty()) {
                (false, false) => format!("{file}（{loc}）"),
                (false, true) => file.to_string(),
                (true, false) => loc.to_string(),
                (true, true) => "（未指定）".to_string(),
            };
            s.push_str(&format!(
                "{}. {} — 问题：{}；要求：{}\n",
                i + 1,
                where_,
                c.issue.trim(),
                c.required.trim()
            ));
        }
    }
    if !p.acceptance.trim().is_empty() {
        s.push_str(&format!("\n验收标准：{}\n", p.acceptance.trim()));
    }
    s.push_str(
        "\n请按上面的要求继续；需要写文件时照常使用工具（会弹给用户批准）。完成后直接给出结论。",
    );
    s
}

/// 把会话里已经聊过的内容整理成喂给模型的历史。
///
/// 正文（用户 / 助手）原样取用；工具卡片**连同真实输出一起回放**成
/// `assistant.tool_calls` + `tool` 消息对——模型追问时能直接看到上一轮
/// 读到了什么，不必把整个项目重读一遍。真实踩过：先让它介绍 `E:\go\nas-os`，
/// 再问 btrfs，它手里只剩「读过哪些文件」的目录，于是又从零读了一遍。
///
/// 工具卡片必须整块进、整块出：`assistant.tool_calls` 与它的 `tool`
/// 结果少一条配对，OpenAI 协议下整轮 400。
///
/// `current` 是本次提问：`send_message` 已经把它 push 进列表，但它会作为
/// `input` 单独传给 `run_agent`，这里要跳过，否则模型看到同一句话两遍。
fn history_for_model(
    messages: &[neverwork_core::Message],
    current: &str,
) -> Vec<neverwork_agent::ChatMessage> {
    // ---- 1. 正序分组：正文各成一块，连续工具卡片合成一块 ----
    let mut blocks: Vec<HistoryBlock> = Vec::new();
    for m in messages {
        match &m.kind {
            MessageKind::ToolCall { status, .. } if !status.is_pending() => {
                match blocks.last_mut() {
                    Some(HistoryBlock::Tools { cards, .. }) => cards.push(m),
                    _ => blocks.push(HistoryBlock::Tools {
                        cards: vec![m],
                        head_only: false,
                    }),
                }
            }
            MessageKind::User | MessageKind::Agent { .. } | MessageKind::Handoff { .. }
                if !m.body.trim().is_empty() =>
            {
                blocks.push(HistoryBlock::Text(m));
            }
            _ => {}
        }
    }

    // ---- 2. 末尾那条就是本次提问才跳过（body 对不上就当普通历史） ----
    if let Some(HistoryBlock::Text(m)) = blocks.last() {
        if matches!(&m.kind, MessageKind::User) && m.body.trim() == current.trim() {
            blocks.pop();
        }
    }

    // ---- 3. 从最近往回挑，先用条数框住 ----
    // 这里不判字符预算：一次 read_file 就能返回几十万字符，按字符挑会把
    // 「读完这个项目」整块丢掉，而那正是追问时最需要的东西。字符交给第 4 步。
    let mut picked: Vec<HistoryBlock> = Vec::new();
    let mut count = 0usize;
    let mut truncated = false;
    for b in blocks.into_iter().rev() {
        let n = b.msgs();
        // 一块自身就超上限（一轮读了几十个文件）也收：它是「读过什么」的
        // 唯一线索，丢了这个块模型就只剩重读一条路。
        if !picked.is_empty() && n <= HISTORY_MAX_MSGS && count + n > HISTORY_MAX_MSGS {
            truncated = true;
            break;
        }
        count += n;
        picked.push(b);
    }
    picked.reverse();

    // ---- 4. 压字符预算 ----
    let mut used: usize = picked.iter().map(|b| b.chars()).sum();
    if used > HISTORY_BUDGET_CHARS {
        truncated = true;
        // 4a. 先截工具结果的开头，最新的那块保持原样（当前推理正靠它）
        let tool_idxs: Vec<usize> = picked
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b, HistoryBlock::Tools { head_only: false, .. }))
            .map(|(i, _)| i)
            .collect();
        for &i in tool_idxs.iter().rev().skip(1) {
            if used <= HISTORY_BUDGET_CHARS {
                break;
            }
            used -= trim_tool_block(&mut picked[i]);
        }
        // 4b. 还不够：最新的也截
        for &i in tool_idxs.iter().rev() {
            if used <= HISTORY_BUDGET_CHARS {
                break;
            }
            used -= trim_tool_block(&mut picked[i]);
        }
        // 4c. 正文本身太长截不了：先丢更早的工具块（内容已被后面的回答消化），
        //     工具块丢完还超才丢更早的正文，且至少留最后一条。
        let mut i = 0;
        while used > HISTORY_BUDGET_CHARS && i < picked.len() {
            if matches!(picked[i], HistoryBlock::Tools { .. }) {
                used = used.saturating_sub(picked.remove(i).chars());
            } else {
                i += 1;
            }
        }
        while used > HISTORY_BUDGET_CHARS && picked.len() > 1 {
            used = used.saturating_sub(picked.remove(0).chars());
        }
    }

    // ---- 5. 正序发出 ----
    let mut out: Vec<neverwork_agent::ChatMessage> = Vec::with_capacity(count + 1);
    for b in &picked {
        match b {
            HistoryBlock::Text(m) => {
                let body = m.body.trim();
                out.push(match &m.kind {
                    MessageKind::User => neverwork_agent::ChatMessage::user(body),
                    _ => neverwork_agent::ChatMessage::assistant(body),
                });
            }
            HistoryBlock::Tools { cards, head_only } => {
                emit_tool_block(cards, *head_only, &mut out)
            }
        }
    }
    if truncated {
        out.insert(0, neverwork_agent::ChatMessage::assistant(HISTORY_OMITTED));
    }
    out
}

#[cfg(test)]
mod handoff_tests {
    use super::{can_handoff, history_for_model, render_handoff};
    use neverwork_core::{Message, MessageKind, Participant, Role, Team};

    fn payload() -> neverwork_agent::HandoffPayload {
        neverwork_agent::HandoffPayload {
            to: "接口成员".into(),
            verdict: "needs_change".into(),
            summary: "需要补一个空值判断".into(),
            changes: vec![neverwork_agent::HandoffChange {
                file: "src/api.rs".into(),
                location: "handle".into(),
                issue: "未判空".into(),
                required: "加 if null 返回 400".into(),
            }],
            acceptance: "单测覆盖 null".into(),
        }
    }

    #[test]
    fn render_handoff_contains_everything_target_needs() {
        let text = render_handoff("需求验证", &payload());
        assert!(text.contains("需求验证"), "要说明是谁交过来的");
        assert!(text.contains("接口成员"));
        assert!(text.contains("需要补一个空值判断"));
        assert!(text.contains("src/api.rs（handle）"), "文件与位置要能直接定位");
        assert!(text.contains("未判空"));
        assert!(text.contains("加 if null 返回 400"));
        assert!(text.contains("单测覆盖 null"));
    }

    fn handoff_msg(detail: &str) -> Message {
        Message {
            kind: MessageKind::Handoff {
                from: "需求验证".into(),
                to: "接口成员".into(),
                action: "需要修改 1 处".into(),
                detail: detail.into(),
            },
            body: detail.into(),
            code: None,
            elapsed_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn agent(body: &str) -> Message {
        Message {
            kind: MessageKind::Agent {
                participant: Participant {
                    role_id: "b".into(),
                    role_name: "接口成员".into(),
                    color_index: 1,
                    initial: "接".into(),
                },
            },
            body: body.into(),
            code: None,
            elapsed_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    #[test]
    fn handoff_detail_is_replayed_to_the_model() {
        let msgs = vec![
            Message {
                kind: MessageKind::User,
                body: "验证需求".into(),
                code: None,
                elapsed_ms: 0,
                tokens_in: 0,
                tokens_out: 0,
            },
            handoff_msg("【团队交接】需求验证把任务交给了接口成员\n需要修改：src/api.rs"),
            agent("改完了"),
        ];
        let h = history_for_model(&msgs, "");
        assert!(
            h.iter()
                .any(|m| m.content.contains("需求验证把任务交给了接口成员")),
            "下一轮追问要能看到上次交接的具体要求"
        );
    }

    #[test]
    fn handoff_stops_at_six_hops() {
        assert!(can_handoff(0));
        assert!(can_handoff(5));
        assert!(!can_handoff(6), "第 6 跳之后不再启动下一跳");
        assert!(!can_handoff(7));
    }

    #[test]
    fn entry_role_is_the_earliest_mention() {
        let team = Team {
            id: "t".into(),
            name: "t".into(),
            flow: neverwork_core::FlowMode::Pipeline,
            roles: vec![
                Role::new("a", "需求验证", "需"),
                Role::new("b", "接口成员", "接"),
            ],
        };
        assert_eq!(
            neverwork_core::parse_role_mention("请 @需求验证 然后 @接口成员 修改", &team.roles),
            Some(0)
        );
        assert_eq!(
            neverwork_core::parse_role_mention("没有点名角色", &team.roles),
            None
        );
    }
}

#[cfg(test)]
mod round_bubble_tests {
    use super::RoundBubbles;

    #[test]
    fn each_round_gets_its_own_bubble() {
        let mut bubbles = RoundBubbles::default();
        let mut next = 0usize;
        let mut alloc = || {
            let i = next;
            next += 1;
            i
        };
        // 同一轮内的流式增量共用一个气泡
        assert_eq!(bubbles.bubble(0, &mut alloc), 0);
        assert_eq!(bubbles.bubble(0, &mut alloc), 0);
        // 开始调工具 → 本轮结束，下一段文字必须换新气泡
        bubbles.end_round(0);
        assert_eq!(bubbles.bubble(0, &mut alloc), 1);
    }

    #[test]
    fn roles_do_not_share_bubbles() {
        let mut bubbles = RoundBubbles::default();
        let mut next = 0usize;
        let mut alloc = || {
            let i = next;
            next += 1;
            i
        };
        assert_eq!(bubbles.bubble(0, &mut alloc), 0);
        assert_eq!(bubbles.bubble(1, &mut alloc), 1);
        // 结束角色 0 的一轮不应该影响角色 1
        bubbles.end_round(0);
        assert_eq!(bubbles.bubble(1, &mut alloc), 1);
        assert_eq!(bubbles.bubble(0, &mut alloc), 2);
    }
}

#[cfg(test)]
mod history_tests {
    use super::{history_for_model, HISTORY_BUDGET_CHARS, HISTORY_MAX_MSGS};
    use neverwork_core::{Message, MessageKind, Participant};

    fn user(body: &str) -> Message {
        Message {
            kind: MessageKind::User,
            body: body.into(),
            code: None,
            elapsed_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn agent(body: &str) -> Message {
        Message {
            kind: MessageKind::Agent {
                participant: Participant {
                    role_id: "agent".into(),
                    role_name: "Assistant".into(),
                    color_index: 0,
                    initial: "A".into(),
                },
            },
            body: body.into(),
            code: None,
            elapsed_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn tool_card_with(status: neverwork_core::ToolRunStatus) -> Message {
        Message {
            kind: MessageKind::ToolCall {
                name: "read_file".into(),
                call_id: "call_1".into(),
                arguments: r#"{"path":"a.rs"}"#.into(),
                path: "a.rs".into(),
                summary: "读取 a.rs".into(),
                status,
                output: "SECRET_OUTPUT".into(),
                diff: vec![],
                added: 0,
                removed: 0,
            },
            body: String::new(),
            code: None,
            elapsed_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    fn tool_card() -> Message {
        tool_card_with(neverwork_core::ToolRunStatus::Ok)
    }

    #[test]
    fn current_question_is_not_duplicated() {
        let msgs = vec![user("第一个问题"), agent("第一个回答"), user("第二个问题")];
        let h = history_for_model(&msgs, "第二个问题");
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].content, "第一个问题");
        assert_eq!(h[1].content, "第一个回答");
    }

    #[test]
    fn tool_cards_replay_with_real_output() {
        let msgs = vec![user("读一下"), tool_card(), agent("读完了")];
        let h = history_for_model(&msgs, "读一下");
        // user → assistant(tool_calls) → tool → assistant
        assert_eq!(h.len(), 4);
        assert_eq!(h[1].tool_calls.len(), 1);
        assert_eq!(h[1].tool_calls[0].id, "call_1");
        assert_eq!(h[1].tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(h[2].tool_call_id.as_deref(), Some("call_1"));
        // 追问时必须看得到上一轮真实读到的内容，否则它只会把项目重读一遍
        assert!(h[2].content.contains("SECRET_OUTPUT"));
        assert_eq!(h[3].content, "读完了");
    }

    #[test]
    fn legacy_cards_without_call_id_fall_back_to_digest() {
        let mut card = tool_card();
        if let MessageKind::ToolCall { call_id, .. } = &mut card.kind {
            call_id.clear();
        }
        let msgs = vec![user("读一下"), card, agent("读完了")];
        let h = history_for_model(&msgs, "读一下");
        // 配不成对的旧记录只能退化摘要，绝不能伪造成 tool_calls
        assert_eq!(h.len(), 3);
        assert!(h.iter().all(|m| m.tool_calls.is_empty()));
        let digest = &h[1].content;
        assert!(digest.contains("read_file"), "{digest}");
        assert!(digest.contains("a.rs"), "{digest}");
        assert!(!digest.contains("SECRET_OUTPUT"), "{digest}");
        assert_eq!(h[2].content, "读完了");
    }

    #[test]
    fn oversized_tool_output_keeps_its_head() {
        // 一次 read_file 就能返回几十万字符。整块丢会让模型对上一轮的观察
        // 一无所知（只能重读），所以要截到开头并说明可重读。
        let mut card = tool_card();
        if let MessageKind::ToolCall { output, .. } = &mut card.kind {
            *output = "中".repeat(150_000);
        }
        let msgs = vec![user("读一下"), card, agent("读完了")];
        let h = history_for_model(&msgs, "读一下");
        let total: usize = h.iter().map(|m| m.content.chars().count()).sum();
        assert!(total <= HISTORY_BUDGET_CHARS + 200, "total={total}");
        let tool = h.iter().find(|m| m.tool_call_id.is_some()).expect("工具结果");
        assert!(tool.content.starts_with('中'));
        assert!(tool.content.contains("已截断"));
        assert!(tool.content.contains("重新读取"));
    }

    #[test]
    fn tool_pairs_are_never_split() {
        // 截断只能发生在块的边界上：tool 结果不能失去它的
        // `assistant.tool_calls`，否则 OpenAI 协议下整轮 400。
        let mut msgs = Vec::new();
        for i in 0..40 {
            msgs.push(user(&format!("u{i}")));
            let mut c = tool_card();
            if let MessageKind::ToolCall { call_id, .. } = &mut c.kind {
                *call_id = format!("call_{i}");
            }
            msgs.push(c);
            msgs.push(agent(&format!("a{i}")));
        }
        msgs.push(user("现在"));
        let h = history_for_model(&msgs, "现在");
        for (i, m) in h.iter().enumerate() {
            if let Some(id) = &m.tool_call_id {
                assert!(i > 0, "tool 结果出现在历史开头：{id}");
                let prev = &h[i - 1];
                assert!(
                    prev.tool_calls.iter().any(|c| &c.id == id),
                    "tool {id} 前面缺少配对的 assistant.tool_calls"
                );
            }
        }
    }

    #[test]
    fn unfinished_tools_are_not_claimed() {
        let msgs = vec![
            user("读一下"),
            tool_card_with(neverwork_core::ToolRunStatus::Running),
            agent("还在读"),
        ];
        let h = history_for_model(&msgs, "读一下");
        // 没跑完的调用不构成「做过的事」
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].content, "读一下");
        assert_eq!(h[1].content, "还在读");
    }

    #[test]
    fn keeps_chronological_order() {
        let msgs = vec![user("a"), agent("b"), user("c"), agent("d"), user("e")];
        let h = history_for_model(&msgs, "e");
        let bodies: Vec<&str> = h.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(bodies, vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn oldest_turns_are_dropped_on_overflow() {
        let mut msgs = Vec::new();
        for i in 0..60 {
            msgs.push(user(&format!("u{i}")));
            msgs.push(agent(&format!("a{i}")));
        }
        msgs.push(user("现在"));
        let h = history_for_model(&msgs, "现在");
        // 90 条正文 + 1 条「已省略」占位
        assert_eq!(h.len(), HISTORY_MAX_MSGS + 1);
        assert!(h[0].content.contains("省略"));
        assert_eq!(h.last().unwrap().content, "a59");
    }

    #[test]
    fn char_budget_is_respected() {
        let half = "字".repeat(HISTORY_BUDGET_CHARS / 2 + 1);
        let msgs = vec![user(&half), agent(&half), user("现在")];
        let h = history_for_model(&msgs, "现在");
        // 最近一条吃得起一半预算，更早那条放不下
        assert_eq!(h.len(), 2);
        assert!(h[0].content.contains("省略"));
        assert_eq!(h[1].content.chars().count(), HISTORY_BUDGET_CHARS / 2 + 1);
    }
}
