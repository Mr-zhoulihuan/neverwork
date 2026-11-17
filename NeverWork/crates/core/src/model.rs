//! 领域模型：工作区、团队、角色、会话、消息。
//!
//! 纯数据结构，不含 UI，便于持久化（JSON）与测试。

use serde::{Deserialize, Serialize};

// ============ 工作区连接 ============

/// 工作区连接方式。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// 本机（第一期默认）。
    Local,
    /// 远程（ssh 或自建服务）。
    Remote,
}

/// 一个工作区（本地或远程）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub path: String,
    pub kind: ConnectionKind,
    /// 远程地址，如 `ssh://office:22/~/payments`。
    pub remote_url: Option<String>,
    pub online: bool,
}

impl Workspace {
    pub fn local(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = path
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or("workspace")
            .to_string();
        Self {
            name,
            path,
            kind: ConnectionKind::Local,
            remote_url: None,
            online: true,
        }
    }
}

// ============ 多 Agent 团队 ============

/// 编排模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowMode {
    /// 流水线：按顺序转交。
    Pipeline,
    /// 并行：多个角色同时跑。
    Parallel,
    /// 主管指派：由 lead 动态分派。
    Supervisor,
}

impl FlowMode {
    pub fn label(&self) -> &'static str {
        match self {
            FlowMode::Pipeline => "流水线",
            FlowMode::Parallel => "并行",
            FlowMode::Supervisor => "主管指派",
        }
    }
}

/// 一个 Agent 角色（可自定义）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    /// 角色类型标签，如"规划者"。
    pub kind_label: String,
    /// 使用的模型。
    pub model: String,
    /// 绑定的已配置厂商 id；空 = 跟随会话默认。
    #[serde(default)]
    pub provider_id: String,
    /// 系统提示词 / 职责。
    pub prompt: String,
    /// 身份色索引（对应 theme::role_color）。
    pub color_index: usize,
    /// 头像首字母。
    pub initial: String,
    /// 绑定的技能（可选）。
    pub skills: Vec<String>,
}

impl Role {
    pub fn new(id: impl Into<String>, name: impl Into<String>, initial: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind_label: String::new(),
            model: String::new(),
            provider_id: String::new(),
            prompt: String::new(),
            color_index: 0,
            initial: initial.into(),
            skills: Vec::new(),
        }
    }
}

/// 一个 Agent 团队（一组可协作的角色）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub flow: FlowMode,
    pub roles: Vec<Role>,
}

impl Team {
    /// 按名字查找角色下标（trim 后 ASCII 忽略大小写）。
    ///
    /// 角色名是展示标签，`@` 提及、`handoff.to` 都靠它定位，
    /// 统一走这里避免各处各写一套匹配。
    pub fn role_by_name(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.roles
            .iter()
            .position(|r| r.name.trim().eq_ignore_ascii_case(name))
    }

    /// 默认协作团队（Planner / Coder / Reviewer 作为示例）。
    pub fn default_team() -> Self {
        Self {
            id: "default".into(),
            name: "默认协作团队".into(),
            flow: FlowMode::Pipeline,
            roles: vec![
                Role {
                    id: "planner".into(),
                    name: "Planner".into(),
                    kind_label: "规划者".into(),
                    model: "Claude Opus".into(),
                    provider_id: String::new(),
                    prompt: "负责拆解任务、制定步骤。".into(),
                    color_index: 0,
                    initial: "P".into(),
                    skills: vec![],
                },
                Role {
                    id: "coder".into(),
                    name: "Coder".into(),
                    kind_label: "编码".into(),
                    model: "GPT-5".into(),
                    provider_id: String::new(),
                    prompt: "负责实现代码改动。".into(),
                    color_index: 1,
                    initial: "C".into(),
                    skills: vec!["rust-dev".into()],
                },
                Role {
                    id: "reviewer".into(),
                    name: "Reviewer".into(),
                    kind_label: "审查者".into(),
                    model: "GPT-5-mini".into(),
                    provider_id: String::new(),
                    prompt: "负责审查正确性、边界与性能。".into(),
                    color_index: 2,
                    initial: "R".into(),
                    skills: vec![],
                },
            ],
        }
    }
}

/// 判断一个字符能不能作为 `@角色名` 的左右边界。
///
/// 包含中英文标点：邮箱式 `a@b.com` 的 `@` 前是字母，不应命中。
fn is_mention_boundary(c: char) -> bool {
    c.is_whitespace()
        || c.is_ascii_punctuation()
        || "，。！？；：、（）【】「」『』《》〈〉…—～·".contains(c)
}

/// 角色名后是否已结束（或跟着合法边界）。
///
/// 特例：`.` 后紧跟字母/数字视为文件扩展名，`@需求验证.md` 不算提及。
fn mention_ends_at(text: &str, end: usize) -> bool {
    let mut rest = text[end..].chars();
    match rest.next() {
        None => true,
        Some('.') => match rest.next() {
            Some(n) if n.is_alphanumeric() => false,
            _ => true,
        },
        Some(c) => is_mention_boundary(c),
    }
}

/// 从用户文本里解析 `@角色名`，返回命中的角色下标。
///
/// 规则（与设计文档一致）：
/// - `@` 前必须是行首、空白或标点，避免邮箱式误匹配；
/// - 角色名后必须是空白、标点或文本结尾；
/// - 前缀重叠取最长（`接口成员` 胜过 `接口`）；
/// - 多处命中取最早出现，同长同位置按角色列表顺序取第一个。
///
/// 只做入口路由：消息里后续的 `@其他角色` 是给模型看的正文，
/// 不改变入口。
pub fn parse_role_mention(text: &str, roles: &[Role]) -> Option<usize> {
    for (at, c) in text.char_indices() {
        if c != '@' {
            continue;
        }
        if at > 0 {
            let prev = text[..at].chars().next_back().unwrap_or(' ');
            if !is_mention_boundary(prev) {
                continue;
            }
        }
        let mut best: Option<(usize, usize)> = None; // (名字长度, 角色下标)
        for (ri, role) in roles.iter().enumerate() {
            let name = role.name.trim();
            if name.is_empty() {
                continue;
            }
            let end = at + 1 + name.len();
            if end > text.len() || !text.is_char_boundary(end) {
                continue;
            }
            if !text[at + 1..end].eq_ignore_ascii_case(name) {
                continue;
            }
            if !mention_ends_at(text, end) {
                continue;
            }
            // 同位置取最长；同长保留列表里靠前的角色。
            if best.map_or(true, |(len, _)| name.len() > len) {
                best = Some((name.len(), ri));
            }
        }
        if let Some((_, ri)) = best {
            return Some(ri);
        }
    }
    None
}

// ============ 会话与消息 ============

/// 会话中的一个参与者（某个角色的一个实例）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    pub role_id: String,
    pub role_name: String,
    pub color_index: usize,
    pub initial: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageKind {
    /// 用户发言。
    User,
    /// 某个 Agent 发言。
    Agent { participant: Participant },
    /// 角色转交提示。
    Handoff {
        from: String,
        to: String,
        /// 一行摘要（卡片默认显示）。
        action: String,
        /// 完整交接载荷文本（卡片展开、历史回放）。旧会话没有该字段，
        /// 反序列化为空串。
        #[serde(default)]
        detail: String,
    },
    /// 工具调用结果。
    Tool { summary: String, files: Vec<String> },
    /// 并行执行状态。
    Parallel { items: Vec<ParallelItem> },
    /// Agent 发起的一次工具调用（含写操作的 diff 与审批状态）。
    ToolCall {
        /// 工具名，如 `write_file`。
        name: String,
        /// 模型给的调用 id。回放历史时要用它与 tool 消息严格配对，
        /// 少一条配对 provider 就整轮 400。旧配置没有这个字段（空串），
        /// 这类老记录只能退化成摘要。
        #[serde(default)]
        call_id: String,
        /// 模型给的原始参数 JSON。回放历史时一并带上，模型才看得到
        /// 当时用的 offset / query / pattern 等细节。
        #[serde(default)]
        arguments: String,
        /// 目标文件（相对工作区），只读工具也可能为空。
        #[serde(default)]
        path: String,
        /// 参数摘要，如 `src/main.rs（写入 12 行）`。
        summary: String,
        status: ToolRunStatus,
        /// 发起这次调用的角色序号（单 Agent 恒 0）。
        /// 旧会话没有该字段：`None` 表示未知，统计行里直接不显示线程数。
        #[serde(default)]
        thread: Option<usize>,
        /// 该角色内的第几次模型调用（0 起）。旧会话为 `None`，不显示轮次。
        #[serde(default)]
        round: Option<usize>,
        /// 执行输出 / 错误信息。
        output: String,
        /// 变化内容（写操作为 diff，读操作为空）。
        diff: Vec<DiffLineView>,
        added: u32,
        removed: u32,
    },
}

/// 工具调用的执行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRunStatus {
    /// 正在执行。
    Running,
    /// 等待用户批准（写操作）。
    Awaiting,
    /// 成功。
    Ok,
    /// 用户拒绝。
    Denied,
    /// 失败。
    Failed,
    /// 用户中断了整条生成，这条工具不会再返回结果。
    Cancelled,
}

impl ToolRunStatus {
    pub fn label(&self) -> &'static str {
        match self {
            ToolRunStatus::Running => "执行中",
            ToolRunStatus::Awaiting => "待确认",
            ToolRunStatus::Ok => "完成",
            ToolRunStatus::Denied => "已拒绝",
            ToolRunStatus::Failed => "失败",
            ToolRunStatus::Cancelled => "已中断",
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, ToolRunStatus::Running | ToolRunStatus::Awaiting)
    }
}

/// diff 的一行（界面展示用，与 agent 内部的 DiffLine 解耦）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLineView {
    /// `+` / `-` / ` `。
    pub marker: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelItem {
    pub role_name: String,
    pub color_index: usize,
    pub initial: String,
    pub task: String,
    pub done: bool,
}

/// 一条用户消息携带的附件。
///
/// 只存路径与类型，正文/图片字节在发送或渲染时再读：
/// - 图片：随消息以多模态图片块发给 Provider；
/// - 文本文件：发送时把内容拼进提示词（大文件截断）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// 展示名（原文件名）。
    pub name: String,
    /// 本地绝对路径。
    pub path: String,
    /// MIME，例如 `image/png` / `text/plain`。
    #[serde(default)]
    pub mime: String,
    /// 附件类型。
    #[serde(default)]
    pub kind: AttachmentKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    /// 图片：随消息发给支持视觉的模型。
    #[default]
    Image,
    /// 文本文件：内容拼进提示词。
    Text,
}

/// Agent 生成的一个产物（写入的文件、生成的图片）。
///
/// 对话输出底部的「产物」条与右栏「产物」面板共用这份数据：
/// 只记路径与类型，缩略图/内容在渲染时再读。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// 展示名（文件名）。
    pub name: String,
    /// 本地绝对路径；远程会话暂为工作区相对路径。
    pub path: String,
    /// 产物类型。
    #[serde(default)]
    pub kind: ArtifactKind,
    /// 生成时间（Unix 秒）。列表按它倒序。
    #[serde(default)]
    pub at: u64,
    /// 生成它的角色序号（工具消息的 thread），0 = 未知。
    #[serde(default)]
    pub thread: usize,
}

impl Artifact {
    /// 从路径构造：名字取最后一段，类型按扩展名判断。
    pub fn from_path(path: &str, at: u64, thread: usize) -> Self {
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(path)
            .to_string();
        Self {
            name,
            path: path.to_string(),
            kind: ArtifactKind::of_path(path),
            at,
            thread,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// 文本 / 代码 / 数据文件。
    #[default]
    File,
    /// 图片。
    Image,
}

impl ArtifactKind {
    /// 按扩展名判断产物类型；图片集合与媒体查看器一致
    ///（gpui 的 `img()` 能解码的全部格式 + SVG）。
    pub fn of_path(path: &str) -> Self {
        let ext = std::path::Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if is_image_extension(&ext) {
            ArtifactKind::Image
        } else {
            ArtifactKind::File
        }
    }
}

/// 图片扩展名（小写、不含点）：`img()` 能解码的全部格式。
pub fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png"
            | "jpg"
            | "jpeg"
            | "jpe"
            | "jfif"
            | "gif"
            | "webp"
            | "bmp"
            | "ico"
            | "svg"
            | "tif"
            | "tiff"
            | "tga"
            | "qoi"
            | "pnm"
            | "pbm"
            | "pgm"
            | "ppm"
            | "hdr"
            | "exr"
            | "dds"
            | "ff"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub kind: MessageKind,
    /// 正文（Markdown/富文本的简化：纯文本 + 代码块）。
    pub body: String,
    /// 代码块（可选）。
    pub code: Option<CodeBlock>,
    /// 用户消息的附件（旧消息没有该字段时为空）。
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// 这一轮生成耗时（毫秒）。0 表示还没结束或未记录。
    #[serde(default)]
    pub elapsed_ms: u64,
    /// 这一轮的真实输入 token（Provider 报的值，0 = 未报）。
    ///
    /// 成本口径用 token 不用货币：单价随模型/折扣变，存下来的美元数字
    /// 第二天就可能全是错的。
    #[serde(default)]
    pub tokens_in: u64,
    /// 这一轮的真实输出 token。
    #[serde(default)]
    pub tokens_out: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    pub filename: String,
    pub added: u32,
    pub removed: u32,
    pub code: String,
}

/// 会话执行状态（决定左侧会话卡片的徽标与状态胶囊）。
///
/// `Working` / `Done` 由发送流程自动流转；`Todo` / `Dropped` 只能由用户
/// 手动标记（卡片 ⋯ 菜单或命令面板）。生成中不允许手动改状态——运行态的
/// 优先级高于标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// 未开始 / 普通会话。
    #[default]
    Idle,
    /// 待执行（用户标记的待办）。
    Todo,
    /// 正在生成（工作中…）。
    Working,
    /// 已完成。
    Done,
    /// 已废弃。
    Dropped,
}

impl SessionStatus {
    /// 中文短标签（卡片胶囊 / 菜单 / 提示统一用它）；`Idle` 无标记，返回空串。
    pub fn label(self) -> &'static str {
        match self {
            SessionStatus::Idle => "",
            SessionStatus::Todo => "待执行",
            SessionStatus::Working => "进行中",
            SessionStatus::Done => "已完成",
            SessionStatus::Dropped => "已废弃",
        }
    }

    /// 主色（24 位 RGB），胶囊底色/文字共用一套。
    pub fn hue(self) -> u32 {
        match self {
            SessionStatus::Idle => 0x8a8f98,
            SessionStatus::Todo => 0x5b8def,
            SessionStatus::Working => 0xe0a33a,
            SessionStatus::Done => 0x4caf7d,
            SessionStatus::Dropped => 0x9a6b6b,
        }
    }
}

/// 任务阶段：机器可读的细状态，决定崩溃后能不能续跑。
///
/// 与 `SessionStatus` 刻意分开：`SessionStatus` 关心「卡片胶囊显示什么」
/// （Working / Done / Dropped），这里关心「磁盘上留下 Running 意味着进程
/// 被杀了」——后者才是断点续跑需要的信号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskStage {
    /// 没在跑任务。
    #[default]
    Idle,
    /// 正在生成（模型产 token / 调工具）。
    Running,
    /// 等用户批准写操作。
    AwaitingApproval,
    /// 正常跑完。
    Done,
    /// 出错停下。
    Failed,
    /// 用户主动停止。
    Cancelled,
    /// 进程在任务途中被杀，重启后由用户决定要不要续跑。
    Interrupted,
}

impl TaskStage {
    /// 是否「有任务没走完」。
    ///
    /// 这两个状态出现在磁盘上就是崩溃证据：正常收尾一定会写成
    /// `Done` / `Failed` / `Cancelled`。
    pub fn is_active(&self) -> bool {
        matches!(self, TaskStage::Running | TaskStage::AwaitingApproval)
    }

    /// 是否可以从这里续跑。
    pub fn can_resume(&self) -> bool {
        matches!(
            self,
            TaskStage::Interrupted | TaskStage::Failed | TaskStage::Cancelled
        )
    }

    /// 映射成会话卡片右上角的圆圈。
    ///
    /// 单一来源：`action` 只做显示投影，权威状态永远是 `stage`。
    pub fn action(&self) -> SessionAction {
        match self {
            TaskStage::Interrupted | TaskStage::Failed | TaskStage::Cancelled => {
                SessionAction::Resume
            }
            TaskStage::Done => SessionAction::Done,
            TaskStage::Idle | TaskStage::Running | TaskStage::AwaitingApproval => {
                SessionAction::None
            }
        }
    }
}

/// 续跑提示：告诉模型「接着做，别重做已经落盘的部分」。
///
/// 为什么不重发原始指令：会话历史里已经有一条同样的 user 消息，再发一遍
/// 等于让模型以为用户又说了一次。续跑只需额外告诉它「上一轮断了」。
pub fn resume_instruction(original: &str) -> String {
    let original = original.trim();
    if original.is_empty() {
        "上一轮被中断了。请先看当前对话与文件现状，再从断点继续未完成的部分；已经完成并落盘的步骤不要重做。".into()
    } else {
        format!(
            "上一轮被中断了。最初的任务是：{original}\n\n请先确认当前进度（需要时用 read_file 看文件现状），再从断点继续未完成的部分；已经完成并落盘的步骤不要重做。"
        )
    }
}

/// 会话分类标记。用于在列表里区分工作类型，并支持按类筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionTag {
    /// 未分类。
    #[default]
    None,
    /// 新功能开发。
    Feature,
    /// 缺陷修复。
    Bug,
    /// 重构。
    Refactor,
    /// 文档。
    Docs,
    /// 调试排查。
    Debug,
    /// 实验探索。
    Experiment,
}

impl SessionTag {
    pub fn all() -> [SessionTag; 7] {
        [
            SessionTag::None,
            SessionTag::Feature,
            SessionTag::Bug,
            SessionTag::Refactor,
            SessionTag::Docs,
            SessionTag::Debug,
            SessionTag::Experiment,
        ]
    }

    /// 中文短标签（显示在卡片上）。
    pub fn label(&self) -> &'static str {
        match self {
            SessionTag::None => "",
            SessionTag::Feature => "功能",
            SessionTag::Bug => "修复",
            SessionTag::Refactor => "重构",
            SessionTag::Docs => "文档",
            SessionTag::Debug => "调试",
            SessionTag::Experiment => "实验",
        }
    }

    /// 分类主色（24 位 RGB）。
    pub fn hue(&self) -> u32 {
        match self {
            SessionTag::None => 0x6b7280,
            SessionTag::Feature => 0x3b82f6,
            SessionTag::Bug => 0xef4444,
            SessionTag::Refactor => 0xa855f7,
            SessionTag::Docs => 0x22c55e,
            SessionTag::Debug => 0xf59e0b,
            SessionTag::Experiment => 0x06b6d4,
        }
    }

    /// 下一个分类（点击循环切换用）。
    pub fn next(&self) -> SessionTag {
        let all = Self::all();
        let i = all.iter().position(|t| t == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }
}

/// 会话的运行模式。
///
/// 在**新建会话时选定**，之后不可切换：单助手与团队的工具面、
/// 事件流、气泡样式完全不同，中途切换会让历史里两种发言混在一起，
/// 角色上下文也接不上。需要另一种模式就新建一个会话。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionMode {
    /// 单助手：一个助手同时负责问答与改代码。
    #[default]
    Solo,
    /// 团队：多角色按交接链协作。
    Team,
}

impl SessionMode {
    pub fn label(self) -> &'static str {
        match self {
            SessionMode::Solo => "单助手",
            SessionMode::Team => "团队",
        }
    }

    /// 列表/标题栏用的小字形。
    pub fn glyph(self) -> &'static str {
        match self {
            SessionMode::Solo => "◎",
            SessionMode::Team => "◈",
        }
    }

    pub fn is_team(self) -> bool {
        matches!(self, SessionMode::Team)
    }
}

/// 思考等级（会话级）。
///
/// 只做「一个档位 → 三家协议各自的参数」的映射，不在 core 里判断模型能力：
/// - OpenAI 兼容：`reasoning_effort`（不认识这个参数的网关会 400，请求层自动去掉重试）
/// - Anthropic：`thinking.budget_tokens`
/// - Gemini：`generationConfig.thinkingConfig.thinkingBudget`
///
/// `Auto` 表示**什么都不发**，行为与加这个功能之前完全一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    /// 自动：不向厂商发送任何思考参数，由模型自己决定。
    #[default]
    Auto,
    Low,
    Medium,
    High,
}

impl ThinkingLevel {
    /// 滑杆上的四档，顺序即从浅到深。
    pub const ALL: [ThinkingLevel; 4] = [
        ThinkingLevel::Auto,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThinkingLevel::Auto => "自动",
            ThinkingLevel::Low => "低",
            ThinkingLevel::Medium => "中",
            ThinkingLevel::High => "高",
        }
    }

    /// 档位下标（0–3），滑杆与存档用。
    pub fn index(self) -> usize {
        match self {
            ThinkingLevel::Auto => 0,
            ThinkingLevel::Low => 1,
            ThinkingLevel::Medium => 2,
            ThinkingLevel::High => 3,
        }
    }

    pub fn from_index(i: usize) -> ThinkingLevel {
        Self::ALL[i.min(Self::ALL.len() - 1)]
    }

    /// OpenAI 兼容协议的 `reasoning_effort`；`Auto` 不发送。
    pub fn effort(self) -> Option<&'static str> {
        match self {
            ThinkingLevel::Auto => None,
            ThinkingLevel::Low => Some("low"),
            ThinkingLevel::Medium => Some("medium"),
            ThinkingLevel::High => Some("high"),
        }
    }

    /// Anthropic 的 `thinking.budget_tokens` / Gemini 的 `thinkingBudget`；`Auto` 不发送。
    ///
    /// 低档就取 Anthropic 允许的最小值 1024，免得「开了低档反而 400」。
    pub fn budget(self) -> Option<u32> {
        match self {
            ThinkingLevel::Auto => None,
            ThinkingLevel::Low => Some(1024),
            ThinkingLevel::Medium => Some(4096),
            ThinkingLevel::High => Some(16384),
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, ThinkingLevel::Auto)
    }
}

/// 会话权限档位（会话级）。
///
/// 决定「写操作与命令」在 agent 循环里怎么处理：
/// - `ReadOnly` / `Plan`：需要审批的工具（写文件、打补丁、批量修改、`run_command`、
///   全部 MCP 工具）**不发给模型**；万一模型仍发来这类调用，执行前直接拒绝、不弹审批。
/// - `Standard`：写操作与命令逐次弹审批（历史行为，默认）。
/// - `Full`：写操作与命令直接执行、不弹审批；路径沙箱与 `.git` 禁写等约束不变。
///
/// `Plan` 与 `ReadOnly` 的工具集相同，差别只在系统提示：计划档要求先产出
/// 实施计划，等用户切档后再执行。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    ReadOnly,
    Plan,
    #[default]
    Standard,
    Full,
}

impl PermissionMode {
    /// 面板里的四档，顺序即从最严到最松。
    pub const ALL: [PermissionMode; 4] = [
        PermissionMode::ReadOnly,
        PermissionMode::Plan,
        PermissionMode::Standard,
        PermissionMode::Full,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::ReadOnly => "只读",
            PermissionMode::Plan => "计划",
            PermissionMode::Standard => "标准",
            PermissionMode::Full => "完整",
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            PermissionMode::ReadOnly => "◉",
            PermissionMode::Plan => "◇",
            PermissionMode::Standard => "◈",
            PermissionMode::Full => "⚡",
        }
    }

    /// 面板里的一句说明。
    pub fn desc(self) -> &'static str {
        match self {
            PermissionMode::ReadOnly => "只能读取、检索与联网；写文件和执行命令的工具不提供",
            PermissionMode::Plan => "只读调研，先产出实施计划；确认后切到标准或完整执行",
            PermissionMode::Standard => "写操作与命令逐次弹审批，读操作自动执行",
            PermissionMode::Full => "写操作与命令直接执行、不弹审批（沙箱仍然生效）",
        }
    }

    /// 是否不向模型提供写工具（只读 / 计划）。
    pub fn hides_write_tools(self) -> bool {
        matches!(self, PermissionMode::ReadOnly | PermissionMode::Plan)
    }

    pub fn is_full(self) -> bool {
        matches!(self, PermissionMode::Full)
    }
}

/// 一个会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub team_name: String,
    /// 创建时选择的团队 id；旧会话为空 → 回落当前 active team。
    #[serde(default)]
    pub team_id: String,
    /// 运行模式（旧配置没有这个字段时按单助手处理）。
    #[serde(default)]
    pub mode: SessionMode,
    pub messages: Vec<Message>,
    pub updated_label: String,

    // ===== 左侧会话卡片展示字段（旧配置反序列化时为空） =====
    /// 最后一条消息的一行摘要（卡片第二行左侧）。
    #[serde(default)]
    pub preview: String,
    /// 执行状态。
    #[serde(default)]
    pub status: SessionStatus,
    /// 额外元信息，如耗时 `4m 42s`（状态胶囊内显示）。
    #[serde(default)]
    pub meta: String,
    /// 卡片头像色索引（对应 `theme::role_color`）。
    #[serde(default)]
    pub accent: usize,
    /// 头像内的字形（空则取标题首字）。
    #[serde(default)]
    pub icon: String,
    /// 标题前的小徽标，如 `⚡`；空则不显示。
    #[serde(default)]
    pub badge: String,
    /// 卡片右上角的状态圆圈：可继续 / 可重放 / 已完成 / 失败。
    #[serde(default)]
    pub action: SessionAction,
    /// 分类标记。
    #[serde(default)]
    pub tag: SessionTag,
    /// 最后活跃时间（Unix 秒）。会话列表按它倒序，最新在最前。
    ///
    /// 用真实时间戳而不是字符串标签（如「2 分钟前」），因为字符串无法排序。
    #[serde(default)]
    pub updated_at: u64,
    /// 是否置顶。置顶的会话永远排在最前，不参与按时间排序。
    #[serde(default)]
    pub pinned: bool,

    // ===== 会话级模型（旧会话为空 → 跟随设置里的当前厂商） =====
    /// 本会话使用的厂商 id；空表示跟随全局「当前厂商」。
    #[serde(default)]
    pub provider_id: String,
    /// 本会话使用的模型 id；空表示用该厂商的默认模型。
    #[serde(default)]
    pub model: String,
    /// 本会话的思考等级；旧会话没有这个字段时按 `Auto`（不发送参数）处理。
    #[serde(default)]
    pub thinking: ThinkingLevel,
    /// 本会话的权限档位；旧会话没有这个字段时按「标准」（逐次审批）处理。
    #[serde(default)]
    pub permission: PermissionMode,

    // ===== 断点续跑（旧配置反序列化时为空/默认） =====
    /// 机器可读的任务阶段。崩溃恢复的依据。
    #[serde(default)]
    pub stage: TaskStage,
    /// 中断那一轮的原始指令；续跑时告诉模型「原来要做的是这件事」。
    #[serde(default)]
    pub resume_prompt: String,

    // ===== 产物（旧会话为空） =====
    /// Agent 写入的文件与生成的图片。对话底部与右栏「产物」面板共用。
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

impl Session {
    /// 把 `action` 对齐到 `stage`（单一来源，避免两套状态打架）。
    pub fn sync_action(&mut self) {
        self.action = self.stage.action();
    }

    /// 记录一次活跃：刷新时间戳与相对标签。
    pub fn touch(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.updated_at = now;
        self.updated_label = relative_label(now);
    }
}

/// 把时间戳转成「x 分钟前」这类相对标签。
///
/// 没有用 chrono：只为这一个用途引一个时间库不值得，
/// 而 `SystemTime` 的秒数差足够表达「几秒/几分/几小时/几天前」。
pub fn relative_label(at: u64) -> String {
    if at == 0 {
        return "刚刚".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(at);
    let d = now.saturating_sub(at);
    match d {
        0..=59 => "刚刚".into(),
        60..=3599 => format!("{}分钟前", d / 60),
        3600..=86399 => format!("{}小时前", d / 3600),
        86400..=2_592_000 => format!("{}天前", d / 86400),
        _ => format!("{}个月前", d / 2_592_000),
    }
}

/// 把会话按「最近活跃优先」排序，返回它们在原数组里的下标。
///
/// 返回下标而不是重排数组：`active_session` 是索引引用，
/// 重排会让它指向别的会话。渲染层按下标取值即可。
pub fn sessions_by_recent(sessions: &[Session]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..sessions.len()).collect();
    // 排序规则：置顶优先 → 最近活跃优先 → 时间相同时保持原顺序（避免顺序乱跳）
    idx.sort_by(|a, b| {
        let pa = sessions[*a].pinned;
        let pb = sessions[*b].pinned;
        pb.cmp(&pa)
            .then(sessions[*b].updated_at.cmp(&sessions[*a].updated_at))
            .then(a.cmp(b))
    });
    idx
}

/// 会话是否属于「已归档」（完成或废弃）。
///
/// 置顶不参与归档：置顶意味着用户要常看，收进折叠组反而藏起来了。
pub fn is_archived_session(s: &Session) -> bool {
    !s.pinned && matches!(s.status, SessionStatus::Done | SessionStatus::Dropped)
}

/// 会话列表的过滤条件：搜索词（标题/摘要/副标题）+ 状态 + 分类。
///
/// 搜索不扫消息正文：列表要在每个输入字符上即时重算，扫全文会卡；
/// 正文检索是另一件事（将来给全局搜索）。
pub fn session_matches_list(
    s: &Session,
    query: &str,
    status: Option<SessionStatus>,
    tag: Option<SessionTag>,
) -> bool {
    if let Some(st) = status {
        if s.status != st {
            return false;
        }
    }
    if let Some(t) = tag {
        if s.tag != t {
            return false;
        }
    }
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    [&s.title, &s.preview, &s.subtitle]
        .iter()
        .any(|x| x.to_lowercase().contains(&q))
}

/// 置顶的会话数量（列表分隔线用）。
pub fn pinned_count(sessions: &[Session]) -> usize {
    sessions.iter().filter(|s| s.pinned).count()
}

/// 会话卡片右上角的圆圈指示（同时也是可点击的操作入口）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionAction {
    /// 不显示圆圈。
    #[default]
    None,
    /// 黄色 ▶：可继续执行。
    Resume,
    /// 黄色 ◉：可重放（历史任务）。
    Replay,
    /// 蓝色 ✓：已完成。
    Done,
    /// 灰色 ✗：失败或已取消。
    Failed,
}

/// 会话内某文件被 Agent 改动过的汇总。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    /// 是否被拒绝（拒绝了就不算真改动）。
    pub denied: bool,
}

/// 把会话里的工具调用聚合成「每个文件改了多少行」。
///
/// 抽成纯函数是为了能脱离 UI 单测：同一文件多次改动会累加，
/// 只要有一次被拒绝就标记为 `denied`（说明并非所有改动都落地了）。
pub fn aggregate_changes(messages: &[Message]) -> Vec<FileChange> {
    use std::collections::BTreeMap;

    let mut map: BTreeMap<String, FileChange> = BTreeMap::new();
    for m in messages {
        if let MessageKind::ToolCall {
            path,
            status,
            added,
            removed,
            ..
        } = &m.kind
        {
            if path.is_empty() || (*added == 0 && *removed == 0) {
                continue;
            }
            let denied = *status == ToolRunStatus::Denied;
            let e = map.entry(path.clone()).or_insert(FileChange {
                path: path.clone(),
                added: 0,
                removed: 0,
                denied: false,
            });
            e.added += added;
            e.removed += removed;
            e.denied = denied;
        }
    }
    map.into_values().collect()
}

/// 文件树节点。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub name: String,
    pub is_dir: bool,
    pub level: u8,
    pub expanded: bool,
    /// 修改标记。
    pub modified: bool,
    /// 产出该文件的角色首字母（可选）。
    pub owner: Option<String>,
    /// 相对工作区根的路径。之前只存文件名，同名文件会点错，
    /// 现在用它精确定位。旧配置反序列化时为空。
    #[serde(default)]
    pub path: String,
}

#[cfg(test)]
mod mention_tests {
    use super::*;

    fn roles() -> Vec<Role> {
        vec![
            Role::new("a", "需求验证", "需"),
            Role::new("b", "接口成员", "接"),
            Role::new("c", "接口", "接"),
            Role::new("d", "Planner", "P"),
            Role::new("e", "b", "B"),
        ]
    }

    #[test]
    fn hits_leading_mention() {
        assert_eq!(parse_role_mention("@需求验证 请验证", &roles()), Some(0));
    }

    #[test]
    fn prefers_longest_prefix() {
        assert_eq!(parse_role_mention("@接口成员 改一下", &roles()), Some(1));
        assert_eq!(parse_role_mention("@接口 改一下", &roles()), Some(2));
    }

    #[test]
    fn takes_earliest_mention() {
        assert_eq!(
            parse_role_mention("@接口成员 然后 @需求验证", &roles()),
            Some(1)
        );
    }

    #[test]
    fn ignores_email_and_glued() {
        assert_eq!(parse_role_mention("@a@b.com", &roles()), None);
        assert_eq!(parse_role_mention("a@需求验证", &roles()), None);
    }

    #[test]
    fn ignores_file_extension() {
        assert_eq!(parse_role_mention("@需求验证.md 看下", &roles()), None);
        // 句末句点是边界，仍应命中。
        assert_eq!(parse_role_mention("请找 @需求验证。", &roles()), Some(0));
    }

    #[test]
    fn ascii_case_insensitive() {
        assert_eq!(parse_role_mention("@planner 你好", &roles()), Some(3));
    }

    #[test]
    fn skips_empty_role_name() {
        let mut rs = roles();
        rs.push(Role::new("f", "  ", "F"));
        assert_eq!(parse_role_mention("@需求验证 你好", &rs), Some(0));
    }

    #[test]
    fn role_by_name_trims_and_ignores_case() {
        let team = Team {
            id: "t".into(),
            name: "t".into(),
            flow: FlowMode::Pipeline,
            roles: roles(),
        };
        assert_eq!(team.role_by_name(" PLANNER "), Some(3));
        assert_eq!(team.role_by_name("不存在"), None);
        assert_eq!(team.role_by_name("  "), None);
    }

    #[test]
    fn handoff_detail_defaults_for_old_sessions() {
        let json = r#"{"type":"handoff","from":"A","to":"B","action":"改3处"}"#;
        let kind: MessageKind = serde_json::from_str(json).unwrap();
        match kind {
            MessageKind::Handoff { detail, action, .. } => {
                assert_eq!(action, "改3处");
                assert_eq!(detail, "");
            }
            _ => panic!("应反序列化为 Handoff"),
        }
    }

    #[test]
    fn role_provider_id_defaults_for_old_configs() {
        // 旧团队配置没有 provider_id：反序列化为空 = 跟随会话默认，
        // 展示标签 `Claude Opus` 不能被当成模型 id 去路由。
        let json = r#"{"id":"planner","name":"规划者","kind_label":"架构","model":"Claude Opus","prompt":"p","color_index":0,"initial":"P","skills":[]}"#;
        let role: Role = serde_json::from_str(json).unwrap();
        assert!(role.provider_id.is_empty());
        assert_eq!(role.model, "Claude Opus");
    }

    #[test]
    fn role_roundtrips_provider_binding() {
        let mut r = Role::new("dev", "开发者", "D");
        r.provider_id = "deepseek".into();
        r.model = "deepseek-chat".into();
        let json = serde_json::to_string(&r).unwrap();
        let back: Role = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider_id, "deepseek");
        assert_eq!(back.model, "deepseek-chat");
    }

    #[test]
    fn session_team_id_defaults_for_old_sessions() {
        // 旧会话没有 team_id：反序列化为空，团队会话回落「当前团队」。
        let json = r#"{"id":"s1","title":"t","subtitle":"","team_name":"默认团队","messages":[],"updated_label":"刚刚"}"#;
        let s: Session = serde_json::from_str(json).unwrap();
        assert!(s.team_id.is_empty());
        assert!(!s.mode.is_team());
    }

    #[test]
    fn tool_call_thread_round_default_for_old_sessions() {
        // 旧会话没有 thread/round 字段：反序列化必须是 None，否则统计行会编造数字。
        let json = r#"{"type":"tool_call","name":"list_dir","summary":"","status":"ok","output":"","diff":[],"added":0,"removed":0}"#;
        let kind: MessageKind = serde_json::from_str(json).unwrap();
        match kind {
            MessageKind::ToolCall { thread, round, .. } => {
                assert_eq!(thread, None);
                assert_eq!(round, None);
            }
            _ => panic!("应反序列化为 ToolCall"),
        }
    }
}

#[cfg(test)]
mod aggregate_tests {
    use super::*;

    fn tool(path: &str, added: u32, removed: u32, status: ToolRunStatus) -> Message {
        Message {
            kind: MessageKind::ToolCall {
                name: "write_file".into(),
                call_id: String::new(),
                arguments: String::new(),
                path: path.into(),
                summary: String::new(),
                status,
                thread: None,
                round: None,
                output: String::new(),
                diff: Vec::new(),
                added,
                removed,
            },
            body: String::new(),
            code: None,
            attachments: Vec::new(),
            elapsed_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        }
    }

    #[test]
    fn aggregates_per_file() {
        let msgs = vec![
            tool("a.rs", 3, 1, ToolRunStatus::Ok),
            tool("a.rs", 2, 0, ToolRunStatus::Ok),
            tool("b.rs", 0, 5, ToolRunStatus::Ok),
        ];
        let out = aggregate_changes(&msgs);
        assert_eq!(out.len(), 2);
        let a = out.iter().find(|c| c.path == "a.rs").unwrap();
        assert_eq!((a.added, a.removed), (5, 1), "同一文件应累加");
        let b = out.iter().find(|c| c.path == "b.rs").unwrap();
        assert_eq!((b.added, b.removed), (0, 5));
    }

    #[test]
    fn denied_marks_file() {
        let msgs = vec![
            tool("a.rs", 3, 0, ToolRunStatus::Ok),
            tool("a.rs", 9, 0, ToolRunStatus::Denied),
        ];
        let out = aggregate_changes(&msgs);
        assert!(out[0].denied, "有被拒绝的改动时要标出来");
    }

    #[test]
    fn skips_noise() {
        let msgs = vec![
            tool("", 3, 0, ToolRunStatus::Ok),          // 无路径
            tool("a.rs", 0, 0, ToolRunStatus::Ok),      // 无增删
            Message {
                kind: MessageKind::User,
                body: "hi".into(),
                code: None,
                attachments: Vec::new(),
                elapsed_ms: 0,
                tokens_in: 0,
                tokens_out: 0,
            },
        ];
        assert!(aggregate_changes(&msgs).is_empty());
    }

    #[test]
    fn sorted_by_path() {
        let msgs = vec![
            tool("z.rs", 1, 0, ToolRunStatus::Ok),
            tool("a.rs", 1, 0, ToolRunStatus::Ok),
        ];
        let out = aggregate_changes(&msgs);
        assert_eq!(out[0].path, "a.rs");
        assert_eq!(out[1].path, "z.rs");
    }
}

#[cfg(test)]
mod session_list_tests {
    use super::*;

    fn session(id: &str) -> Session {
        Session {
            id: id.into(),
            title: "标题".into(),
            subtitle: String::new(),
            team_name: String::new(),
            team_id: String::new(),
            mode: SessionMode::Solo,
            messages: Vec::new(),
            updated_label: String::new(),
            preview: String::new(),
            status: SessionStatus::Idle,
            meta: String::new(),
            accent: 0,
            icon: String::new(),
            badge: String::new(),
            action: SessionAction::None,
            tag: SessionTag::None,
            updated_at: 0,
            pinned: false,
            provider_id: String::new(),
            model: String::new(),
            thinking: ThinkingLevel::Auto,
            permission: PermissionMode::Standard,
            stage: TaskStage::Idle,
            resume_prompt: String::new(),
            artifacts: Vec::new(),
        }
    }

    #[test]
    fn permission_mode_defaults_to_standard_and_roundtrips() {
        assert_eq!(PermissionMode::default(), PermissionMode::Standard);
        assert!(PermissionMode::ReadOnly.hides_write_tools());
        assert!(PermissionMode::Plan.hides_write_tools());
        assert!(!PermissionMode::Standard.hides_write_tools());
        assert!(!PermissionMode::Full.hides_write_tools());
        let text = serde_json::to_string(&PermissionMode::ReadOnly).unwrap();
        assert_eq!(text, "\"read_only\"");
        assert_eq!(
            serde_json::from_str::<PermissionMode>("\"full\"").unwrap(),
            PermissionMode::Full
        );
        // 旧会话没有 permission 字段 → 标准档
        let old = r#"{"id":"a","title":"旧","subtitle":"","team_name":"",
                      "messages":[],"updated_label":""}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        assert_eq!(s.permission, PermissionMode::Standard);
    }

    #[test]
    fn thinking_level_maps_to_provider_params() {
        assert_eq!(ThinkingLevel::default(), ThinkingLevel::Auto);
        // 自动什么都不发
        assert_eq!(ThinkingLevel::Auto.effort(), None);
        assert_eq!(ThinkingLevel::Auto.budget(), None);
        assert_eq!(ThinkingLevel::High.effort(), Some("high"));
        assert_eq!(ThinkingLevel::Medium.budget(), Some(4096));
        // 低档不能低于 Anthropic 的最小预算 1024
        assert_eq!(ThinkingLevel::Low.budget(), Some(1024));
    }

    #[test]
    fn thinking_level_serde_and_old_session_default() {
        let text = serde_json::to_string(&ThinkingLevel::High).unwrap();
        assert_eq!(text, "\"high\"");
        // 旧会话 JSON 没有 thinking 字段：必须回落 Auto（不改变原有行为）
        let old = r#"{"id":"a","title":"旧","subtitle":"","team_name":"","messages":[],"updated_label":"","preview":""}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        assert_eq!(s.thinking, ThinkingLevel::Auto);
        // 下标往返（滑杆用）
        for (i, lv) in ThinkingLevel::ALL.iter().enumerate() {
            assert_eq!(ThinkingLevel::from_index(i), *lv);
            assert_eq!(lv.index(), i);
        }
        // 越界下标收敛到最后一档，不 panic
        assert_eq!(ThinkingLevel::from_index(99), ThinkingLevel::High);
    }

    #[test]
    fn todo_status_round_trips_and_old_json_defaults_idle() {
        let text = serde_json::to_string(&SessionStatus::Todo).unwrap();
        assert_eq!(text, "\"todo\"");
        let back: SessionStatus = serde_json::from_str(&text).unwrap();
        assert_eq!(back, SessionStatus::Todo);
        // 旧会话 JSON 没有 status 字段：必须回落 Idle
        let old = r#"{"id":"a","title":"旧","subtitle":"","team_name":"","messages":[],"updated_label":"","preview":""}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        assert_eq!(s.status, SessionStatus::Idle);
    }

    #[test]
    fn archived_excludes_pinned() {
        let mut s = session("a");
        s.status = SessionStatus::Done;
        assert!(is_archived_session(&s));
        s.pinned = true;
        assert!(!is_archived_session(&s), "置顶的完成会话不进归档");
        s.pinned = false;
        s.status = SessionStatus::Todo;
        assert!(!is_archived_session(&s));
    }

    #[test]
    fn matches_query_status_and_tag() {
        let mut s = session("a");
        s.title = "修复登录 Bug".into();
        s.preview = "登录页报错".into();
        s.status = SessionStatus::Todo;
        s.tag = SessionTag::Bug;
        // 空条件全通过
        assert!(session_matches_list(&s, "", None, None));
        // 标题命中（大小写不敏感）
        assert!(session_matches_list(&s, "bug", None, None));
        // 摘要命中
        assert!(session_matches_list(&s, "登录页", None, None));
        // 搜不到
        assert!(!session_matches_list(&s, "部署", None, None));
        // 状态/分类不符
        assert!(!session_matches_list(&s, "", Some(SessionStatus::Done), None));
        assert!(session_matches_list(&s, "", Some(SessionStatus::Todo), None));
        assert!(!session_matches_list(&s, "", None, Some(SessionTag::Docs)));
        assert!(session_matches_list(&s, "", None, Some(SessionTag::Bug)));
        // 组合：都满足才通过
        assert!(session_matches_list(
            &s,
            "登录",
            Some(SessionStatus::Todo),
            Some(SessionTag::Bug)
        ));
        assert!(!session_matches_list(
            &s,
            "部署",
            Some(SessionStatus::Todo),
            Some(SessionTag::Bug)
        ));
    }

    #[test]
    fn message_attachments_default_for_old_sessions() {
        // 旧会话里的消息没有 attachments 字段：反序列化为空，不能报错。
        let old = r#"{"kind":{"type":"user"},"body":"旧消息","code":null}"#;
        let m: Message = serde_json::from_str(old).unwrap();
        assert!(m.attachments.is_empty());
        assert_eq!(m.body, "旧消息");
    }

    #[test]
    fn attachment_roundtrip_keeps_kind_and_path() {
        let a = Attachment {
            name: "shot.png".into(),
            path: r"C:\tmp\shot.png".into(),
            mime: "image/png".into(),
            kind: AttachmentKind::Image,
        };
        let text = serde_json::to_string(&a).unwrap();
        let back: Attachment = serde_json::from_str(&text).unwrap();
        assert_eq!(back, a);
        // 文本附件走 snake_case，旧数据缺失 kind 时默认图片（不会丢字段）
        let legacy = r#"{"name":"a.md","path":"C:/a.md"}"#;
        let l: Attachment = serde_json::from_str(legacy).unwrap();
        assert_eq!(l.kind, AttachmentKind::Image);
        assert!(l.mime.is_empty());
    }
}
