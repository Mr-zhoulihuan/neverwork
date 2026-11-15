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
            model: String::from("Claude Opus"),
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
                    prompt: "负责审查正确性、边界与性能。".into(),
                    color_index: 2,
                    initial: "R".into(),
                    skills: vec![],
                },
            ],
        }
    }
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
    Handoff { from: String, to: String, action: String },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub kind: MessageKind,
    /// 正文（Markdown/富文本的简化：纯文本 + 代码块）。
    pub body: String,
    /// 代码块（可选）。
    pub code: Option<CodeBlock>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// 未开始 / 普通会话。
    #[default]
    Idle,
    /// 正在生成（工作中…）。
    Working,
    /// 已完成。
    Done,
    /// 已废弃。
    Dropped,
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

/// 一个会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub team_name: String,
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

    // ===== 断点续跑（旧配置反序列化时为空/默认） =====
    /// 机器可读的任务阶段。崩溃恢复的依据。
    #[serde(default)]
    pub stage: TaskStage,
    /// 中断那一轮的原始指令；续跑时告诉模型「原来要做的是这件事」。
    #[serde(default)]
    pub resume_prompt: String,
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
                output: String::new(),
                diff: Vec::new(),
                added,
                removed,
            },
            body: String::new(),
            code: None,
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
