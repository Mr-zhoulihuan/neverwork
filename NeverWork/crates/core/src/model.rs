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
