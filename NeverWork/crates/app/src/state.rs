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
    ConnectionKind, Config, FileNode, Message, MessageKind, Session, SessionMode, ToolRunStatus,
    Workspace,
};
use neverwork_ui::theme;

use crate::filesystem;
use crate::keymap::{self, KeyAction, Motion};
use crate::line_cache::LineCache;
use crate::marketplace::{MarketEntry, MarketSource};
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
    /// 会话列表搜索框。
    SessionSearch,
    /// 会话重命名输入框（回车提交，Esc 取消）。
    RenameSession,
    /// 设置页 / 技能页里的某个表单字段。
    Field(FormField),
}

/// 可编辑的表单字段。索引指向 `settings.providers` 的下标。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormField {
    /// 技能市场搜索框。
    SkillSearch,
    /// 团队编辑器里给角色挑插件的搜索框。
    RoleSkillSearch,
    /// 厂商 API Key。
    ProviderKey(u8),
    /// 厂商 API 基址。
    ProviderUrl(u8),
    /// 厂商显示名。
    ProviderName(u8),
    /// 厂商详情页的模型过滤框。
    ProviderModelFilter,
    /// 厂商详情页「手动添加模型」输入框。
    ProviderNewModel(u8),
    /// 「添加厂商」面板的搜索框。
    ProviderSearch,
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
            | FormField::ProviderName(i)
            | FormField::ProviderNewModel(i) => Some(i),
            _ => None,
        }
    }
}

/// 「模型与厂商」页的三种视图。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderView {
    /// 厂商列表。
    List,
    /// 某个厂商的详情（`settings.providers` 下标）。
    Detail(usize),
    /// 添加厂商面板。
    Add,
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
    /// 给当前会话设状态（参数是编号：0 清除 / 1 待执行 / 2 已完成 / 3 已废弃）。
    SessionStatus(u8),
    /// 把当前会话导出成 Markdown。
    ExportSession,
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
            (Command::TagSession(0), "分类：清除标记", ""),
            (Command::TagSession(1), "分类：功能", ""),
            (Command::TagSession(2), "分类：修复", ""),
            (Command::TagSession(3), "分类：重构", ""),
            (Command::TagSession(4), "分类：文档", ""),
            (Command::TagSession(5), "分类：调试", ""),
            (Command::TagSession(6), "分类：实验", ""),
            (Command::SessionStatus(1), "状态：待执行", ""),
            (Command::SessionStatus(2), "状态：已完成", ""),
            (Command::SessionStatus(3), "状态：已废弃", ""),
            (Command::SessionStatus(0), "状态：清除标记", ""),
            (Command::ExportSession, "导出当前会话为 Markdown", ""),
            (Command::Settings, "打开设置", ""),
            (Command::Skills, "技能市场", ""),
        ]
    }
}

pub struct AppState {
    // ===== 工作区 =====
    /// 当前工作区；`None` = 用户还没选（对话可用，工具不限制路径）。
    pub workspace: Option<Workspace>,
    /// 本地工作区根；`None` = 无工作区。
    pub root: Option<PathBuf>,
    pub files: Vec<FileNode>,
    pub expanded: HashSet<PathBuf>,
    /// 远程工作区目标；`Some` 时文件/编辑器/工具全部走 ssh，`root` 为 `None`。
    pub remote: Option<neverwork_core::SshTarget>,
    /// 远程已展开目录（相对 POSIX 路径）。
    pub remote_expanded: HashSet<String>,
    /// 当前打开的远程文件（相对工作区根）。
    pub remote_open_rel: Option<String>,
    /// 远程树请求代次：迟到的结果不能盖掉新工作区 / 新展开状态。
    remote_gen: u64,
    /// 远程连接/探测代次：切工作区后，迟到的 probe 结果作废。
    remote_conn_gen: u64,
    /// 远程打开文件请求代次：快速连点两个文件时只采用最后一次。
    remote_open_gen: u64,
    /// 文件监听器的停止信号：切工作区时先停旧的，再监听新根。
    watcher_stop: Option<flume::Sender<()>>,
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
    /// 会话列表搜索框（标题/摘要/副标题，不扫正文）。
    pub session_search: TextBuffer,
    /// 正在展示右键菜单的会话下标。
    pub session_menu: Option<usize>,
    /// 菜单弹出的窗口坐标（clamp 在视口内，定位在渲染时做）。
    pub session_menu_at: Option<(f32, f32)>,
    /// 删除二次确认：`Some(idx)` 时该会话的删除按钮变成「再点一次确认」。
    pub session_delete_confirm: Option<usize>,
    /// 「已归档」折叠组是否展开（搜索/筛选激活时强制展开）。
    pub archive_open: bool,
    /// 清理归档的二次确认。
    pub archive_clean_confirm: bool,
    /// 正在重命名的会话下标与输入缓冲。
    pub rename_session: Option<usize>,
    pub rename_buf: TextBuffer,
    /// 工作区管理器浮层是否打开。
    pub workspace_popup: bool,
    /// 正在重命名的工作区下标（`settings.workspaces` 的下标）。
    pub ws_rename: Option<usize>,
    /// 工作区管理器里的「连接远程 SSH」输入行是否展开。
    pub ws_remote_open: bool,
    /// 远程连接/探测失败原因（就地显示在管理器里，不关输入行）。
    pub ws_remote_err: Option<String>,
    /// 远程密码：只在内存里（每条 ssh 命令经 `SSH_ASKPASS` 传），不写配置、不进日志。
    pub remote_pw: Option<String>,
    /// 远程输入行的密码缓冲区（连接成功后转进 `remote_pw` 并清空）。
    pub ws_pw_input: TextBuffer,
    /// 远程输入行焦点是否在密码框（复用 `InputTarget::Workspace`）。
    pub ws_pw_focus: bool,

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
    /// 技能市场：标签过滤（真实来源标签，如「文档工具」「npm」；None = 全部）。
    pub market_tag: Option<String>,
    /// 技能市场排序：0=综合 1=安装量 2=星标。
    pub skill_sort: u8,
    /// 技能页顶部 Tab：0=MCP 1=Skill。
    pub skills_tab: u8,
    /// 技能搜索关键词。
    pub skill_search: String,
    /// 导入配置的路径。
    pub import_path: String,
    /// 「模型与厂商」页当前视图：列表 / 详情 / 添加。
    pub provider_view: ProviderView,
    /// 「添加厂商」面板的搜索词。
    pub provider_query: String,
    /// 详情页模型列表的过滤词。
    pub provider_model_filter: String,
    /// 正在拉取模型（按钮显示「拉取中…」并防重入）。
    pub provider_models_busy: bool,
    /// 正在测试厂商连通性（按钮显示「测试中…」并防重复发请求）。
    pub provider_test_busy: bool,
    /// 移除厂商的二次确认：`Some(idx)` 时该厂商的「移除」变成「确认移除」。
    /// 误删会连已填的 Key 一起丢掉，必须再点一次。
    pub provider_remove_confirm: Option<usize>,
    /// 对话区模型选择器是否打开（按厂商分组，选中写回当前会话）。
    pub model_picker: bool,
    /// 模型选择器的搜索词。
    pub model_query: String,
    /// 模型选择器里键盘高亮的下标。
    pub model_highlight: usize,
    /// 输入框下方的思考等级面板是否展开。
    pub thinking_panel: bool,
    /// 思考档位滑杆的轨道范围（渲染期由 canvas 写入）。
    /// 拖动时用鼠标横坐标减去这个左边，换算成档位。
    pub think_track: std::rc::Rc<std::cell::Cell<Option<gpui::Bounds<gpui::Pixels>>>>,
    /// 正在拖动滑杆：窗口级 mouse move 只在拖动期间改档位，松开时落盘。
    pub thinking_dragging: bool,
    /// 输入框左下「＋」菜单是否展开（引用文件 / 工具 / 清空）。
    pub composer_menu: bool,
    /// 导出配置时是否包含 API Key（默认不含）。
    pub export_with_secrets: bool,
    /// 是否允许 Agent 调用工具（读写文件）。关掉就退化为纯对话。
    /// 本轮生成的开始时刻，用来在回答末尾显示耗时。
    pub turn_started: Option<std::time::Instant>,
    pub tools_enabled: bool,
    /// 新建会话的模式选择浮层是否打开。
    ///
    /// 模式在新建时选定、之后不可切换（见 `SessionMode`），
    /// 所以这个浮层是唯一的入口：点「＋」或命令面板「新建会话」都先到这儿。
    pub new_session_picker: bool,
    /// 团队编辑器里正在挑插件的角色下标；`None` 表示未展开。
    pub role_skill_picker: Option<usize>,
    /// 角色插件搜索词。
    pub role_skill_search: String,
    /// 团队插件面板的筛选页签：0=全部 1=MCP 2=技能 3=已安装。
    pub role_skill_filter: u8,
    /// 待确认的 MCP 插件安装（技能 id）。
    /// MCP 插件安装 = 注册并启动外部进程，必须用户点头后才接。
    pub pending_mcp_install: Option<String>,
    /// MCP 确认通过后要绑定到的角色下标（从团队编辑器发起时记录）。
    pub pending_mcp_attach: Option<usize>,
    /// 技能导入（.md）输入行是否展开。
    pub skill_import_open: bool,
    /// 技能市场当前来源（MCP 页：官方 / 腾讯云；Skill 页：skills.sh）。
    pub market_source: MarketSource,
    /// MCP 页上次使用的来源；从 Skill 页切回来时恢复（默认官方）。
    pub mcp_source: MarketSource,
    /// Skill 页来源（目前固定 skills.sh，保留字段以便将来接新源）。
    pub skill_source: MarketSource,
    /// 详情抽屉里的条目；`None` 表示抽屉关闭。
    pub market_detail: Option<MarketEntry>,
    /// 详情抽屉正文是否正在从上游拉取（打开抽屉时懒加载）。
    pub market_detail_loading: bool,
    /// 详情抽屉正文拉取失败的原因。
    pub market_detail_error: Option<String>,
    /// 详情抽屉从团队角色面板打开时要绑定的角色下标。
    /// `Some` 时底栏主按钮是「绑定到角色」而不是安装 / 卸载。
    pub market_detail_role: Option<usize>,
    /// 各来源最近一次拉到的条目（内存，不序列化）。
    pub market_entries: std::collections::HashMap<MarketSource, Vec<MarketEntry>>,
    /// 各来源上次成功同步的时间。
    pub market_fetched_at: std::collections::HashMap<MarketSource, u64>,
    /// 正在拉取市场数据。
    pub market_loading: bool,
    /// 最近一次拉取失败的原因（有缓存时仍展示缓存）。
    pub market_error: Option<String>,
    /// 正在抓详情配置的腾讯云条目 id。
    pub market_installing: Option<String>,
    /// 团队会话里正在发言的角色下标（顶部流水线高亮用）。
    pub active_role: Option<usize>,
    /// 等待用户批准的写操作。同一时刻最多一个。
    pub pending: Option<PendingApproval>,
    /// 自动批准写操作（仅供自动化验证，`NW_AUTO_APPROVE=1` 开启）。
    pub auto_approve: bool,
    /// 展开的交接卡片（键 = 消息下标）。
    pub expanded_handoff: std::collections::HashSet<usize>,
    /// 右侧栏是否收起。
    pub right_collapsed: bool,
    /// 会话列表是否收起（收成图标栏右侧的 16px 窄条，点窄条展开）。
    pub session_collapsed: bool,
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
    /// 技能搜索防抖代数（每次输入 +1，只有最后一次计时真正发请求）。
    pub skill_search_gen: std::rc::Rc<std::cell::Cell<u64>>,
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
    /// 正在编辑的表单输入框在窗口中的位置（「点外面自动提交」判定用）。
    pub field_area_bounds: std::rc::Rc<Cell<Option<Bounds<Pixels>>>>,
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

/// `NW_PROVIDER_VIEW=add|<下标>`：验证时直接落到「添加厂商」或某个厂商详情。
fn initial_provider_view() -> ProviderView {
    match std::env::var("NW_PROVIDER_VIEW").unwrap_or_default().as_str() {
        "add" => ProviderView::Add,
        other => match other.parse::<usize>() {
            Ok(i) => ProviderView::Detail(i),
            Err(_) => ProviderView::List,
        },
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
/// 优先级：`NW_ROOT` → 上次用过的（`settings.last_workspace`）→ 无工作区。
///
/// 最后一级**不**回落到当前目录 / 向上找 `Cargo.toml`，也**不**自动造一个
/// 中立目录：「没指定工作区」是真实状态——对话照常，工具不限制路径
/// （`ToolContext::unrestricted`）。
fn resolve_root(saved: &str) -> Option<PathBuf> {
    pick_root(std::env::var_os("NW_ROOT").map(PathBuf::from), saved)
}

/// 纯函数版：两个候选都必须是真实存在的目录，否则返回 `None`。
fn pick_root(nw_root: Option<PathBuf>, saved: &str) -> Option<PathBuf> {
    if let Some(p) = nw_root.filter(|p| p.is_dir()) {
        return Some(p);
    }
    let saved = saved.trim();
    if !saved.is_empty() {
        let p = PathBuf::from(saved);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod root_tests {
    use super::pick_root;

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
}

/// 启动时该连哪个远程工作区：`NW_REMOTE` 优先，其次是上次的 `ssh://` 连接串。
fn startup_remote(saved: &str) -> Option<neverwork_core::SshTarget> {
    if let Ok(v) = std::env::var("NW_REMOTE") {
        let v = v.trim();
        if !v.is_empty() {
            return neverwork_core::SshTarget::parse(v).ok();
        }
    }
    if neverwork_core::is_remote_url(saved) {
        return neverwork_core::SshTarget::parse(saved.trim()).ok();
    }
    None
}

fn remote_workspace(target: &neverwork_core::SshTarget) -> Workspace {
    Workspace {
        name: target.name(),
        path: target.url(),
        kind: ConnectionKind::Remote,
        remote_url: Some(target.url()),
        online: true,
    }
}

/// 新建会话的默认模式：默认单助手；`NW_TEAM=1` 供自动化验收直接以团队模式启动。
fn default_session_mode() -> SessionMode {
    if std::env::var("NW_TEAM").map(|v| v == "1").unwrap_or(false) {
        SessionMode::Team
    } else {
        SessionMode::Solo
    }
}

/// 造一个空白会话。
///
/// id 带时间戳：只用 `s{len+1}` 的话，删掉一个再新建就会撞号，
/// 而 id 是会话在磁盘上的唯一标识（`active_id` 靠它恢复）。
fn blank_session(seq: usize, config: &Config, mode: SessionMode) -> Session {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Session {
        id: format!("s{now}-{seq}"),
        title: "新会话".into(),
        subtitle: "空白会话".into(),
        team_name: if mode.is_team() {
            config
                .active_team()
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "默认协作团队".into())
        } else {
            "单助手".into()
        },
        mode,
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
        provider_id: String::new(),
        model: String::new(),
        thinking: neverwork_core::ThinkingLevel::Auto,
        stage: neverwork_core::TaskStage::Idle,
        resume_prompt: String::new(),
    }
}

/// 新会话该继承的（厂商 id, 模型 id）：源会话得真选过，且厂商仍在、启用、
/// 已就绪。没填 Key 的厂商继承过去也发不出去，不如跟着默认走。
fn inherit_session_model(
    src: Option<&Session>,
    providers: &[neverwork_core::ProviderEntry],
) -> (String, String) {
    let Some(src) = src else {
        return (String::new(), String::new());
    };
    let usable = !src.provider_id.is_empty()
        && providers
            .iter()
            .any(|p| p.id == src.provider_id && p.enabled && p.ready());
    if usable {
        (src.provider_id.clone(), src.model.clone())
    } else {
        (String::new(), String::new())
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 把一个会话库装进内存：空库补一条空白会话、重算相对时间、按 `active_id` 恢复选中。
///
/// 启动与切换工作区共用。切换时不用损伤报告（
/// 启动才需要提示「有记录损坏」）。
fn adopt_store(
    store: neverwork_core::SessionStore,
    config: &Config,
) -> (Vec<Session>, usize) {
    let mut sessions = store.sessions;
    if sessions.is_empty() {
        sessions.push(blank_session(0, config, default_session_mode()));
    }
    // 文件里存的「3分钟前」是写盘那一刻的快照，按真实时间戳重算：
    // 否则隔一周打开，列表还写着「3分钟前」。
    for s in &mut sessions {
        s.updated_label = neverwork_core::relative_label(s.updated_at);
    }
    let active = sessions
        .iter()
        .position(|s| s.id == store.active_id)
        .unwrap_or(0);
    (sessions, active)
}

/// 导出为 Markdown 的正文。工具调用压成一行引用，完整输出不进导图文件。
fn session_markdown(s: &Session) -> String {
    use neverwork_core::MessageKind;
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", s.title));
    out.push_str(&format!(
        "- 模式：{}{}\n",
        s.mode.label(),
        if s.mode.is_team() {
            format!("（{}）", s.team_name)
        } else {
            String::new()
        }
    ));
    out.push_str(&format!("- 最近活跃：{}\n", s.updated_label));
    out.push_str(&format!("- 消息数：{}\n\n---\n\n", s.messages.len()));
    for m in &s.messages {
        match &m.kind {
            MessageKind::User => out.push_str(&format!("## 用户\n\n{}\n\n", m.body.trim_end())),
            MessageKind::Agent { participant } => {
                out.push_str(&format!("## {}\n\n{}\n\n", participant.role_name, m.body.trim_end()))
            }
            MessageKind::ToolCall {
                name,
                summary,
                status,
                ..
            } => out.push_str(&format!(
                "> 🔧 `{name}` {} — {:?}\n\n",
                summary,
                status
            )),
            MessageKind::Tool { summary, .. } => {
                if !summary.trim().is_empty() {
                    out.push_str(&format!("> 🛠 {}\n\n", summary));
                }
            }
            MessageKind::Handoff {
                from, to, action, ..
            } => out.push_str(&format!("> 🔀 {from} → {to}：{action}\n\n")),
            _ => {
                let body = m.body.trim_end();
                if !body.is_empty() {
                    out.push_str(&format!("{body}\n\n"));
                }
            }
        }
    }
    out
}

/// 导出文件名：标题只留字母数字/汉字/少量安全符号，其余换成下划线。
/// Windows 非法字符（`<>:"/\\|?*`）一个都不能进路径。
fn safe_file_name(title: &str, id: &str) -> String {
    let mut name: String = title
        .chars()
        .take(32)
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | ' ' | '·' | '（' | '）' | '(' | ')') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = name.trim().trim_matches('_').to_string();
    name = if trimmed.is_empty() {
        "session".into()
    } else {
        trimmed
    };
    format!("{name}-{id}")
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
        let remote_target = startup_remote(&settings.last_workspace);
        let root = if remote_target.is_some() {
            // 远程工作区没有本地根；树/编辑器走 ssh。
            None
        } else {
            resolve_root(&settings.last_workspace)
        };
        let expanded = root
            .as_ref()
            .map(|r| filesystem::default_expanded(r))
            .unwrap_or_default();
        let files = root
            .as_ref()
            .and_then(|r| filesystem::build_tree(r, &expanded).ok())
            .unwrap_or_default();
        let ws = match (&remote_target, &root) {
            (Some(t), _) => Some(remote_workspace(t)),
            (None, Some(root)) => Some(Workspace {
                name: root
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "workspace".into()),
                path: root.to_string_lossy().to_string(),
                kind: ConnectionKind::Local,
                remote_url: None,
                online: true,
            }),
            _ => None,
        };
        let config = neverwork_core::config::load();
        // 主题立即生效，首次渲染就是用户上次选的颜色。
        neverwork_ui::theme::set_theme_mode(settings.theme);
        // MCP 工具在启动时配置一次。连接是惰性的（第一次真要用工具才起进程），
        // 没配 MCP 的用户零开销；改完配置需重启生效。
        neverwork_agent::mcp::set_mirror(settings.mcp_mirror);
        let mcp_servers = neverwork_agent::mcp::servers_from_env()
            .unwrap_or_else(|| settings.mcp_servers.clone());
        neverwork_agent::mcp::configure(mcp_servers);
        let ai_ready = settings.usable_provider_count() > 0
            || neverwork_agent::config_from_env().is_some();
        // 会话来自磁盘，不再是演示数据：重启后要能看到上次聊到哪。
        // `load_with_recovery_key` 会把崩溃残留的 Running/AwaitingApproval 降级成
        // `Interrupted`，并告诉这里降级了几条、丢弃了几条。
        // 无工作区用固定键 `scratch`；本地工作区用路径字符串（兼容已有文件）。
        let session_key = match (&remote_target, &root) {
            (Some(t), _) => t.url(),
            (None, Some(r)) => r.to_string_lossy().to_string(),
            _ => "scratch".to_string(),
        };
        let (store, report) = neverwork_core::sessions::load_with_recovery_key(&session_key);
        let (sessions, active_session) = adopt_store(store, &config);

        // 技能市场：先读缓存（离线也能看到上次同步的真实条目），启动后后台刷新。
        let market_cache = crate::marketplace::load_cache();
        let mut market_entries = std::collections::HashMap::new();
        let mut market_fetched_at = std::collections::HashMap::new();
        for src in MarketSource::all() {
            if let Some(snap) = market_cache.snapshot(src) {
                market_fetched_at.insert(src, snap.fetched_at);
                market_entries.insert(src, snap.entries.clone());
            }
        }

        let mut me = Self {
            workspace: ws,
            root,
            files,
            expanded,
            remote: None,
            remote_expanded: HashSet::new(),
            remote_open_rel: None,
            remote_gen: 0,
            remote_conn_gen: 0,
            remote_open_gen: 0,
            watcher_stop: None,
            git: crate::git::RepoStatus::default(),
            git_selected: None,
            git_is_repo: false,
            git_repos: Vec::new(),
            git_active: None,
            git_commit_msg: TextBuffer::single_line(),
            git_branch_input: TextBuffer::single_line(),
            git_confirm: None,
            git_show_branches: false,
            session_search: TextBuffer::single_line(),
            session_menu: None,
            session_menu_at: None,
            session_delete_confirm: None,
            archive_open: false,
            archive_clean_confirm: false,
            rename_session: None,
            rename_buf: TextBuffer::single_line(),
            workspace_popup: false,
            ws_rename: None,
            ws_remote_open: false,
            ws_remote_err: None,
            remote_pw: None,
            ws_pw_input: TextBuffer::single_line(),
            ws_pw_focus: false,
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
            market_tag: None,
            skill_sort: 0,
            skills_tab: 0,
            skill_search: String::new(),
            import_path: String::new(),
            provider_view: initial_provider_view(),
            provider_query: String::new(),
            provider_model_filter: String::new(),
            provider_models_busy: false,
            provider_test_busy: false,
            provider_remove_confirm: None,
            model_picker: false,
            model_query: String::new(),
            model_highlight: 0,
            thinking_panel: false,
            think_track: Default::default(),
            thinking_dragging: false,
            composer_menu: false,
            export_with_secrets: false,
            turn_started: None,
            tools_enabled: true,
            new_session_picker: false,
            role_skill_picker: None,
            role_skill_search: String::new(),
            role_skill_filter: 0,
            pending_mcp_install: None,
            pending_mcp_attach: None,
            skill_import_open: false,
            market_source: MarketSource::Official,
            mcp_source: MarketSource::Official,
            skill_source: MarketSource::SkillsSh,
            market_detail: None,
            market_detail_loading: false,
            market_detail_error: None,
            market_detail_role: None,
            market_entries,
            market_fetched_at,
            market_loading: false,
            market_error: None,
            market_installing: None,
            active_role: None,
            pending: None,
            auto_approve: false,
            expanded_handoff: std::collections::HashSet::new(),
            // 右侧文件/Git 面板默认收起：把宽度让给对话区，
            // 需要时点一下窄条展开（`NW_RIGHT_COLLAPSED=0` 可强制展开用于验证）。
            right_collapsed: std::env::var("NW_RIGHT_COLLAPSED")
                .map(|v| v != "0")
                .unwrap_or(true),
            // 会话列表默认展开；`NW_SESSION_COLLAPSED=1` 供验证时以收起态启动。
            session_collapsed: std::env::var("NW_SESSION_COLLAPSED")
                .map(|v| v == "1")
                .unwrap_or(false),
            find: crate::find::FindState::default(),
            find_query_buf: TextBuffer::single_line(),
            find_replace_buf: TextBuffer::single_line(),
            folds: std::collections::BTreeSet::new(),
            action_msg: None,
            update_status: None,
            layout_w: std::rc::Rc::new(std::cell::Cell::new(1200.0)),
            chat_w: 1200.0,
            auto_save_gen: std::rc::Rc::new(std::cell::Cell::new(0)),
            skill_search_gen: std::rc::Rc::new(std::cell::Cell::new(0)),
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
            field_area_bounds: std::rc::Rc::new(Cell::new(None)),
        };

        // 启动时按需打开文件（便于验证）。远程工作区不支持本地路径，跳过。
        if me.remote.is_none() {
            if let Ok(path) = std::env::var("NW_OPEN") {
                let p = PathBuf::from(&path);
                if let Ok(content) = filesystem::read_file(&p) {
                    me.editor.reset(content);
                    me.editor.move_to(0, false);
                    me.open_path = Some(p);
                    me.center = CenterView::File;
                }
            }
        }

        me.watcher_stop = start_watcher_opt(me.root.clone(), cx);
        // 远程工作区：先把界面撑起来，再异步探测真实根（展开 `~` / 确认目录存在）。
        if let Some(target) = remote_target.clone() {
            me.remote = Some(target);
            me.watcher_stop = None;
            me.probe_remote_root(cx);
        }
        // 自动化验证用：`NW_SEND=<文本>` 启动后自动发送一条消息。
        let auto_send = std::env::var("NW_SEND").ok().filter(|t| !t.trim().is_empty());
        // 验证用：启动即展开对话区模型选择器（按厂商分组）。
        if let Ok(v) = std::env::var("NW_PICK_MODEL") {
            if !v.is_empty() {
                me.open_chat_model_picker(cx);
            }
        }
        // 验证用：启动即打开输入框面板（thinking=思考档位 / menu=＋菜单）。
        if let Ok(v) = std::env::var("NW_COMPOSER") {
            match v.as_str() {
                "thinking" => me.thinking_panel = true,
                "menu" => me.composer_menu = true,
                _ => {}
            }
        }
        // 验证用：启动即弹出「新建会话」的模式选择浮层。
        if std::env::var("NW_PICK_MODE").map(|v| v == "1").unwrap_or(false) {
            me.new_session_picker = true;
        }
        // 验证用：启动即展开团队编辑器里某个角色的插件市场面板。
        if let Ok(v) = std::env::var("NW_PICK_ROLE_SKILL") {
            if let Ok(i) = v.parse::<usize>() {
                me.role_skill_picker = Some(i);
            }
        }
        // 验证用：预置团队插件面板的筛选页签（0=全部 1=MCP 2=技能 3=已安装）。
        if let Ok(v) = std::env::var("NW_ROLE_SKILL_FILTER") {
            if let Ok(f) = v.parse::<u8>() {
                me.role_skill_filter = f.min(3);
            }
        }
        // 验证用：预置角色插件搜索词（配合 NW_PICK_ROLE_SKILL 截图用）。
        if let Ok(q) = std::env::var("NW_ROLE_SKILL_QUERY") {
            me.role_skill_search = q;
        }
        // 验证用：详情抽屉从团队角色面板打开（参数是角色下标，配合 NW_MARKET_DETAIL 选条目）。
        if let Ok(v) = std::env::var("NW_MARKET_DETAIL_ROLE") {
            if let Ok(i) = v.parse::<usize>() {
                me.market_detail_role = Some(i);
            }
        }
        // 验证用：启动即安装一个技能（文本技能直接落盘；MCP 进入待确认；
        // skills.sh 条目先解析上游正文再落盘）。
        if let Ok(id) = std::env::var("NW_INSTALL_SKILL") {
            if let Some(entry) = me.market_entry_for(&id) {
                me.install_market_entry(entry, cx);
            } else if let Some(sk) = me.find_skill(&id) {
                me.install_skill(sk, cx);
            }
        }
        // 验证用：MCP 插件直接确认安装（配合 NW_INSTALL_SKILL=mcp.* 验证真实写入）。
        if std::env::var("NW_CONFIRM_MCP").map(|v| v == "1").unwrap_or(false) {
            me.confirm_mcp_install(cx);
        }
        // 验证用：展开技能导入输入行。
        if std::env::var("NW_SKILL_IMPORT").map(|v| v == "1").unwrap_or(false) {
            me.skill_import_open = true;
        }
        // 验证用：预置技能市场搜索词。
        if let Ok(q) = std::env::var("NW_SKILL_QUERY") {
            me.skill_search = q;
        }
        // 验证用：预选市场来源（official|tencent|skillssh）。
        if let Ok(k) = std::env::var("NW_MARKET_SOURCE") {
            if let Some(src) = MarketSource::from_key(&k) {
                me.market_source = src;
            }
        }
        // 验证用：预选技能页 Tab（0=MCP 1=Skill）。来源跟着 Tab 归一：
        // Skill 页只展示 skills.sh，MCP 页不展示 skills.sh。
        if let Ok(v) = std::env::var("NW_SKILLS_TAB") {
            if let Ok(tab) = v.parse::<u8>() {
                me.skills_tab = tab.min(1);
            }
        }
        if me.market_source == MarketSource::SkillsSh {
            me.skills_tab = 1;
        }
        if me.skills_tab == 1 {
            me.market_source = me.skill_source;
        } else if me.market_source == MarketSource::SkillsSh {
            me.market_source = me.mcp_source;
        }
        // 验证用：启动即打开详情抽屉（`1` = 当前来源第一条，或写条目 id）。
        if let Ok(v) = std::env::var("NW_MARKET_DETAIL") {
            let id = v.trim();
            me.market_detail = if id.is_empty() || id == "1" {
                me.market_entries
                    .get(&me.market_source)
                    .and_then(|l| l.first().cloned())
                    .or_else(|| me.current_market_items().into_iter().next())
            } else {
                me.market_entries
                    .values()
                    .flatten()
                    .find(|e| e.skill.id == id)
                    .cloned()
            };
            // 与点卡片打开一致：远程条目要拉真实正文
            if me.market_detail.is_some() {
                me.load_market_detail_body(cx);
            }
        }
        // 启动即刷新当前来源；缓存足够新鲜（10 分钟）时跳过，避免每次打开
        // 技能页都等上游首页（实测 1~5s）；刷新按钮仍强制拉。
        // NW_MARKET_NO_REFRESH=1 时只读缓存（离线验收用）。
        let no_refresh = std::env::var("NW_MARKET_NO_REFRESH")
            .map(|v| v == "1")
            .unwrap_or(false);
        let fresh = me
            .market_fetched_at
            .get(&me.market_source)
            .map(|at| neverwork_core::config::now_secs().saturating_sub(*at) < 600)
            .unwrap_or(false);
        if !no_refresh && !fresh {
            me.refresh_market(cx);
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
            if let (Ok(name), Some(root)) = (std::env::var("NW_GIT_REPO"), me.root.clone()) {
                me.git_active = Some(root.join(name));
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
        // 验证专用：会话列表搜索/操作面板/归档组/工作区浮层直达
        if let Ok(q) = std::env::var("NW_SESSION_QUERY") {
            me.session_search.reset(&q);
        }
        if let Ok(v) = std::env::var("NW_SESSION_MENU") {
            me.session_menu = v.parse::<usize>().ok();
            // 截图用：直接给一个固定坐标（会话列表区域中部）
            if me.session_menu.is_some() {
                me.session_menu_at = Some((150.0, 320.0));
            }
        }
        if std::env::var("NW_WORKSPACE_POPUP").map(|v| v == "1").unwrap_or(false) {
            me.workspace_popup = true;
        }
        // 验证专用：启动即打开「手动输入路径」行（复现 Esc 后输入失效的回归）。
        if std::env::var("NW_WS_INPUT").map(|v| v == "1").unwrap_or(false) {
            me.open_workspace_folder_input(cx);
        }
        if std::env::var("NW_ARCHIVE_OPEN").map(|v| v == "1").unwrap_or(false) {
            me.archive_open = true;
        }
        // 验收专用：启动即切到另一个工作区（等价于点工作区管理器里的一行）。
        // 必须在 NW_DEMO_STATUS 之前：先切走再打标记，避免把演示状态写回旧库。
        if let Ok(p) = std::env::var("NW_SWITCH_WS") {
            me.enter_workspace(std::path::PathBuf::from(p), cx);
        }
        // 验收专用：给前几条会话临时打状态，检查胶囊/归档组。
        // 只改内存，不落盘（不调 save_sessions），进程退出即无。
        if std::env::var("NW_DEMO_STATUS").map(|v| v == "1").unwrap_or(false) {
            let demo = [
                neverwork_core::SessionStatus::Todo,
                neverwork_core::SessionStatus::Done,
                neverwork_core::SessionStatus::Dropped,
            ];
            for (i, st) in demo.iter().enumerate() {
                if let Some(s) = me.sessions.get_mut(i) {
                    s.status = *st;
                }
            }
        }
        // 启动就把当前工作区记进最近列表（首次运行时列表里至少有一条）。
        // 无工作区时不记——「还没选」不该变成一个最近项。
        if let Some(target) = me.remote.clone() {
            let url = target.url();
            me.settings.last_workspace = url.clone();
            me.settings.record_workspace(&url, now_secs());
            let _ = neverwork_core::config::save_settings(&me.settings);
        } else if let Some(root) = me.root.clone() {
            me.settings
                .record_workspace(&root.to_string_lossy(), now_secs());
            let _ = neverwork_core::config::save_settings(&me.settings);
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
            me.new_session(default_session_mode(), cx);
            me.chat.reset(t);
            me.send_message(cx);
        } else if let Ok(v) = std::env::var("NW_NEW_SESSION") {
            // 验证用：新建一个指定模式的空会话（team|solo）。
            let mode = if v.eq_ignore_ascii_case("team") {
                SessionMode::Team
            } else {
                SessionMode::Solo
            };
            me.new_session(mode, cx);
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

    /// 当前会话的运行模式。模式在新建会话时选定，之后不可切换。
    pub(crate) fn session_mode(&self) -> SessionMode {
        self.session().mode
    }

    pub(crate) fn is_team_mode(&self) -> bool {
        self.session_mode().is_team()
    }

    /// 会话库键：远程用连接串，本地用路径（兼容旧文件），无工作区用 `scratch`。
    pub(crate) fn session_key(&self) -> String {
        if let Some(t) = &self.remote {
            return t.url();
        }
        self.root
            .as_ref()
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or_else(|| "scratch".to_string())
    }

    pub(crate) fn has_workspace(&self) -> bool {
        self.workspace.is_some()
    }

    /// 把工作区相对路径还原成绝对路径；无工作区时原样返回。
    pub(crate) fn abs_path(&self, rel: &str) -> PathBuf {
        let p = PathBuf::from(rel);
        match &self.root {
            Some(root) if p.is_relative() => root.join(p),
            _ => p,
        }
    }

    /// 把会话写回磁盘。失败不中断对话，只提示一次。
    pub(crate) fn save_sessions(&mut self) {
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
        if let Err(e) = neverwork_core::sessions::save_key(&self.session_key(), &store) {
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

    /// 新建会话并切过去。`mode` 在这里定死，之后不可切换。
    ///
    /// 新会话继承当前会话选的厂商 / 模型：刚在 A 会话切到某模型，
    /// 接着开 B 会话干活是最常见的节奏，回到默认会让人觉得「白切了」。
    /// 厂商已被删或停用时回落到「跟随默认」。
    pub(crate) fn new_session(&mut self, mode: SessionMode, cx: &mut Context<Self>) {
        let mut s = blank_session(self.sessions.len(), &self.config, mode);
        let (provider_id, model) = inherit_session_model(
            self.sessions.get(self.active_session),
            &self.settings.providers,
        );
        s.provider_id = provider_id;
        s.model = model;
        self.sessions.push(s);
        self.active_session = self.sessions.len() - 1;
        self.center = CenterView::Chat;
        self.palette_open = false;
        self.new_session_picker = false;
        self.active_role = None;
        self.save_sessions();
        cx.notify();
    }

    /// 打开「新建会话」的模式选择浮层。
    pub(crate) fn open_new_session_picker(&mut self, cx: &mut Context<Self>) {
        self.new_session_picker = true;
        self.palette_open = false;
        cx.notify();
    }

    pub(crate) fn close_new_session_picker(&mut self, cx: &mut Context<Self>) {
        self.new_session_picker = false;
        cx.notify();
    }

    pub(crate) fn select_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.sessions.len() {
            self.active_session = idx;
            self.center = CenterView::Chat;
            // 上一个会话的流水线高亮不能带到新会话
            self.active_role = None;
            // 模型选择器里的勾选 / 高亮都是上一个会话的，换会话就收起。
            if self.model_picker {
                self.close_chat_model_picker(cx);
            }
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

    /// 手动标记会话状态。生成中不允许改——运行态优先级高于标记。
    pub(crate) fn set_session_status(
        &mut self,
        idx: usize,
        status: neverwork_core::SessionStatus,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.sessions.get_mut(idx) else {
            return;
        };
        if s.status == neverwork_core::SessionStatus::Working {
            self.notice = Some("正在生成，结束后再标记状态".into());
            cx.notify();
            return;
        }
        s.status = status;
        let short: String = s.title.chars().take(16).collect();
        let label = status.label();
        self.notice = Some(if label.is_empty() {
            format!("已清除「{short}」的状态标记")
        } else {
            format!("「{short}」已标记为{label}")
        });
        self.session_menu = None;
        self.session_menu_at = None;
        self.save_sessions();
        cx.notify();
    }

    /// 在窗口坐标处弹出会话操作菜单；同一会话再点则收起。
    pub(crate) fn toggle_session_menu(
        &mut self,
        idx: usize,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        if self.session_menu == Some(idx) {
            self.close_session_menu(cx);
            return;
        }
        self.session_menu = Some(idx);
        self.session_menu_at = Some(at);
        self.session_delete_confirm = None;
        cx.notify();
    }

    /// 收起会话操作菜单。
    pub(crate) fn close_session_menu(&mut self, cx: &mut Context<Self>) {
        self.session_menu = None;
        self.session_menu_at = None;
        self.session_delete_confirm = None;
        cx.notify();
    }

    /// 删除会话（第一次点进入确认态，第二次真删）。
    pub(crate) fn click_delete_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if self.session_delete_confirm != Some(idx) {
            self.session_delete_confirm = Some(idx);
            cx.notify();
            return;
        }
        if idx >= self.sessions.len() {
            return;
        }
        let removed = self.sessions.remove(idx);
        let short: String = removed.title.chars().take(16).collect();
        if self.sessions.is_empty() {
            self.sessions.push(blank_session(0, &self.config, default_session_mode()));
        }
        // active_session 是索引引用：删当前会话落到相邻的，删前面的则前移一格
        if idx == self.active_session {
            self.active_session = idx.min(self.sessions.len() - 1);
            self.active_role = None;
        } else if idx < self.active_session {
            self.active_session -= 1;
        }
        // 重命名/菜单指向的是下标，删完要同步修正，否则下次展开会错位到别人
        self.session_menu = None;
        self.session_menu_at = None;
        self.session_delete_confirm = None;
        match self.rename_session {
            Some(r) if r == idx => {
                self.rename_session = None;
                self.rename_buf.reset("");
                self.input_target = InputTarget::Chat;
            }
            Some(r) if r > idx => self.rename_session = Some(r - 1),
            _ => {}
        }
        self.notice = Some(format!("已删除「{short}」"));
        self.save_sessions();
        cx.notify();
    }

    /// 清理全部归档会话（已完成/已废弃且未置顶），二次确认。
    pub(crate) fn click_clean_archived(&mut self, cx: &mut Context<Self>) {
        if !self.archive_clean_confirm {
            self.archive_clean_confirm = true;
            cx.notify();
            return;
        }
        let active_id = self
            .sessions
            .get(self.active_session)
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let before = self.sessions.len();
        self.sessions
            .retain(|s| !neverwork_core::is_archived_session(s));
        let removed = before - self.sessions.len();
        if self.sessions.is_empty() {
            self.sessions.push(blank_session(0, &self.config, default_session_mode()));
        }
        self.active_session = self
            .sessions
            .iter()
            .position(|s| s.id == active_id)
            .unwrap_or(0);
        self.active_role = None;
        self.archive_clean_confirm = false;
        self.archive_open = false;
        // 清理会打乱所有下标，开着的菜单/重命名/确认态全部作废
        self.session_menu = None;
        self.session_menu_at = None;
        self.session_delete_confirm = None;
        self.rename_session = None;
        self.rename_buf.reset("");
        if self.input_target == InputTarget::RenameSession {
            self.input_target = InputTarget::Chat;
        }
        self.notice = Some(format!("已清理 {removed} 个归档会话"));
        self.save_sessions();
        cx.notify();
    }

    pub(crate) fn begin_rename_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(s) = self.sessions.get(idx) else {
            return;
        };
        let title = s.title.clone();
        self.rename_session = Some(idx);
        self.rename_buf.reset(&title);
        self.input_target = InputTarget::RenameSession;
        self.session_menu = None;
        self.session_menu_at = None;
        self.session_delete_confirm = None;
        cx.notify();
    }

    pub(crate) fn commit_rename_session(&mut self, cx: &mut Context<Self>) {
        let Some(idx) = self.rename_session.take() else {
            return;
        };
        let name = self.rename_buf.text().trim().to_string();
        if !name.is_empty() {
            if let Some(s) = self.sessions.get_mut(idx) {
                s.title = name;
            }
            self.save_sessions();
        }
        self.input_target = InputTarget::Chat;
        cx.notify();
    }

    pub(crate) fn cancel_rename_session(&mut self, cx: &mut Context<Self>) {
        self.rename_session = None;
        self.rename_buf.reset("");
        self.input_target = InputTarget::Chat;
        cx.notify();
    }

    /// 把会话导出为 Markdown：落在 `~/.neverwork/exports/`，不污染工作区目录。
    pub(crate) fn export_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(s) = self.sessions.get(idx) else {
            return;
        };
        let dir = neverwork_core::config::config_dir().join("exports");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.notice = Some(format!("导出失败：{e}"));
            cx.notify();
            return;
        };
        let path = dir.join(format!("{}.md", safe_file_name(&s.title, &s.id)));
        match std::fs::write(&path, session_markdown(s)) {
            Ok(()) => self.notice = Some(format!("已导出到 {}", path.display())),
            Err(e) => self.notice = Some(format!("导出失败：{e}")),
        }
        self.session_menu = None;
        self.session_menu_at = None;
        cx.notify();
    }

    // ===== 会话库切换与工作区管理器 =====

    /// 把当前内存里的会话装回原位（切换工作区后调用）。
    fn adopt_session_store(&mut self, store: neverwork_core::SessionStore) {
        let (sessions, active) = adopt_store(store, &self.config);
        self.sessions = sessions;
        self.active_session = active;
        self.active_role = None;
    }

    /// 进入一个工作区：存旧会话 → 换根 → 重载该工作区的会话库。
    ///
    /// 会话按工作区隔离存储（`sessions/<slug>-<hash>.json`），不换库就会把
    /// 上一个项目的列表带过来，保存时还会写进新工作区的文件。
    pub(crate) fn enter_workspace(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.streaming {
            self.notice = Some("正在生成，先停止再切换工作区".into());
            self.workspace_popup = false;
            self.restore_input_target();
            cx.notify();
            return;
        }
        if !path.is_dir() {
            self.notice = Some(format!("不是一个目录：{}", path.display()));
            self.workspace_popup = false;
            self.restore_input_target();
            cx.notify();
            return;
        }
        // 先把旧工作区的会话落盘，否则下面改 root 后旧会话会写进新文件
        self.save_sessions();
        let root_str = path.to_string_lossy().to_string();
        self.settings.last_workspace = root_str.clone();
        self.settings.record_workspace(&root_str, now_secs());
        let _ = neverwork_core::config::save_settings(&self.settings);
        self.expanded = filesystem::default_expanded(&path);
        self.files = filesystem::build_tree(&path, &self.expanded).unwrap_or_default();
        self.workspace = Some(Workspace {
            name: path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".into()),
            path: root_str.clone(),
            kind: ConnectionKind::Local,
            remote_url: None,
            online: true,
        });
        self.root = Some(path);
        // 监听跟着换根：不停旧的，旧目录的改动还会刷进新文件树。
        self.watcher_stop = start_watcher_opt(self.root.clone(), cx);
        self.open_path = None;
        self.editor.reset("");
        self.dirty = false;
        self.folds.clear();
        self.center = CenterView::Chat;
        self.adopt_session_store(neverwork_core::sessions::load_with_recovery_key(&root_str).0);
        // 列表的搜索/菜单是上一个工作区的语境，全部复位
        self.session_search.reset("");
        self.session_menu = None;
        self.session_menu_at = None;
        self.session_delete_confirm = None;
        self.rename_session = None;
        self.archive_clean_confirm = false;
        self.archive_open = false;
        self.workspace_popup = false;
        self.ws_rename = None;
        self.ws_prompt_open = false;
        self.palette_open = false;
        self.ws_input.reset("");
        // 远程状态整体让位：连接目标、已展开目录、打开的远程文件、在途请求代次。
        // 不清会在本地工作区里继续显示远程树，保存还会写回远端。
        self.remote = None;
        self.remote_expanded.clear();
        self.remote_open_rel = None;
        self.remote_gen += 1;
        self.remote_conn_gen += 1;
        self.remote_open_gen += 1;
        self.ws_remote_open = false;
        self.ws_remote_err = None;
        // 密码不留到本地工作区：那是另一台机器的凭据
        self.remote_pw = None;
        self.ws_pw_input.reset("");
        self.ws_pw_focus = false;
        // 关闭路径行后把输入目标交回正文：目标留着 Workspace，
        // 焦点会一直挂在只在该行渲染时才存在的句柄上，打字/回车全部失灵。
        self.restore_input_target();
        self.notice = Some(format!("已切换到 {root_str}"));
        self.refresh_git(cx);
    }

    // ===== 远程工作区 =====

    /// 当前远程认证：有密码就带上（只把内存里的值克隆给后台线程）。
    fn remote_auth(&self) -> neverwork_core::RemoteAuth {
        match &self.remote_pw {
            Some(pw) => neverwork_core::RemoteAuth::password(pw.clone()),
            None => neverwork_core::RemoteAuth::key_only(),
        }
    }

    /// 带当前认证的远程句柄（后台线程用）。
    fn remote_fs(&self) -> Option<neverwork_core::RemoteFs> {
        self.remote
            .clone()
            .map(|t| neverwork_core::RemoteFs::with_auth(t, self.remote_auth()))
    }

    /// 启动时后台探测远程根：确认 ssh 可用、目录存在，然后拉树。
    ///
    /// 不把探测出的绝对路径写回 `SshTarget`：会话库键就是连接串，
    /// 展开 `~` 前后键不同会让用户以为会话丢了。后续命令统一 `cd` 到原值。
    pub(crate) fn probe_remote_root(&mut self, cx: &mut Context<Self>) {
        if self.remote.is_none() {
            return;
        }
        let Some(fs) = self.remote_fs() else {
            return;
        };
        self.remote_conn_gen += 1;
        let conn_gen = self.remote_conn_gen;
        let (tx, rx) = flume::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let _ = tx.send(fs.probe().map(|_| ()));
        });
        cx.spawn(async move |weak, async_cx| {
            if let Ok(msg) = rx.recv_async().await {
                let _ = weak.update(async_cx, |this, cx| {
                    // 期间切走了：旧探测结果不许动新工作区
                    if this.remote_conn_gen != conn_gen || this.remote.is_none() {
                        return;
                    }
                    match msg {
                        Ok(()) => {
                            if let Some(w) = &mut this.workspace {
                                w.online = true;
                            }
                            this.ws_remote_err = None;
                            this.refresh_remote_tree(cx);
                        }
                        Err(e) => {
                            if let Some(w) = &mut this.workspace {
                                w.online = false;
                            }
                            this.notice = Some(format!("连接远程失败：{e}"));
                            cx.notify();
                        }
                    }
                });
            }
        })
        .detach();
    }

    /// 拉远程树：只请求已展开目录，结果按代次丢弃（迟到的不能盖新状态）。
    pub(crate) fn refresh_remote_tree(&mut self, cx: &mut Context<Self>) {
        if self.remote.is_none() {
            self.files.clear();
            cx.notify();
            return;
        }
        let Some(fs) = self.remote_fs() else {
            return;
        };
        self.remote_gen += 1;
        let gen = self.remote_gen;
        let expanded = self.remote_expanded.clone();
        let (tx, rx) = flume::bounded::<Result<Vec<neverwork_core::RemoteNode>, String>>(1);
        std::thread::spawn(move || {
            let _ = tx.send(fs.tree(&expanded));
        });
        cx.spawn(async move |weak, async_cx| {
            if let Ok(msg) = rx.recv_async().await {
                let _ = weak.update(async_cx, |this, cx| {
                    if this.remote_gen != gen {
                        return;
                    }
                    match msg {
                        Ok(nodes) => {
                            this.files = nodes
                                .into_iter()
                                .map(|n| FileNode {
                                    name: n
                                        .path
                                        .rsplit('/')
                                        .find(|s| !s.is_empty())
                                        .unwrap_or(&n.path)
                                        .to_string(),
                                    is_dir: n.is_dir,
                                    level: n.path.matches('/').count() as u8,
                                    expanded: n.is_dir && this.remote_expanded.contains(&n.path),
                                    modified: false,
                                    owner: None,
                                    path: n.path,
                                })
                                .collect();
                            if let Some(w) = &mut this.workspace {
                                w.online = true;
                            }
                            cx.notify();
                        }
                        Err(e) => {
                            this.notice = Some(format!("读取远程目录失败：{e}"));
                            cx.notify();
                        }
                    }
                });
            }
        })
        .detach();
    }

    /// 远程目录展开/收起（路径是相对工作区根的 POSIX 路径）。
    pub(crate) fn toggle_remote_dir(&mut self, rel: String, cx: &mut Context<Self>) {
        if !self.remote_expanded.remove(&rel) {
            self.remote_expanded.insert(rel);
        }
        self.refresh_remote_tree(cx);
    }

    /// 连一个远程工作区：先探测（目录存在 / 认证可用），成功才真正切过去。
    ///
    /// `password` 只在内存里流转，不写配置；留空时走密钥 / ssh-agent。
    /// 失败后输入行会就地展开：上面是连接串，下面是密码，改完直接重试。
    pub(crate) fn connect_remote(
        &mut self,
        target: neverwork_core::SshTarget,
        password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.streaming {
            self.ws_remote_err = Some("正在生成，先停止再切换工作区".into());
            cx.notify();
            return;
        }
        self.ws_remote_err = None;
        self.notice = Some(format!("正在连接 {}…", target.display()));
        self.remote_conn_gen += 1;
        let conn_gen = self.remote_conn_gen;
        let auth = match &password {
            Some(pw) if !pw.is_empty() => neverwork_core::RemoteAuth::password(pw.clone()),
            _ => neverwork_core::RemoteAuth::key_only(),
        };
        let fs = neverwork_core::RemoteFs::with_auth(target.clone(), auth);
        let (tx, rx) = flume::bounded::<Result<(), String>>(1);
        std::thread::spawn(move || {
            let _ = tx.send(fs.probe().map(|_| ()));
        });
        cx.spawn(async move |weak, async_cx| {
            if let Ok(msg) = rx.recv_async().await {
                let _ = weak.update(async_cx, |this, cx| {
                    // 探测期间又切了工作区：不要强行把用户拉回这个远程
                    if this.remote_conn_gen != conn_gen {
                        return;
                    }
                    match msg {
                        Ok(()) => this.enter_remote_workspace(target, password, cx),
                        Err(e) => {
                            this.ws_remote_err = Some(e.clone());
                            this.notice = Some(format!("连接远程失败：{e}"));
                            // 从最近列表点的远程项：就地展开输入行，带着连接串/密码方便改
                            if this.workspace_popup {
                                this.ws_remote_open = true;
                                this.ws_input.reset(&target.display());
                                if let Some(pw) = &password {
                                    this.ws_pw_input.reset(pw);
                                }
                                this.ws_pw_focus = password.is_some();
                                this.input_target = InputTarget::Workspace;
                            }
                            cx.notify();
                        }
                    }
                });
            }
        })
        .detach();
        cx.notify();
    }

    /// 工作区管理器里「连接远程 SSH…」输入行：回车 / 按钮都到这里。
    pub(crate) fn connect_remote_from_input(&mut self, cx: &mut Context<Self>) {
        let input = self.ws_input.text().trim().to_string();
        if input.is_empty() {
            self.ws_remote_err = Some("请输入 user@host:/path".into());
            cx.notify();
            return;
        }
        let pw = self.ws_pw_input.text().trim().to_string();
        let pw = (!pw.is_empty()).then_some(pw);
        match neverwork_core::SshTarget::parse(&input) {
            Ok(t) => self.connect_remote(t, pw, cx),
            Err(e) => {
                self.ws_remote_err = Some(e);
                cx.notify();
            }
        }
    }

    /// 展开「连接远程 SSH」输入行。
    pub(crate) fn open_remote_workspace_input(&mut self, cx: &mut Context<Self>) {
        self.ws_remote_open = true;
        self.ws_remote_err = None;
        self.ws_rename = None;
        self.ws_input.reset("");
        self.ws_pw_input.reset("");
        self.ws_pw_focus = false;
        self.input_target = InputTarget::Workspace;
        cx.notify();
    }

    /// 收起远程输入行（不再重试，错误一起清掉）。
    pub(crate) fn close_remote_workspace_input(&mut self, cx: &mut Context<Self>) {
        self.ws_remote_open = false;
        self.ws_remote_err = None;
        self.ws_input.reset("");
        self.ws_pw_input.reset("");
        self.ws_pw_focus = false;
        self.restore_input_target();
        cx.notify();
    }

    /// 切到远程工作区：存旧会话 → 清本地状态 → 换会话库 → 拉远程树。
    pub(crate) fn enter_remote_workspace(
        &mut self,
        target: neverwork_core::SshTarget,
        password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        // 旧工作区的会话必须先落盘，否则下面换 session_key 后写进新文件
        self.save_sessions();
        let url = target.url();
        self.settings.last_workspace = url.clone();
        self.settings.record_workspace(&url, now_secs());
        let _ = neverwork_core::config::save_settings(&self.settings);
        // 本地那一套全部让位：监听器、根、展开态、编辑器、Git
        self.watcher_stop = None;
        self.root = None;
        self.expanded.clear();
        self.files.clear();
        self.open_path = None;
        self.editor.reset("");
        self.dirty = false;
        self.folds.clear();
        self.remote_expanded.clear();
        self.remote_gen += 1;
        self.remote_conn_gen += 1;
        self.remote_open_gen += 1;
        self.remote_open_rel = None;
        self.remote = Some(target.clone());
        // 密码只活在内存：后续树/读写/Agent 工具每次 ssh 都带上它
        self.remote_pw = password.filter(|p| !p.is_empty());
        self.ws_pw_input.reset("");
        self.ws_pw_focus = false;
        self.workspace = Some(remote_workspace(&target));
        self.center = CenterView::Chat;
        self.adopt_session_store(neverwork_core::sessions::load_with_recovery_key(&url).0);
        // 列表的搜索/菜单是上一个工作区的语境，全部复位
        self.session_search.reset("");
        self.session_menu = None;
        self.session_menu_at = None;
        self.session_delete_confirm = None;
        self.rename_session = None;
        self.archive_clean_confirm = false;
        self.archive_open = false;
        self.workspace_popup = false;
        self.ws_rename = None;
        self.ws_prompt_open = false;
        self.ws_remote_open = false;
        self.ws_remote_err = None;
        self.ws_input.reset("");
        self.palette_open = false;
        self.restore_input_target();
        self.notice = Some(format!("已连接到 {url}"));
        self.refresh_git(cx);
        self.refresh_remote_tree(cx);
    }

    pub(crate) fn open_workspace_prompt(&mut self, cx: &mut Context<Self>) {
        self.workspace_popup = true;
        self.ws_rename = None;
        self.ws_remote_open = false;
        self.ws_remote_err = None;
        self.ws_input.reset("");
        self.ws_pw_input.reset("");
        self.ws_pw_focus = false;
        cx.notify();
    }

    pub(crate) fn close_workspace_prompt(&mut self, cx: &mut Context<Self>) {
        self.workspace_popup = false;
        self.ws_rename = None;
        self.ws_remote_open = false;
        self.ws_remote_err = None;
        self.ws_input.reset("");
        self.ws_pw_input.reset("");
        self.ws_pw_focus = false;
        self.restore_input_target();
        cx.notify();
    }

    /// 工作区管理器底部的「打开文件夹…」：转入手输路径行。
    pub(crate) fn open_workspace_folder_input(&mut self, cx: &mut Context<Self>) {
        self.ws_prompt_open = true;
        self.palette_open = true;
        self.workspace_popup = false;
        self.ws_rename = None;
        self.ws_input.reset("");
        self.ws_pw_focus = false;
        self.input_target = InputTarget::Workspace;
        cx.notify();
    }

    /// 关闭浮层后把键盘交还正文：路径行/重命名行不在时，`Workspace` 目标必须复位。
    ///
    /// 不复位的后果：每帧焦点被拉向只在路径行渲染时才存在的 `ws_focus`，
    /// 打字、Esc、回车全部派发到无节点句柄被丢弃。
    pub(crate) fn restore_input_target(&mut self) {
        if self.input_target != InputTarget::Workspace {
            return;
        }
        if self.ws_prompt_open || self.ws_rename.is_some() || self.ws_remote_open {
            return;
        }
        self.ws_pw_focus = false;
        self.input_target = if self.center == CenterView::File
            && (self.open_path.is_some() || self.remote_open_rel.is_some())
        {
            InputTarget::Editor
        } else {
            InputTarget::Chat
        };
        self.marked_range = None;
    }

    /// 「打开文件夹…」：弹系统原生目录选择器，选中后直接进工作区。
    pub(crate) fn pick_workspace_folder(&mut self, cx: &mut Context<Self>) {
        use gpui::PathPromptOptions;
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("选择工作区文件夹".into()),
        });
        self.workspace_popup = false;
        self.restore_input_target();
        cx.spawn(async move |weak, async_cx| {
            match rx.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(p) = paths.into_iter().next() {
                        let _ = weak.update(async_cx, |this, cx| this.enter_workspace(p, cx));
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => {
                    let _ = weak.update(async_cx, |this, cx| {
                        this.notice = Some(format!("打开文件夹失败：{e}"));
                        cx.notify();
                    });
                }
                Err(_) => {}
            }
        })
        .detach();
    }

    pub(crate) fn switch_to_recent_workspace(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(path) = self
            .settings
            .workspaces
            .get(i)
            .map(|w| w.path.clone())
        else {
            return;
        };
        // 远程条目的 path 是 `ssh://` 连接串，本地条目才走 enter_workspace。
        if neverwork_core::is_remote_url(&path) {
            match neverwork_core::SshTarget::parse(&path) {
                // 就是当前这台：复用内存里的密码；换目标则先试密钥
                Ok(t) => {
                    let pw = if self.remote.as_ref() == Some(&t) {
                        self.remote_pw.clone()
                    } else {
                        None
                    };
                    self.connect_remote(t, pw, cx)
                }
                Err(e) => {
                    self.ws_remote_err = Some(e.clone());
                    self.notice = Some(format!("远程连接串无效：{e}"));
                    cx.notify();
                }
            }
            return;
        }
        self.enter_workspace(PathBuf::from(path), cx);
    }

    pub(crate) fn toggle_workspace_pin(&mut self, i: usize, cx: &mut Context<Self>) {
        if let Some(w) = self.settings.workspaces.get_mut(i) {
            w.pinned = !w.pinned;
        }
        let _ = neverwork_core::config::save_settings(&self.settings);
        cx.notify();
    }

    pub(crate) fn remove_recent_workspace(&mut self, i: usize, cx: &mut Context<Self>) {
        if i < self.settings.workspaces.len() {
            self.settings.workspaces.remove(i);
        }
        self.ws_rename = None;
        let _ = neverwork_core::config::save_settings(&self.settings);
        cx.notify();
    }

    pub(crate) fn begin_rename_workspace(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(w) = self.settings.workspaces.get(i) else {
            return;
        };
        self.ws_rename = Some(i);
        self.ws_input.reset(&w.name);
        self.ws_pw_focus = false;
        self.input_target = InputTarget::Workspace;
        cx.notify();
    }

    pub(crate) fn commit_rename_workspace(&mut self, cx: &mut Context<Self>) {
        let Some(i) = self.ws_rename.take() else {
            return;
        };
        let name = self.ws_input.text().trim().to_string();
        if let Some(w) = self.settings.workspaces.get_mut(i) {
            w.name = name;
        }
        let _ = neverwork_core::config::save_settings(&self.settings);
        self.ws_input.reset("");
        self.input_target = InputTarget::Chat;
        cx.notify();
    }

    pub(crate) fn cancel_rename_workspace(&mut self, cx: &mut Context<Self>) {
        self.ws_rename = None;
        self.ws_input.reset("");
        self.input_target = InputTarget::Chat;
        cx.notify();
    }

    /// 清空搜索框（Esc / ✕ 按钮）。
    pub(crate) fn clear_session_search(&mut self, cx: &mut Context<Self>) {
        self.session_search.reset("");
        cx.notify();
    }

    /// 展开/收起「已归档」折叠组。
    pub(crate) fn toggle_archive(&mut self, cx: &mut Context<Self>) {
        self.archive_open = !self.archive_open;
        self.archive_clean_confirm = false;
        cx.notify();
    }

    // ===== 文件 =====

    pub fn open_file_name(&self) -> String {
        if let Some(rel) = &self.remote_open_rel {
            return rel
                .rsplit('/')
                .find(|s| !s.is_empty())
                .unwrap_or(rel)
                .to_string();
        }
        self.open_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "未命名".into())
    }

    pub(crate) fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            self.refresh_remote_tree(cx);
            return;
        }
        self.files = match &self.root {
            Some(root) => filesystem::build_tree(root, &self.expanded).unwrap_or_default(),
            None => Vec::new(),
        };
        cx.notify();
    }

    pub(crate) fn toggle_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
        self.refresh_tree(cx);
    }

    pub(crate) fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if self.remote.is_some() {
            self.remote_expanded.clear();
            self.refresh_remote_tree(cx);
            return;
        }
        self.expanded.clear();
        self.refresh_tree(cx);
    }

    /// 当前 Git 面板操作的目录（选中的子仓库，或工作区根）；无工作区/远程为 `None`。
    pub(crate) fn git_dir(&self) -> Option<PathBuf> {
        if self.remote.is_some() {
            return None;
        }
        self.git_active.clone().or_else(|| self.root.clone())
    }

    /// 重新发现工作区里的仓库。
    ///
    /// 规则：工作区根本身是仓库 → 只列它；否则列直接子目录里的仓库。
    /// 不递归深层找，避免在 node_modules 之类目录里翻出几百个仓库。
    fn discover_git_repos(&mut self) {
        self.git_repos.clear();
        let Some(root) = self.root.clone() else {
            self.git_active = None;
            return;
        };
        if crate::git::repo_root(&root).is_some() {
            self.git_repos.push((".".to_string(), root.clone()));
        } else {
            for name in crate::git::child_repos(&root) {
                self.git_repos.push((name.clone(), root.join(&name)));
            }
        }

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
        let Some(dir) = self.git_dir() else {
            // 无工作区：清空上一次的状态，面板显示占位而不是旧仓库的改动
            self.git_is_repo = false;
            self.git = crate::git::RepoStatus::default();
            self.git_selected = None;
            cx.notify();
            return;
        };
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
            crate::git::RepoQuery::NotARepo => {
                self.git_is_repo = false;
                self.git = crate::git::RepoStatus::default();
                self.git_selected = None;
                // 不往对话区发通知：Git 面板自己会显示「不是仓库」的占位。
                // 打开一个非 Git 文件夹是正常用法，不该报错。
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
        let Some(dir) = self.git_dir() else {
            self.notice = Some("还没有选择工作区".into());
            cx.notify();
            return;
        };
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
                self.remote_open_rel = None;
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

    /// 打开远程文件：读文件走 ssh，必须放后台线程，否则界面会卡住等网络。
    pub(crate) fn open_remote_file(&mut self, rel: String, cx: &mut Context<Self>) {
        let Some(fs) = self.remote_fs() else {
            return;
        };
        // 已经在编辑同一个文件：直接切到编辑器，不重新拉内容（否则丢失未保存改动）
        if self.remote_open_rel.as_deref() == Some(rel.as_str()) {
            self.center = CenterView::File;
            self.input_target = InputTarget::Editor;
            cx.notify();
            return;
        }
        let read_rel = rel.clone();
        self.remote_open_gen += 1;
        let open_gen = self.remote_open_gen;
        let (tx, rx) = flume::bounded::<Result<String, String>>(1);
        std::thread::spawn(move || {
            // 用 read_text_exact：超过上限直接报错，避免截断内容被保存写坏远端文件
            let _ = tx.send(fs.read_text_exact(&read_rel));
        });
        cx.spawn(async move |weak, async_cx| {
            if let Ok(msg) = rx.recv_async().await {
                let _ = weak.update(async_cx, |this, cx| {
                    // 期间又点了别的文件 / 切了工作区：只采用最后一次打开的请求
                    if this.remote_open_gen != open_gen {
                        return;
                    }
                    match msg {
                        Ok(content) => {
                            this.editor.reset(content);
                            this.editor.move_to(0, false);
                            this.scroll_editor_to_cursor();
                            this.remote_open_rel = Some(rel);
                            this.open_path = None;
                            this.center = CenterView::File;
                            this.dirty = false;
                            this.notice = None;
                            this.on_editor_opened();
                            cx.notify();
                        }
                        Err(e) => {
                            this.notice = Some(format!("无法读取远程文件：{e}"));
                            cx.notify();
                        }
                    }
                });
            }
        })
        .detach();
    }

    pub(crate) fn save_file(&mut self, cx: &mut Context<Self>) {
        if let Some(rel) = self.remote_open_rel.clone() {
            self.save_remote_file(rel, cx);
            return;
        }
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

    /// 远程保存：写回走 ssh，后台线程执行；回来后按内容判断是否仍是脏的。
    fn save_remote_file(&mut self, rel: String, cx: &mut Context<Self>) {
        let Some(fs) = self.remote_fs() else {
            self.notice = Some("远程连接已断开".into());
            cx.notify();
            return;
        };
        let content = self.editor.text().to_string();
        let sent = content.clone();
        let check_rel = rel.clone();
        let (tx, rx) = flume::bounded::<Result<String, String>>(1);
        std::thread::spawn(move || {
            let r = fs.write_text(&rel, &sent);
            let _ = tx.send(r.map(|()| sent));
        });
        cx.spawn(async move |weak, async_cx| {
            if let Ok(msg) = rx.recv_async().await {
                let _ = weak.update(async_cx, |this, cx| {
                    // 期间切了工作区 / 换了文件：这份结果不属于当前编辑器
                    if this.remote_open_rel.as_deref() != Some(check_rel.as_str()) {
                        return;
                    }
                    match msg {
                        Ok(saved) => {
                            // 写盘期间可能又改了：内容一致才算干净
                            if this.editor.text() == saved.as_str() {
                                this.dirty = false;
                            }
                            this.notice = Some("已保存（远程）".into());
                        }
                        Err(e) => this.notice = Some(format!("保存失败：{e}")),
                    }
                    cx.notify();
                });
            }
        })
        .detach();
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

    /// 切换到一个已知存在的目录作为工作区（子仓库入口用）。
    pub(crate) fn switch_workspace(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.enter_workspace(path, cx);
    }

    /// 手输路径确认（从工作区管理器底部的「打开文件夹…」进入）。
    pub(crate) fn apply_workspace_input(&mut self, cx: &mut Context<Self>) {
        let raw = self.ws_input.text().trim().to_string();
        if raw.is_empty() {
            return;
        }
        // 手输路径行也收 `ssh://` 连接串，与工作区管理器的「连接远程」一致。
        if neverwork_core::is_remote_url(&raw) {
            match neverwork_core::SshTarget::parse(&raw) {
                Ok(t) => {
                    self.ws_prompt_open = false;
                    self.connect_remote(t, None, cx);
                }
                Err(e) => {
                    self.notice = Some(format!("远程连接串无效：{e}"));
                    cx.notify();
                }
            }
            return;
        }
        self.enter_workspace(PathBuf::from(&raw), cx);
    }

    // ===== 命令面板 =====

    pub(crate) fn toggle_focus_mode(&mut self, cx: &mut Context<Self>) {
        self.focus_mode = !self.focus_mode;
        cx.notify();
    }

    pub(crate) fn toggle_palette(&mut self, cx: &mut Context<Self>) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            // 两个浮层不同时开：命令面板压住新建会话选择器
            self.new_session_picker = false;
        } else {
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
            Command::NewSession => self.open_new_session_picker(cx),
            Command::Changes => self.set_right_tab(RightTab::Git, cx),
            Command::Preview => self.set_right_tab(RightTab::Preview, cx),
            Command::SwitchWorkspace => self.open_workspace_prompt(cx),
            Command::FocusMode => {
                self.focus_mode = !self.focus_mode;
                cx.notify();
            }
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
            Command::SessionStatus(code) => {
                let status = match code {
                    1 => neverwork_core::SessionStatus::Todo,
                    2 => neverwork_core::SessionStatus::Done,
                    3 => neverwork_core::SessionStatus::Dropped,
                    _ => neverwork_core::SessionStatus::Idle,
                };
                let idx = self.active_session;
                self.set_session_status(idx, status, cx);
            }
            Command::ExportSession => {
                let idx = self.active_session;
                self.export_session(idx, cx);
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
            InputTarget::Workspace => {
                if self.ws_pw_focus {
                    &self.ws_pw_input
                } else {
                    &self.ws_input
                }
            }
            InputTarget::SessionSearch => &self.session_search,
            InputTarget::RenameSession => &self.rename_buf,
            InputTarget::Field(_) => &self.field_buf,
        }
    }

    fn buffer_mut(&mut self, target: InputTarget) -> &mut TextBuffer {
        match target {
            InputTarget::Chat => &mut self.chat,
            InputTarget::Editor => &mut self.editor,
            InputTarget::Workspace => {
                if self.ws_pw_focus {
                    &mut self.ws_pw_input
                } else {
                    &mut self.ws_input
                }
            }
            InputTarget::SessionSearch => &mut self.session_search,
            InputTarget::RenameSession => &mut self.rename_buf,
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
        if !self.settings.auto_save
            || (self.open_path.is_none() && self.remote_open_rel.is_none())
        {
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
        // 远程文件也走自动保存（内部已转后台线程）
        if let Some(rel) = self.remote_open_rel.clone() {
            self.save_remote_file(rel, cx);
            return;
        }
        if let Some(path) = self.open_path.clone() {
            if crate::filesystem::write_file(&path, self.editor.text()).is_ok() {
                self.dirty = false;
            }
        }
        cx.notify();
    }

    pub(crate) fn on_chat_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        self.switch_input_target(InputTarget::Chat);
        if ev.keystroke.key == "escape" {
            let mut closed = false;
            if self.thinking_panel {
                self.thinking_panel = false;
                // 面板关了就不再跟踪拖动，否则残留的 true 会让下一次鼠标移动改档位
                self.thinking_dragging = false;
                closed = true;
            }
            if self.composer_menu {
                self.composer_menu = false;
                closed = true;
            }
            if closed {
                cx.notify();
                return;
            }
        }
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
                if self.skill_search != value {
                    self.skill_search = value;
                    self.on_skill_search_changed(cx);
                }
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

    pub(crate) fn on_session_search_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        self.switch_input_target(InputTarget::SessionSearch);
        if let Some(action) = keymap::route(&ev.keystroke) {
            self.apply_action(InputTarget::SessionSearch, action, cx);
        }
    }

    pub(crate) fn on_rename_session_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        self.switch_input_target(InputTarget::RenameSession);
        if let Some(action) = keymap::route(&ev.keystroke) {
            self.apply_action(InputTarget::RenameSession, action, cx);
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
                // 工作区输入同时用于「切换到新路径」与「重命名工作区」两种场景
                InputTarget::Workspace => {
                    if self.ws_remote_open {
                        self.connect_remote_from_input(cx);
                    } else if self.ws_rename.is_some() {
                        self.commit_rename_workspace(cx);
                    } else {
                        self.apply_workspace_input(cx);
                    }
                }
                // 搜索是实时的，回车只收起焦点，不改词
                InputTarget::SessionSearch => {
                    self.input_target = InputTarget::Chat;
                    cx.notify();
                }
                InputTarget::RenameSession => self.commit_rename_session(cx),
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
                if self.market_detail.is_some() {
                    // 详情抽屉在最上层，Esc 先关它
                    self.close_market_detail(cx);
                } else if self.workspace_popup {
                    self.close_workspace_prompt(cx);
                } else if self.new_session_picker {
                    self.close_new_session_picker(cx);
                } else if self.model_picker {
                    self.close_chat_model_picker(cx);
                } else if self.ws_prompt_open {
                    self.ws_prompt_open = false;
                    self.palette_open = false;
                } else if self.palette_open {
                    self.palette_open = false;
                } else if self.rename_session.is_some() {
                    self.cancel_rename_session(cx);
                } else if self.session_menu.is_some() {
                    // 先收菜单，再收确认态
                    self.close_session_menu(cx);
                } else if self.archive_clean_confirm {
                    self.archive_clean_confirm = false;
                } else if !self.session_search.text().trim().is_empty() {
                    self.session_search.reset("");
                } else if self.role_skill_picker.is_some() {
                    self.close_role_skill_picker(cx);
                } else if self.streaming {
                    self.stop_streaming(cx);
                }
                // 浮层/路径行可能刚刚关闭：把输入目标拉回正文。
                self.restore_input_target();
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
        let session_idx = self.active_session.min(self.sessions.len().saturating_sub(1));
        // 厂商/模型按会话取：会话里没选过就回落设置里的全局当前厂商。
        let Some((provider_id, config)) = self.provider_config_for(session_idx) else {
            self.notice = Some(
                "还没有可用的模型：打开「设置 → 模型与厂商」填入 API Key 即可开始对话".into(),
            );
            cx.notify();
            return;
        };

        // 团队配置随取随用；把角色绑定的插件摘要拼进 prompt（仅内存，不回写配置）。
        let mut team = self
            .config
            .active_team()
            .cloned()
            .unwrap_or_else(neverwork_core::Team::default_team);
        self.enrich_team_skills(&mut team);
        // 模式在新建会话时选定，不能切换；需要另一种就新建一个会话。
        let team_mode = self.session_mode().is_team();

        self.chat.reset("");
        self.notice = None;
        self.pending = None;
        self.center = CenterView::Chat;

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
            session.team_name = if team_mode {
                team.name.clone()
            } else {
                "单助手".to_string()
            };
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

        // 三种走法，按「会话模式 + 工具开关」分：
        //
        // 1. 单助手会话（`SessionMode::Solo`）：**一个**助手带工具。
        //    它能问答、能读文件、能改代码，由模型自己判断这次要不要动工具。
        // 2. 团队会话 + 开工具：多角色串行带工具，前一个角色的产出作为
        //    后一个的上下文，写操作逐个弹审批。
        // 3. 团队会话 + 关工具：多角色纯文本轮流发言。
        if team_mode && self.tools_enabled {
            let mut roles = team.roles.clone();
            if roles.is_empty() {
                roles.push(neverwork_core::Role::new("agent", "Assistant", "A"));
            }
            // 模型与厂商以本会话的选择为准（会话没选过才回落全局默认）；
            // role.model 只是展示标签，不参与路由。
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
        } else if team_mode {
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
            // 与具体诉求无关的角色约束把问答带偏。但已安装的文本技能仍要
            // 注入，否则装完技能对话没有任何变化（看起来像假安装）。
            let mut solo = neverwork_core::Role::new("agent", "Assistant", "A");
            let installed_ids: Vec<String> = self
                .settings
                .skills
                .iter()
                .filter(|s| s.installed && !s.is_mcp())
                .map(|s| s.id.clone())
                .collect();
            if !installed_ids.is_empty() {
                solo.prompt.push_str(&role_skills_section(
                    &installed_ids,
                    &self.settings.skills,
                    &self.known_skills(),
                ));
            }
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

    /// 把角色绑定的插件（技能）内容拼进 prompt。
    ///
    /// `Role.skills` 存的是技能 id：
    /// - 文本技能：读 `skills/<id>/SKILL.md` 的正文整段注入（未落盘时回退目录简介）；
    /// - MCP 插件：告知可用工具前缀，模型需要时直接调用（进程由 mcp 模块按需拉起）。
    /// 只在内存里的团队副本上追加，不回写配置。
    fn enrich_team_skills(&self, team: &mut neverwork_core::Team) {
        let catalog = self.known_skills();
        for role in &mut team.roles {
            let section = role_skills_section(&role.skills, &self.settings.skills, &catalog);
            if !section.is_empty() {
                role.prompt.push_str(&section);
            }
        }
    }

    /// 在团队编辑器里给角色挂一个插件：没装过先安装，再记到 `Role.skills`。
    /// MCP 插件需要用户确认（启动外部进程），确认通过后自动绑到这个角色；
    /// skills.sh 条目正文要等安装时从上游解析，走远程安装并记下绑定。
    pub(crate) fn attach_skill_to_role(
        &mut self,
        role_idx: usize,
        skill: neverwork_core::Skill,
        cx: &mut Context<Self>,
    ) {
        if !self.is_skill_installed(&skill.id) {
            if skill.is_mcp() {
                self.pending_mcp_attach = Some(role_idx);
                self.install_skill(skill, cx);
                return;
            }
            if let Some(entry) = self
                .market_entry_for(&skill.id)
                .filter(|e| e.remote.is_some() && !e.remote_only)
            {
                self.install_remote_entry(entry, Some(role_idx), cx);
                return;
            }
            self.install_skill(skill.clone(), cx);
        }
        self.bind_skill_to_role(role_idx, &skill.id, &skill.name, cx);
    }

    /// 把已安装的技能记到角色的 `Role.skills`（内存，点「保存」才写盘）。
    pub(crate) fn bind_skill_to_role(
        &mut self,
        role_idx: usize,
        id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let ti = self.active_team_index();
        if let Some(team) = self.config.teams.get_mut(ti) {
            if let Some(role) = team.roles.get_mut(role_idx) {
                if !role.skills.iter().any(|x| x == id) {
                    role.skills.push(id.to_string());
                }
            }
        }
        self.notice = Some(format!("已给角色添加插件「{name}」，点「保存」写盘"));
        cx.notify();
    }

    pub(crate) fn detach_skill_from_role(
        &mut self,
        role_idx: usize,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        let ti = self.active_team_index();
        if let Some(team) = self.config.teams.get_mut(ti) {
            if let Some(role) = team.roles.get_mut(role_idx) {
                role.skills.retain(|x| x != id);
            }
        }
        cx.notify();
    }

    /// 所有已知技能（已安装 + 内建 + 各来源市场条目），按 id 去重。
    ///
    /// 市场数据来自真实源，随刷新变化；注入提示词、按 id 查配置都走这里。
    pub(crate) fn known_skills(&self) -> Vec<neverwork_core::Skill> {
        let mut out: Vec<neverwork_core::Skill> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for s in self
            .settings
            .skills
            .iter()
            .cloned()
            .chain(crate::skills_catalog::builtin())
        {
            if seen.insert(s.id.clone()) {
                out.push(s);
            }
        }
        for entries in self.market_entries.values() {
            for e in entries {
                if seen.insert(e.skill.id.clone()) {
                    out.push(e.skill.clone());
                }
            }
        }
        out
    }

    /// 按 id 找市场条目（任意来源）。
    ///
    /// 团队插件面板靠它拿真实图标与来源链接；本地导入 / 内建条目
    /// 可能没有对应市场条目，调用方回退字形与「本地」徽标。
    pub(crate) fn market_entry_for(&self, id: &str) -> Option<MarketEntry> {
        self.market_entries
            .values()
            .flatten()
            .find(|e| e.skill.id == id)
            .cloned()
    }

    /// 当前来源的市场条目（内建示例只在官方来源里合并展示）。
    pub(crate) fn current_market_items(&self) -> Vec<MarketEntry> {
        let mut out: Vec<MarketEntry> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        if self.market_source == MarketSource::Official {
            for s in crate::skills_catalog::builtin() {
                if seen.insert(s.id.clone()) {
                    let mut e = MarketEntry::new(s);
                    // 内建示例真实来自 npm 包 server-everything
                    e.tags = vec!["npm".into()];
                    out.push(e);
                }
            }
        }
        if let Some(list) = self.market_entries.get(&self.market_source) {
            for e in list {
                if seen.insert(e.skill.id.clone()) {
                    out.push(e.clone());
                }
            }
        }
        out
    }

    /// 按 id 找技能的展示信息：已安装优先，其次内建与各来源市场条目。
    pub(crate) fn find_skill(&self, id: &str) -> Option<neverwork_core::Skill> {
        self.known_skills().into_iter().find(|s| s.id == id)
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
        // 远程工作区走 ssh 后端；本地 / 无工作区保持原语义。
        let remote_auth = self.remote_auth();
        let ctx = match (&self.remote, &self.root) {
            (Some(target), _) => {
                neverwork_agent::tools::ToolContext::remote_auth(target.clone(), remote_auth)
            }
            (None, Some(root)) => neverwork_agent::tools::ToolContext::new(root.clone()),
            (None, None) => neverwork_agent::tools::ToolContext::unrestricted(),
        };
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
                            ctx.clone(),
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
                            ctx.clone(),
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
                            // 顶部流水线跟着当前发言角色高亮
                            this.active_role = Some(ri);
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
                        round,
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
                                    thread: Some(ri),
                                    round: Some(round),
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
                this.active_role = None;
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
                            this.active_role = Some(role_index);
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
                            this.active_role = None;
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

    /// 收起 / 展开会话列表（图标栏永远保留）。
    pub(crate) fn toggle_session_list(&mut self, cx: &mut Context<Self>) {
        self.session_collapsed = !self.session_collapsed;
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
        self.active_role = None;
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
        } else if self.session_collapsed {
            // 会话列表收起时只留一条 16px 窄边（展开按钮），和右栏收起一致。
            crate::views::sidebar::RAIL_W + 16.0
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
        // 渲染 clamp：路径行与重命名行都不在时，`Workspace` 目标不能留。
        // 否则焦点每帧被拉向无节点句柄，键盘输入全部丢失。
        if self.input_target == InputTarget::Workspace
            && !self.ws_prompt_open
            && !self.ws_remote_open
            && self.ws_rename.is_none()
        {
            self.restore_input_target();
        }
        {
            let want = match self.input_target {
                InputTarget::Editor | InputTarget::Field(_) => Some(self.editor_focus.clone()),
                InputTarget::Workspace => Some(self.ws_focus.clone()),
                InputTarget::Chat
                | InputTarget::SessionSearch
                | InputTarget::RenameSession => Some(self.focus_handle.clone()),
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
            // 表单字段「点外面自动提交」：气泡阶段根节点最后收到鼠标按下，
            // 此时子元素的点击（切换卡片、开字段…）都已处理完。
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, ev: &gpui::MouseDownEvent, _w, cx| {
                    this.commit_field_clicked_away(ev.position, cx);
                }),
            )
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
            .child(crate::views::titlebar(self, window.is_maximized(), cx))
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

        // 会话右键菜单：坐标 clamp 进可见区域，靠下/靠右时往上/左弹。
        if let (Some(idx), Some((x, y))) = (self.session_menu, self.session_menu_at) {
            // 宽度与菜单卡片共用同一个常量；高度是估计值（含标题行/分组分隔线）
            const MENU_W: f32 = crate::views::session_menu::MENU_W;
            const MENU_H: f32 = 380.0;
            let mx = if x + MENU_W + 8.0 <= visible_w {
                x
            } else {
                (x - MENU_W).max(8.0)
            };
            let my = if y + MENU_H + 8.0 <= visible_h {
                y
            } else {
                (y - MENU_H).max(8.0)
            };
            root = root.child(self.session_menu_overlay(idx, mx, my, cx).into_any_element());
        }
        if self.palette_open {
            root = root.child(self.palette_overlay(cx).into_any_element());
        }
        // 工作区管理器：与命令面板同形态的居中遮罩卡片。
        if self.workspace_popup {
            root = root.child(self.workspace_popup_overlay(cx).into_any_element());
        }
        // 条目详情：从右侧抽出 420px 抽屉（技能页里点卡片打开）。
        if self.market_detail.is_some() {
            root = root.child(self.market_detail_overlay(cx).into_any_element());
        }
        // 新建会话：先选运行模式（单助手 / 团队），选定后不可切换。
        if self.new_session_picker {
            root = root.child(self.new_session_overlay(cx).into_any_element());
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
        // 表单字段（设置页 / 查找框 / 技能搜索…）全部 track `editor_focus`，
        // 且中栏是单选渲染——设置页打开时对话输入框不在渲染树里，
        // 把字段归到 chat handle 会让焦点落在一个无节点的句柄上，
        // 按键派发回退到 root，打字与粘贴全部无效。
        InputTarget::Editor | InputTarget::Field(_) => editor_focus.clone(),
        InputTarget::Workspace => ws_focus.clone(),
        InputTarget::Chat
        | InputTarget::SessionSearch
        | InputTarget::RenameSession => chat_focus.clone(),
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
///
/// 返回停止信号：切工作区时先丢掉旧的（线程随之退出），再起新的；
/// 无工作区返回 `None`，不起线程。
fn start_watcher_opt(
    root: Option<PathBuf>,
    cx: &mut Context<AppState>,
) -> Option<flume::Sender<()>> {
    use std::time::Duration;
    let root = root?;
    let (tx, rx) = flume::unbounded::<()>();
    let (stop_tx, stop_rx) = flume::unbounded::<()>();
    std::thread::spawn(move || {
        let (etx, erx) = std::sync::mpsc::channel::<()>();
        let mut watcher = match notify::recommended_watcher(
            move |res: notify::Result<notify::Event>| {
                if res.is_ok() {
                    let _ = etx.send(());
                }
            },
        ) {
            Ok(w) => w,
            Err(_) => return,
        };
        use notify::Watcher;
        if watcher.watch(&root, notify::RecursiveMode::Recursive).is_err() {
            return;
        }
        loop {
            // 停止信号（信号 Sender 被丢弃也算）
            match stop_rx.try_recv() {
                Ok(()) | Err(flume::TryRecvError::Disconnected) => break,
                Err(flume::TryRecvError::Empty) => {}
            }
            match erx.recv_timeout(Duration::from_millis(200)) {
                Ok(()) => {
                    // 合并这一波连续改动
                    while erx.recv_timeout(Duration::from_millis(300)).is_ok() {}
                    let _ = tx.send(());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    cx.spawn(async move |weak, async_cx| {
        while rx.recv_async().await.is_ok() {
            let _ = weak.update(async_cx, |this, cx| {
                if let Some(root) = this.root.clone() {
                    this.files = filesystem::build_tree(&root, &this.expanded).unwrap_or_default();
                    cx.notify();
                }
            });
        }
    })
    .detach();
    Some(stop_tx)
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

/// 生成「已绑定插件」的 prompt 段落（空串 = 没绑插件）。
///
/// 单独抽出来是为了能脱离 `AppState` 单测：已安装信息优先、
/// 内置目录回退，查不到的 id 也保留（至少让模型知道有这么个插件）。
fn role_skills_section(
    ids: &[String],
    installed: &[neverwork_core::Skill],
    catalog: &[neverwork_core::Skill],
) -> String {
    if ids.is_empty() {
        return String::new();
    }
    let mut s = String::from("\n\n已绑定插件（技能），优先用其中的方法完成任务：");
    for id in ids {
        let found = installed
            .iter()
            .find(|x| &x.id == id)
            .or_else(|| catalog.iter().find(|x| &x.id == id));
        // MCP 插件：告诉模型可用前缀，工具描述里已有参数，不必展开正文
        if found.map(|sk| sk.is_mcp()).unwrap_or(false) {
            let sk = found.unwrap();
            let prefix = sk
                .mcp
                .as_ref()
                .map(|m| format!("mcp__{}__", m.sanitized()))
                .unwrap_or_else(|| "mcp__".into());
            s.push_str(&format!(
                "\n- {}（MCP 插件，工具前缀 {}，需要时直接调用）",
                sk.name.trim(),
                prefix
            ));
            continue;
        }
        // 真实安装的技能：把落盘正文带上（截断防止塞满上下文）
        if let Some(body) = crate::skills_ops::read_body(id) {
            let name = found
                .map(|sk| sk.name.trim().to_string())
                .unwrap_or_else(|| id.clone());
            s.push_str(&format!("\n\n### {name}\n{}", truncate_chars(&body, 1500)));
            continue;
        }
        match found {
            Some(sk) if !sk.summary.trim().is_empty() => {
                s.push_str(&format!("\n- {}：{}", sk.name.trim(), sk.summary.trim()));
            }
            Some(sk) => s.push_str(&format!("\n- {}", sk.name.trim())),
            None => s.push_str(&format!("\n- {id}")),
        }
    }
    s
}

/// 按字符截断（中文安全），超长补省略号。
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod role_skills_tests {
    use super::{role_skills_section, truncate_chars};
    use neverwork_core::{McpServerConfig, Skill, SkillCategory, SkillKind};

    fn sk(id: &str, name: &str, summary: &str) -> Skill {
        Skill {
            id: id.into(),
            name: name.into(),
            summary: summary.into(),
            detail: String::new(),
            category: SkillCategory::Dev,
            author: String::new(),
            installs: 0,
            stars: 0,
            glyph: String::new(),
            installed: true,
            kind: SkillKind::Text,
            mcp: None,
        }
    }

    fn mcp_sk(id: &str, name: &str, server: &str) -> Skill {
        Skill {
            kind: SkillKind::Mcp,
            mcp: Some(McpServerConfig {
                name: server.into(),
                command: "npx".into(),
                ..Default::default()
            }),
            ..sk(id, name, "")
        }
    }

    #[test]
    fn no_skills_means_no_section() {
        assert!(role_skills_section(&[], &[], &[]).is_empty());
    }

    #[test]
    fn installed_skill_wins_over_catalog() {
        let installed = vec![sk("a", "已装版", "安装后的简介")];
        let catalog = vec![sk("a", "市场版", "市场简介")];
        let s = role_skills_section(&["a".into()], &installed, &catalog);
        assert!(s.contains("已装版：安装后的简介"), "{s}");
        assert!(!s.contains("市场版"), "{s}");
    }

    #[test]
    fn catalog_is_fallback_for_uninstalled() {
        let catalog = vec![sk("req", "需求分析", "把需求拆成验收标准")];
        let s = role_skills_section(&["req".into()], &[], &catalog);
        assert!(s.contains("需求分析：把需求拆成验收标准"), "{s}");
    }

    #[test]
    fn unknown_id_is_kept_as_bare_name() {
        let s = role_skills_section(&["ghost".into()], &[], &[]);
        assert!(s.contains("- ghost"), "{s}");
        assert!(s.contains("已绑定插件"), "{s}");
    }

    #[test]
    fn mcp_plugin_reports_tool_prefix() {
        let installed = vec![mcp_sk("mcp.everything", "MCP 示例服务器", "everything")];
        let s = role_skills_section(&["mcp.everything".into()], &installed, &[]);
        assert!(s.contains("MCP 插件"), "{s}");
        assert!(s.contains("mcp__everything__"), "{s}");
    }

    #[test]
    fn truncate_chars_is_cjk_safe() {
        assert_eq!(truncate_chars("需求分析", 10), "需求分析");
        let cut = truncate_chars("需求分析需求分析", 4);
        assert_eq!(cut, "需求分析…");
        assert_eq!(cut.chars().count(), 5);
    }
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
                thread: None,
                round: None,
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

#[cfg(test)]
mod export_tests {
    use super::{safe_file_name, session_markdown};
    use neverwork_core::Session;

    fn session(json: &str) -> Session {
        serde_json::from_str(json).expect("会话 JSON 应能反序列化")
    }

    #[test]
    fn safe_file_name_strips_windows_illegal_chars() {
        let n = safe_file_name("a/b:c*d?e|f\\g<h>i\"j", "s1");
        assert!(
            !n.chars()
                .any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')),
            "文件名里还有 Windows 非法字符：{n}"
        );
        assert!(n.ends_with("-s1"));
    }

    #[test]
    fn safe_file_name_falls_back_when_title_is_all_punctuation() {
        assert_eq!(safe_file_name("///", "s9"), "session-s9");
    }

    #[test]
    fn markdown_keeps_user_and_agent_headers() {
        let s = session(
            r#"{"id":"s1","title":"修一个 bug","subtitle":"","team_name":"单助手",
                 "messages":[
                   {"kind":{"type":"user"},"body":"帮我看下","code":null},
                   {"kind":{"type":"agent","participant":{"role_id":"a","role_name":"助手","color_index":0,"initial":"A"}},"body":"好的","code":null}
                 ],
                 "updated_label":"刚刚"}"#,
        );
        let md = session_markdown(&s);
        assert!(md.starts_with("# 修一个 bug"));
        assert!(md.contains("## 用户"));
        assert!(md.contains("## 助手"));
    }
}

#[cfg(test)]
mod session_inherit_tests {
    use super::{blank_session, inherit_session_model};
    use neverwork_core::{Config, ProviderEntry, ProviderPreset, SessionMode};

    fn session_with(pid: &str, model: &str) -> neverwork_core::Session {
        let mut s = blank_session(0, &Config::default(), SessionMode::Solo);
        s.provider_id = pid.into();
        s.model = model.into();
        s
    }

    /// 填了 Key 的厂商（没 Key 的 Inherit 会被跳过）。
    fn ready_provider(preset: ProviderPreset) -> ProviderEntry {
        let mut p = ProviderEntry::from_preset(preset);
        p.api_key = "sk-test".into();
        p
    }

    #[test]
    fn inherits_when_provider_ready() {
        let providers = vec![ready_provider(ProviderPreset::DeepSeek)];
        let src = session_with("deepseek", "deepseek-flash");
        let (pid, model) = inherit_session_model(Some(&src), &providers);
        assert_eq!(pid, "deepseek");
        assert_eq!(model, "deepseek-flash");
    }

    #[test]
    fn skips_provider_without_key() {
        // 旧会话里存着没填 Key 的厂商时，新会话不继承，跟全局默认走。
        let providers = vec![ProviderEntry::from_preset(ProviderPreset::OpenAi)];
        let src = session_with("openai", "gpt-4o");
        assert_eq!(
            inherit_session_model(Some(&src), &providers),
            (String::new(), String::new())
        );
    }

    #[test]
    fn follows_default_when_source_never_chose() {
        let providers = vec![ProviderEntry::from_preset(ProviderPreset::DeepSeek)];
        let src = session_with("", "");
        assert_eq!(
            inherit_session_model(Some(&src), &providers),
            (String::new(), String::new())
        );
    }

    #[test]
    fn falls_back_when_provider_disabled_or_removed() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::DeepSeek);
        p.enabled = false;
        let src = session_with("deepseek", "deepseek-flash");
        assert!(inherit_session_model(Some(&src), &[p]).0.is_empty());
        assert!(inherit_session_model(Some(&src), &[]).0.is_empty());
        assert_eq!(inherit_session_model(None, &[]), (String::new(), String::new()));
    }
}
