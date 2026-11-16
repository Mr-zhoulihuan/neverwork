//! 应用设置：外观、编辑器、模型厂商、技能与同步。
//!
//! 与 `Config` 分开存盘（`settings.json`），这样团队配置和偏好设置可以分别
//! 导入导出，也便于将来做云同步时只同步设置部分。

use serde::{Deserialize, Serialize};

// ============ 外观 ============

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// 深色（默认）。
    #[default]
    Dark,
    /// 浅色。
    Light,
    /// 跟随系统。
    System,
}

impl ThemeMode {
    pub fn label(&self) -> &'static str {
        match self {
            ThemeMode::Dark => "深色",
            ThemeMode::Light => "浅色",
            ThemeMode::System => "跟随系统",
        }
    }

    pub fn all() -> [ThemeMode; 3] {
        [ThemeMode::Dark, ThemeMode::Light, ThemeMode::System]
    }
}

// ============ 模型厂商 ============

/// 内置厂商预设。新增厂商只需在这里加一项 + 默认 base_url / 模型列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderPreset {
    OpenAi,
    Anthropic,
    Gemini,
    DeepSeek,
    Qwen,
    Glm,
    Moonshot,
    Baichuan,
    Doubao,
    Ollama,
    Custom,
}

impl ProviderPreset {
    pub fn all() -> &'static [ProviderPreset] {
        &[
            ProviderPreset::OpenAi,
            ProviderPreset::Anthropic,
            ProviderPreset::Gemini,
            ProviderPreset::DeepSeek,
            ProviderPreset::Qwen,
            ProviderPreset::Glm,
            ProviderPreset::Moonshot,
            ProviderPreset::Doubao,
            ProviderPreset::Baichuan,
            ProviderPreset::Ollama,
            ProviderPreset::Custom,
        ]
    }

    pub fn id(&self) -> &'static str {
        match self {
            ProviderPreset::OpenAi => "openai",
            ProviderPreset::Anthropic => "anthropic",
            ProviderPreset::Gemini => "gemini",
            ProviderPreset::DeepSeek => "deepseek",
            ProviderPreset::Qwen => "qwen",
            ProviderPreset::Glm => "glm",
            ProviderPreset::Moonshot => "moonshot",
            ProviderPreset::Doubao => "doubao",
            ProviderPreset::Baichuan => "baichuan",
            ProviderPreset::Ollama => "ollama",
            ProviderPreset::Custom => "custom",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            ProviderPreset::OpenAi => "OpenAI",
            ProviderPreset::Anthropic => "Anthropic",
            ProviderPreset::Gemini => "Google Gemini",
            ProviderPreset::DeepSeek => "DeepSeek 深度求索",
            ProviderPreset::Qwen => "通义千问 Qwen",
            ProviderPreset::Glm => "智谱 GLM",
            ProviderPreset::Moonshot => "Moonshot Kimi",
            ProviderPreset::Doubao => "豆包 Doubao",
            ProviderPreset::Baichuan => "百川 Baichuan",
            ProviderPreset::Ollama => "Ollama（本地）",
            ProviderPreset::Custom => "自定义",
        }
    }

    /// 默认 API 基址。
    pub fn base_url(&self) -> &'static str {
        match self {
            ProviderPreset::OpenAi => "https://api.openai.com/v1",
            ProviderPreset::Anthropic => "https://api.anthropic.com/v1",
            ProviderPreset::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            ProviderPreset::DeepSeek => "https://api.deepseek.com/v1",
            ProviderPreset::Qwen => "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ProviderPreset::Glm => "https://open.bigmodel.cn/api/paas/v4",
            ProviderPreset::Moonshot => "https://api.moonshot.cn/v1",
            ProviderPreset::Doubao => "https://ark.cn-beijing.volces.com/api/v3",
            ProviderPreset::Baichuan => "https://api.baichuan-ai.com/v1",
            ProviderPreset::Ollama => "http://localhost:11434/v1",
            ProviderPreset::Custom => "",
        }
    }

    /// 默认可用模型（第一个即默认选中）。
    pub fn models(&self) -> &'static [&'static str] {
        match self {
            ProviderPreset::OpenAi => &["gpt-4o", "gpt-4o-mini", "o3", "o4-mini", "gpt-4.1"],
            ProviderPreset::Anthropic => &[
                "claude-sonnet-4-5",
                "claude-opus-4-1",
                "claude-haiku-4-5",
                "claude-3-5-sonnet",
            ],
            ProviderPreset::Gemini => &["gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.0-flash"],
            ProviderPreset::DeepSeek => &["deepseek-chat", "deepseek-reasoner", "deepseek-coder"],
            ProviderPreset::Qwen => &["qwen-max", "qwen-plus", "qwen-turbo", "qwen2.5-coder-32b-instruct"],
            ProviderPreset::Glm => &["glm-4-plus", "glm-4", "glm-4-flash", "glm-4v"],
            ProviderPreset::Moonshot => &["moonshot-v1-128k", "moonshot-v1-32k", "kimi-k2-0711-preview"],
            ProviderPreset::Doubao => &["doubao-pro-32k", "doubao-lite-32k", "doubao-seed-1-6"],
            ProviderPreset::Baichuan => &["Baichuan4", "Baichuan3-Turbo"],
            ProviderPreset::Ollama => &["llama3.2", "qwen2.5-coder", "deepseek-r1", "mistral"],
            ProviderPreset::Custom => &[],
        }
    }

    /// 该厂商走哪种协议（决定用哪个 Provider 实现）。
    pub fn protocol(&self) -> Protocol {
        match self {
            ProviderPreset::Anthropic => Protocol::Anthropic,
            ProviderPreset::Gemini => Protocol::Gemini,
            _ => Protocol::OpenAiCompatible,
        }
    }

    /// 是否本地服务（不需要 API Key）。
    pub fn is_local(&self) -> bool {
        matches!(self, ProviderPreset::Ollama)
    }

    /// 申请 Key 的地址（空表示本地服务）。
    pub fn console_url(&self) -> &'static str {
        match self {
            ProviderPreset::OpenAi => "https://platform.openai.com/api-keys",
            ProviderPreset::Anthropic => "https://console.anthropic.com/settings/keys",
            ProviderPreset::Gemini => "https://aistudio.google.com/app/apikey",
            ProviderPreset::DeepSeek => "https://platform.deepseek.com/api_keys",
            ProviderPreset::Qwen => "https://bailian.console.aliyun.com/",
            ProviderPreset::Glm => "https://open.bigmodel.cn/usercenter/apikeys",
            ProviderPreset::Moonshot => "https://platform.moonshot.cn/console/api-keys",
            ProviderPreset::Doubao => "https://console.volcengine.com/ark",
            ProviderPreset::Baichuan => "https://platform.baichuan-ai.com/console/apikey",
            ProviderPreset::Ollama => "",
            ProviderPreset::Custom => "",
        }
    }
}

/// 调用协议。决定 `neverwork-agent` 里用哪个实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// OpenAI `/chat/completions` 兼容（覆盖绝大多数国产厂商）。
    OpenAiCompatible,
    /// Anthropic `/v1/messages`。
    Anthropic,
    /// Google Gemini 原生。
    Gemini,
}

impl Protocol {
    pub fn label(&self) -> &'static str {
        match self {
            Protocol::OpenAiCompatible => "OpenAI 兼容",
            Protocol::Anthropic => "Anthropic",
            Protocol::Gemini => "Gemini 原生",
        }
    }
}

/// 一个 MCP（Model Context Protocol）服务器：由本机 stdio 启动的外部工具进程。
///
/// 安全边界：MCP server 是**用户自己配置的本地进程**，信任级别等同用户自己
/// 开的终端——它能做任何事。所以它提供的工具一律当作写操作（`requires_approval
/// = true`），且不走 `ToolContext` 的路径限制（限制不了别的进程）。
///
/// **不 derive Debug**：`args` 里可能带 `--api-key xxx`，`env` 里常放
/// `GITHUB_TOKEN`。手写实现只输出数量与 key 名。
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 服务名，用作工具名前缀（`mcp__<name>__<tool>`），只用 ASCII 字母数字下划线。
    pub name: String,
    /// 可执行文件（`npx` / `node` / `python`…），按 PATH 解析。
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// 额外环境变量。值不打印、不导出。
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServerConfig")
            .field("name", &self.name)
            .field("command", &self.command)
            .field("args_len", &self.args.len())
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl McpServerConfig {
    /// 名字消毒：只留 `[a-z0-9_]`，因为要拼进工具名发给模型。
    pub fn sanitized(&self) -> String {
        let mut out = String::new();
        for c in self.name.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_lowercase());
            } else if !out.ends_with('_') {
                out.push('_');
            }
        }
        let out = out.trim_matches('_').to_string();
        if out.is_empty() {
            "mcp".into()
        } else {
            out
        }
    }

    /// 能否启动（命令非空且启用）。
    pub fn usable(&self) -> bool {
        self.enabled && !self.command.trim().is_empty()
    }
}

/// 一条厂商配置。
///
/// 注意：`api_key` 是唯一会明文落盘的敏感字段，因此**不 derive Debug**，
/// 改用下面手写的实现，保证任何 `{:?}` 打印都不会泄露密钥。
#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    /// 唯一 id（自定义厂商用 `custom-<n>`）。
    pub id: String,
    /// 预设类型；自定义时为 `Custom`。
    pub preset: ProviderPreset,
    /// 显示名（可改，如「公司中转」）。
    pub name: String,
    pub base_url: String,
    /// API Key。仅保存在本地 `~/.neverwork/settings.json`。
    #[serde(default)]
    pub api_key: String,
    /// 该厂商可用模型。
    #[serde(default)]
    pub models: Vec<String>,
    /// 当前选中模型。
    #[serde(default)]
    pub model: String,
    /// 是否启用（启用后才会出现在模型选择里）。
    #[serde(default = "yes")]
    pub enabled: bool,
    /// 采样温度 0.0–2.0。
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

fn yes() -> bool {
    true
}

fn default_temperature() -> f32 {
    0.7
}

impl ProviderEntry {
    pub fn from_preset(preset: ProviderPreset) -> Self {
        let models: Vec<String> = preset.models().iter().map(|s| s.to_string()).collect();
        let model = models.first().cloned().unwrap_or_default();
        Self {
            id: preset.id().to_string(),
            preset,
            name: preset.name().to_string(),
            base_url: preset.base_url().to_string(),
            api_key: String::new(),
            models,
            model,
            enabled: !preset.models().is_empty(),
            temperature: default_temperature(),
        }
    }

    pub fn protocol(&self) -> Protocol {
        self.preset.protocol()
    }

    /// 是否可用：本地服务只要有地址，其余需要 Key + 模型。
    pub fn ready(&self) -> bool {
        if self.preset.is_local() {
            return !self.base_url.trim().is_empty() && !self.model.trim().is_empty();
        }
        !self.api_key.trim().is_empty() && !self.model.trim().is_empty()
    }

    /// Key 打码显示：**只留末尾 4 位**，其余一律遮掉。
    ///
    /// 不显示前缀是有意的——不少厂商的 Key 前缀含账号/项目标识，
    /// 截图、录屏、共享屏幕时都可能被看到。
    pub fn masked_key(&self) -> String {
        let k = self.api_key.trim();
        if k.is_empty() {
            return String::new();
        }
        let n = k.chars().count();
        if n <= 4 {
            return "•".repeat(8);
        }
        let tail: String = k.chars().skip(n - 4).collect();
        format!("{}{tail}", "•".repeat(8))
    }

    /// 供「复制 Key」使用：只有用户显式点击才拿到明文。
    pub fn plain_key(&self) -> &str {
        &self.api_key
    }

    /// 脱敏副本：把所有密钥清空，用于导出/分享配置。
    pub fn redacted(&self) -> Self {
        let mut p = self.clone();
        p.api_key.clear();
        p
    }
}

impl std::fmt::Debug for ProviderEntry {
    /// 手写 Debug：密钥只显示脱敏形式，避免日志意外泄露。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("base_url", &self.base_url)
            .field("api_key", &self.masked_key())
            .field("model", &self.model)
            .field("enabled", &self.enabled)
            .finish()
    }
}

// ============ 技能 ============

/// 技能分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCategory {
    Office,
    Knowledge,
    Dev,
    Content,
    Design,
    Data,
    Agent,
    Industry,
    Business,
    Life,
    Ops,
}

impl SkillCategory {
    pub fn all() -> &'static [SkillCategory] {
        &[
            SkillCategory::Office,
            SkillCategory::Knowledge,
            SkillCategory::Dev,
            SkillCategory::Content,
            SkillCategory::Design,
            SkillCategory::Data,
            SkillCategory::Agent,
            SkillCategory::Industry,
            SkillCategory::Business,
            SkillCategory::Life,
            SkillCategory::Ops,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            SkillCategory::Office => "办公效率",
            SkillCategory::Knowledge => "知识管理",
            SkillCategory::Dev => "开发编程",
            SkillCategory::Content => "内容创作",
            SkillCategory::Design => "设计多媒体",
            SkillCategory::Data => "数据分析",
            SkillCategory::Agent => "AI Agent",
            SkillCategory::Industry => "行业专业",
            SkillCategory::Business => "商业运营",
            SkillCategory::Life => "生活服务",
            SkillCategory::Ops => "IT 运维与安全",
        }
    }

    /// serde 名（SKILL.md front-matter 里用，与 JSON 序列化一致）。
    pub fn key(&self) -> &'static str {
        match self {
            SkillCategory::Office => "office",
            SkillCategory::Knowledge => "knowledge",
            SkillCategory::Dev => "dev",
            SkillCategory::Content => "content",
            SkillCategory::Design => "design",
            SkillCategory::Data => "data",
            SkillCategory::Agent => "agent",
            SkillCategory::Industry => "industry",
            SkillCategory::Business => "business",
            SkillCategory::Life => "life",
            SkillCategory::Ops => "ops",
        }
    }

    /// 从 serde 名解析，未知值回落到开发编程。
    pub fn from_key(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "office" => SkillCategory::Office,
            "knowledge" => SkillCategory::Knowledge,
            "content" => SkillCategory::Content,
            "design" => SkillCategory::Design,
            "data" => SkillCategory::Data,
            "agent" => SkillCategory::Agent,
            "industry" => SkillCategory::Industry,
            "business" => SkillCategory::Business,
            "life" => SkillCategory::Life,
            "ops" => SkillCategory::Ops,
            _ => SkillCategory::Dev,
        }
    }

    /// 分类主色（用于卡片图标底色）。
    pub fn hue(&self) -> u32 {
        match self {
            SkillCategory::Office => 0x3b82f6,
            SkillCategory::Knowledge => 0x22c55e,
            SkillCategory::Dev => 0xf97316,
            SkillCategory::Content => 0xa855f7,
            SkillCategory::Design => 0x06b6d4,
            SkillCategory::Data => 0xeab308,
            SkillCategory::Agent => 0xec4899,
            SkillCategory::Industry => 0x6366f1,
            SkillCategory::Business => 0x14b8a6,
            SkillCategory::Life => 0xef4444,
            SkillCategory::Ops => 0x64748b,
        }
    }
}

/// 技能/插件的形态。
///
/// - `Text`：落盘为 `~/.neverwork/skills/<id>/SKILL.md`，安装后把正文注入提示词。
/// - `Mcp`：装的是一个 MCP server，安装 = 写入 `settings.mcp_servers`，
///   模型通过 `mcp__<server>__<tool>` 工具真实调用（进程由用户安装动作引入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    #[default]
    Text,
    Mcp,
}

/// 一个可安装的技能。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// 唯一标识，形如 `indiv-ebandao.dev-expert`。
    pub id: String,
    pub name: String,
    /// 一行简介。
    pub summary: String,
    /// 详细说明（安装后展示）。
    #[serde(default)]
    pub detail: String,
    pub category: SkillCategory,
    /// 作者 / 提供方。
    #[serde(default)]
    pub author: String,
    /// 安装量（展示用）。
    #[serde(default)]
    pub installs: u64,
    /// 评分人数（展示用）。
    #[serde(default)]
    pub stars: u64,
    /// 图标内字形（2 个字符以内）。
    #[serde(default)]
    pub glyph: String,
    /// 是否已安装。
    #[serde(default)]
    pub installed: bool,
    /// 插件形态。旧数据没有该字段，按 `Text` 处理。
    #[serde(default)]
    pub kind: SkillKind,
    /// `Mcp` 形态的启动配置；`Text` 形态为 `None`。
    /// `env` 里可能放 token，`Settings::redacted()` 会清空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpServerConfig>,
}

impl Skill {
    pub fn is_mcp(&self) -> bool {
        self.kind == SkillKind::Mcp
    }
    /// 安装量的人性化显示：1_149_000 -> `1149.9万`。
    pub fn installs_label(&self) -> String {
        let n = self.installs;
        if n >= 10_000 {
            format!("{:.1}万", n as f64 / 10_000.0)
        } else {
            format!("{n}")
        }
    }

    pub fn stars_label(&self) -> String {
        if self.stars >= 10_000 {
            format!("{:.1}万", self.stars as f64 / 10_000.0)
        } else {
            format!("{}", self.stars)
        }
    }

    /// 综合评分（排序用）：安装量占主要权重，星数作补充。
    pub fn score(&self) -> f64 {
        (self.installs as f64).sqrt() + (self.stars as f64) * 0.5
    }
}

// ============ 工作区 ============

/// 一个最近打开过的工作区。
///
/// 只做「最近列表」，不扫描磁盘、不监听目录——工作区是否仍存在由切换时
/// `is_dir` 现场判断（列表里标「不存在」）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentWorkspace {
    /// 绝对路径，兼作唯一键。
    pub path: String,
    /// 用户重命名；空则显示目录名。
    #[serde(default)]
    pub name: String,
    /// 收藏：排序永远在前，且不会被上限淘汰。
    #[serde(default)]
    pub pinned: bool,
    /// 最近打开（Unix 秒）。排序用时间戳而不是字符串。
    #[serde(default)]
    pub opened_at: u64,
}

impl RecentWorkspace {
    /// 列表里显示的名字：自定义名 → 目录名 → 完整路径。
    pub fn display_name(&self) -> String {
        let n = self.name.trim();
        if !n.is_empty() {
            return n.to_string();
        }
        std::path::Path::new(&self.path)
            .file_name()
            .map(|x| x.to_string_lossy().to_string())
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| self.path.clone())
    }
}

// ============ 设置根 ============

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    // ---- 外观 ----
    #[serde(default)]
    pub theme: ThemeMode,
    /// 界面缩放百分比（80–160），影响左右栏宽度与行高。
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,

    // ---- 编辑器 ----
    /// 编辑器字号（px）。
    #[serde(default = "default_editor_font")]
    pub editor_font_size: f32,
    /// 显示行号。
    #[serde(default = "yes")]
    pub show_line_numbers: bool,
    /// Tab / 缩进宽度（空格数）。
    #[serde(default = "default_tab_width")]
    pub tab_width: u8,
    /// 自动保存（停止输入 800ms 后落盘）。
    #[serde(default)]
    pub auto_save: bool,
    /// 编辑器自动换行。
    #[serde(default)]
    pub word_wrap: bool,

    // ---- AI ----
    /// 厂商列表。
    #[serde(default = "default_providers")]
    pub providers: Vec<ProviderEntry>,
    /// 当前使用的厂商 id。
    #[serde(default)]
    pub active_provider_id: String,
    /// 单次运行（一条提问触发的全部轮次）的 token 预算，`0` 表示不限。
    ///
    /// 成本口径用 token 而不是货币：单价随模型与折扣变化，硬编码汇率
    /// 只会算出一个看起来很专业但错得离谱的数字。用量由各 Provider
    /// 从协议自带的 usage 字段解析，拿不到时按 0 计。
    #[serde(default)]
    pub max_tokens_per_run: u64,
    /// 单次运行最多几轮工具调用（一轮 = 一次模型调用 + 若干工具执行），`0` 表示不限。
    #[serde(default)]
    pub max_rounds_per_run: u32,
    /// 单次运行的最长墙钟秒数（含模型思考与工具执行），`0` 表示不限。
    #[serde(default)]
    pub max_run_seconds: u64,

    // ---- 技能 ----
    #[serde(default)]
    pub skills: Vec<Skill>,

    // ---- 工作区 ----
    /// 上次打开的工作区（绝对路径）。空则用内置默认。
    #[serde(default)]
    pub last_workspace: String,
    /// 最近打开过的工作区（收藏在前）。上限见 [`Settings::record_workspace`]。
    /// 会话按工作区隔离存储，切换工作区会重载对应会话库。
    #[serde(default)]
    pub workspaces: Vec<RecentWorkspace>,

    // ---- 同步与升级 ----
    /// 配置同步开关（导出/导入或云同步）。
    #[serde(default)]
    pub sync_enabled: bool,
    /// 上次同步时间（秒级时间戳）。
    #[serde(default)]
    pub last_sync_at: u64,
    /// 上次检查更新的时间。
    #[serde(default)]
    pub last_update_check: u64,

    // ---- 扩展 ----
    /// MCP 服务器（stdio）。空 = 不使用 MCP。
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    /// 启动 MCP 子进程时注入国内镜像（npm→npmmirror、PyPI→清华）。
    /// 用户环境里已设置对应变量时不覆盖。
    #[serde(default = "default_true")]
    pub mcp_mirror: bool,
}

fn default_ui_scale() -> f32 {
    100.0
}

fn default_editor_font() -> f32 {
    12.0
}

fn default_tab_width() -> u8 {
    4
}

fn default_providers() -> Vec<ProviderEntry> {
    // 预置主流厂商，都是未填 Key 的状态；用户填一个即可用。
    ProviderPreset::all()
        .iter()
        .filter(|p| **p != ProviderPreset::Custom)
        .map(|p| ProviderEntry::from_preset(*p))
        .collect()
}

impl Default for Settings {
    fn default() -> Self {
        let providers = default_providers();
        let active = providers
            .iter()
            .find(|p| p.preset == ProviderPreset::OpenAi)
            .map(|p| p.id.clone())
            .unwrap_or_default();
        Self {
            theme: ThemeMode::Dark,
            ui_scale: default_ui_scale(),
            editor_font_size: default_editor_font(),
            show_line_numbers: true,
            tab_width: default_tab_width(),
            auto_save: false,
            word_wrap: false,
            providers,
            active_provider_id: active,
            // 三项运行上限默认全为 0（不限）：读项目这类任务会被固定预算掐断，
            // 限制留给需要控制成本的人自己配。
            max_tokens_per_run: 0,
            max_rounds_per_run: 0,
            max_run_seconds: 0,
            skills: Vec::new(),
            last_workspace: String::new(),
            workspaces: Vec::new(),
            sync_enabled: false,
            last_sync_at: 0,
            last_update_check: 0,
            mcp_servers: Vec::new(),
            mcp_mirror: true,
        }
    }
}

impl Settings {
    /// 缩进用的空格串。
    pub fn indent(&self) -> String {
        " ".repeat(self.tab_width.clamp(1, 8) as usize)
    }

    pub fn active_provider(&self) -> Option<&ProviderEntry> {
        self.providers
            .iter()
            .find(|p| p.id == self.active_provider_id)
            .or_else(|| self.providers.iter().find(|p| p.ready()))
    }

    /// 已填 API Key 的厂商数量（本地服务不参与统计，它本来就不需要 Key）。
    pub fn keyed_provider_count(&self) -> usize {
        self.providers
            .iter()
            .filter(|p| !p.preset.is_local() && !p.api_key.trim().is_empty())
            .count()
    }

    /// 当前真正可用的厂商数量（含已就绪的本地服务）。
    pub fn usable_provider_count(&self) -> usize {
        self.providers.iter().filter(|p| p.ready()).count()
    }

    /// 已安装技能数。
    pub fn installed_skill_count(&self) -> usize {
        self.skills.iter().filter(|s| s.installed).count()
    }

    pub fn provider_mut(&mut self, id: &str) -> Option<&mut ProviderEntry> {
        self.providers.iter_mut().find(|p| p.id == id)
    }

    /// 记录一次工作区打开：同路径更新打开时间（保留自定义名与收藏），
    /// 超过上限时淘汰最久未打开且未收藏的。
    ///
    /// 收藏项不参与淘汰：用户特意星标的项目被挤掉，比列表少几条难受得多。
    pub fn record_workspace(&mut self, path: &str, now: u64) {
        const MAX_WORKSPACES: usize = 12;
        if path.trim().is_empty() {
            return;
        }
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.path == path) {
            w.opened_at = now;
            return;
        }
        self.workspaces.push(RecentWorkspace {
            path: path.to_string(),
            name: String::new(),
            pinned: false,
            opened_at: now,
        });
        while self.workspaces.len() > MAX_WORKSPACES {
            let victim = self
                .workspaces
                .iter()
                .enumerate()
                .filter(|(_, w)| !w.pinned)
                .min_by_key(|(_, w)| w.opened_at)
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.workspaces.remove(i);
                }
                None => break,
            }
        }
    }

    /// 工作区列表的展示顺序：收藏在前 → 最近打开优先。返回原数组下标。
    pub fn workspace_order(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.workspaces.len()).collect();
        idx.sort_by(|&a, &b| {
            self.workspaces[b]
                .pinned
                .cmp(&self.workspaces[a].pinned)
                .then(self.workspaces[b].opened_at.cmp(&self.workspaces[a].opened_at))
                .then(a.cmp(&b))
        });
        idx
    }

    /// 脱敏副本：清空所有厂商的 API Key。
    /// 导出、同步、截图分享都应走这个，绝不能直接序列化带密钥的 Settings。
    pub fn redacted(&self) -> Self {
        let mut s = self.clone();
        for p in &mut s.providers {
            p.api_key.clear();
        }
        // MCP 的 env 里常放 token（GITHUB_TOKEN 等），导出时必须清掉
        for m in &mut s.mcp_servers {
            m.env.clear();
        }
        // 已安装的 MCP 插件里也可能带 env
        for sk in &mut s.skills {
            if let Some(m) = &mut sk.mcp {
                m.env.clear();
            }
        }
        s
    }

    /// 已填 Key 的厂商列表（只返回名字，不含密钥）。
    pub fn keyed_provider_names(&self) -> Vec<String> {
        self.providers
            .iter()
            .filter(|p| !p.api_key.trim().is_empty())
            .map(|p| p.name.clone())
            .collect()
    }

    /// 下一个自定义厂商 id。
    pub fn next_custom_id(&self) -> String {
        let mut n = 1;
        loop {
            let id = format!("custom-{n}");
            if !self.providers.iter().any(|p| p.id == id) {
                return id;
            }
            n += 1;
        }
    }
}

/// 当前应用版本（用于「检查更新」展示）。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 从 `~/.neverwork/settings.json` 读取的设置目录。
pub fn settings_dir() -> std::path::PathBuf {
    crate::config::config_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_mainstream_providers() {
        let s = Settings::default();
        assert!(s.providers.len() >= 9, "应预置主流厂商");
        for p in &s.providers {
            assert!(!p.base_url.is_empty(), "{} 缺 base_url", p.name);
        }
        // 默认都没填 Key；只有本地 Ollama 因为自带 localhost 地址与默认模型算可用。
        assert_eq!(s.keyed_provider_count(), 0);
        assert_eq!(s.usable_provider_count(), 1);
    }

    #[test]
    fn protocol_mapping() {
        assert_eq!(ProviderPreset::Anthropic.protocol(), Protocol::Anthropic);
        assert_eq!(ProviderPreset::Gemini.protocol(), Protocol::Gemini);
        assert_eq!(ProviderPreset::DeepSeek.protocol(), Protocol::OpenAiCompatible);
        assert_eq!(ProviderPreset::Qwen.protocol(), Protocol::OpenAiCompatible);
    }

    #[test]
    fn ollama_is_local_and_ready_without_key() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::Ollama);
        assert!(p.preset.is_local());
        assert!(p.ready(), "本地服务有地址和默认模型即可用，不需要 Key");
        p.model = String::new();
        assert!(!p.ready(), "没有模型时不算就绪");
        p.model = "llama3.2".into();
        p.base_url = String::new();
        assert!(!p.ready(), "没有地址时不算就绪");
    }

    #[test]
    fn masked_key_only_keeps_tail() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::OpenAi);
        assert_eq!(p.masked_key(), "");
        p.api_key = "sk-1234567890abcdef".into();
        let m = p.masked_key();
        // 只保留末尾 4 位；前缀也不该出现（截图泄露风险）。
        assert!(m.ends_with("cdef"), "应保留末尾 4 位: {m}");
        assert!(!m.contains("sk-"), "不应泄露前缀: {m}");
        assert!(!m.contains("1234"), "不应泄露中段: {m}");
        assert!(m.contains('•'));
    }

    #[test]
    fn masked_key_short_is_fully_hidden() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::OpenAi);
        p.api_key = "abc".into();
        assert_eq!(p.masked_key(), "••••••••", "过短的 Key 应整体遮掉");
    }

    #[test]
    fn debug_output_never_leaks_key() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::OpenAi);
        p.api_key = "sk-super-secret-value".into();
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("super-secret"), "Debug 输出泄露了密钥: {dbg}");
        assert!(dbg.contains("ProviderEntry"));
    }

    #[test]
    fn redacted_clears_all_keys() {
        let mut s = Settings::default();
        for p in &mut s.providers {
            p.api_key = "sk-test".into();
        }
        let r = s.redacted();
        assert!(r.providers.iter().all(|p| p.api_key.is_empty()));
        // 原对象不受影响
        assert!(s.providers.iter().any(|p| !p.api_key.is_empty()));
    }

    #[test]
    fn keyed_provider_names_excludes_keys() {
        let mut s = Settings::default();
        s.providers[0].api_key = "sk-abc".into();
        let names = s.keyed_provider_names();
        assert_eq!(names.len(), 1);
        assert!(!names[0].contains("sk-"), "名单里不能带密钥");
    }

    #[test]
    fn indent_respects_tab_width() {
        let mut s = Settings::default();
        s.tab_width = 2;
        assert_eq!(s.indent(), "  ");
        s.tab_width = 0;
        assert_eq!(s.indent().len(), 1, "宽度要夹在 1..=8");
        s.tab_width = 200;
        assert_eq!(s.indent().len(), 8);
    }

    #[test]
    fn settings_roundtrip() {
        let mut s = Settings::default();
        s.theme = ThemeMode::Light;
        s.editor_font_size = 15.0;
        s.auto_save = true;
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.theme, ThemeMode::Light);
        assert_eq!(back.editor_font_size, 15.0);
        assert!(back.auto_save);
        assert_eq!(back.providers.len(), s.providers.len());
    }

    #[test]
    fn old_settings_without_new_fields_still_load() {
        // 模拟旧版本只有部分字段的配置。
        let json = r#"{"theme":"dark"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(s.show_line_numbers);
        assert_eq!(s.tab_width, 4);
        assert!(!s.providers.is_empty(), "缺失的厂商列表应回落到默认");
    }

    #[test]
    fn skill_install_label() {
        let mut k = Skill {
            id: "a".into(),
            name: "a".into(),
            summary: String::new(),
            detail: String::new(),
            category: SkillCategory::Dev,
            author: String::new(),
            installs: 1_149_000,
            stars: 119,
            glyph: "编".into(),
            installed: false,
            kind: SkillKind::Text,
            mcp: None,
        };
        assert_eq!(k.installs_label(), "114.9万");
        k.installs = 980;
        assert_eq!(k.installs_label(), "980");
    }

    #[test]
    fn skill_kind_defaults_to_text_and_mcp_is_optional() {
        // 旧数据没有 kind/mcp 字段：必须回落 Text，不能反序列化失败
        let json = r#"{"id":"a","name":"a","summary":"","category":"dev"}"#;
        let k: Skill = serde_json::from_str(json).unwrap();
        assert_eq!(k.kind, SkillKind::Text);
        assert!(k.mcp.is_none());
    }

    #[test]
    fn redacted_clears_skill_mcp_env() {
        let mut s = Settings::default();
        let mut m = McpServerConfig {
            name: "gh".into(),
            command: "npx".into(),
            ..Default::default()
        };
        m.env.insert("GITHUB_TOKEN".into(), "secret".into());
        s.skills.push(Skill {
            id: "mcp.gh".into(),
            name: "GitHub".into(),
            summary: String::new(),
            detail: String::new(),
            category: SkillCategory::Dev,
            author: String::new(),
            installs: 0,
            stars: 0,
            glyph: "".into(),
            installed: true,
            kind: SkillKind::Mcp,
            mcp: Some(m),
        });
        let r = s.redacted();
        assert!(r.skills[0].mcp.as_ref().unwrap().env.is_empty(), "导出时插件 env 必须清空");
    }

    #[test]
    fn custom_id_does_not_collide() {
        let mut s = Settings::default();
        assert_eq!(s.next_custom_id(), "custom-1");
        s.providers
            .push(ProviderEntry::from_preset(ProviderPreset::Custom));
        s.providers.last_mut().unwrap().id = "custom-1".into();
        assert_eq!(s.next_custom_id(), "custom-2");
    }

    #[test]
    fn record_workspace_upserts_and_keeps_name() {
        let mut s = Settings::default();
        s.record_workspace("E:\\AI\\a", 100);
        s.workspaces[0].name = "项目A".into();
        s.workspaces[0].pinned = true;
        s.record_workspace("E:\\AI\\a", 200);
        assert_eq!(s.workspaces.len(), 1, "同一路径不能重复入列");
        assert_eq!(s.workspaces[0].opened_at, 200);
        assert_eq!(s.workspaces[0].name, "项目A", "重新打开不能冲掉自定义名");
        assert!(s.workspaces[0].pinned, "重新打开不能冲掉收藏");
    }

    #[test]
    fn record_workspace_evicts_oldest_unpinned_but_keeps_pinned() {
        let mut s = Settings::default();
        s.record_workspace("E:\\w0", 1);
        s.workspaces[0].pinned = true;
        for i in 1..=12u64 {
            s.record_workspace(&format!("E:\\w{i}"), i * 10);
        }
        assert_eq!(s.workspaces.len(), 12, "上限 12 条");
        assert!(
            s.workspaces.iter().any(|w| w.path == "E:\\w0"),
            "收藏项不参与淘汰"
        );
        assert!(
            !s.workspaces.iter().any(|w| w.path == "E:\\w1"),
            "最久未打开的先被淘汰"
        );
    }

    #[test]
    fn workspace_order_pinned_first_then_recent() {
        let mut s = Settings::default();
        s.record_workspace("E:\\a", 100);
        s.record_workspace("E:\\b", 200);
        s.record_workspace("E:\\c", 50);
        s.workspaces[2].pinned = true; // c 收藏
        let order = s.workspace_order();
        assert_eq!(s.workspaces[order[0]].path, "E:\\c");
        assert_eq!(s.workspaces[order[1]].path, "E:\\b");
        assert_eq!(s.workspaces[order[2]].path, "E:\\a");
    }

    #[test]
    fn recent_workspace_display_name_falls_back_to_dir() {
        let mut w = RecentWorkspace {
            path: "E:\\AI\\ide".into(),
            name: String::new(),
            pinned: false,
            opened_at: 0,
        };
        assert_eq!(w.display_name(), "ide");
        w.name = "心爱项目".into();
        assert_eq!(w.display_name(), "心爱项目");
    }

    #[test]
    fn old_settings_without_workspaces_load() {
        let json = r#"{"theme":"dark"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(s.workspaces.is_empty(), "旧配置不能因缺字段整库失效");
    }
}
