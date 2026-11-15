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

/// 一条厂商配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Key 打码显示（只留前 4 后 4）。
    pub fn masked_key(&self) -> String {
        let k = self.api_key.trim();
        if k.is_empty() {
            return String::new();
        }
        let n = k.chars().count();
        if n <= 10 {
            return "•".repeat(n.max(4));
        }
        let head: String = k.chars().take(4).collect();
        let tail: String = k.chars().skip(n - 4).collect();
        format!("{head}••••••{tail}")
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
}

impl Skill {
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

    // ---- 技能 ----
    #[serde(default)]
    pub skills: Vec<Skill>,

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
            skills: Vec::new(),
            sync_enabled: false,
            last_sync_at: 0,
            last_update_check: 0,
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
    fn masked_key_hides_middle() {
        let mut p = ProviderEntry::from_preset(ProviderPreset::OpenAi);
        assert_eq!(p.masked_key(), "");
        p.api_key = "sk-1234567890abcdef".into();
        let m = p.masked_key();
        assert!(m.starts_with("sk-1"));
        assert!(m.contains('•'));
        assert!(!m.contains("6789"), "中间部分不应出现");
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
        };
        assert_eq!(k.installs_label(), "114.9万");
        k.installs = 980;
        assert_eq!(k.installs_label(), "980");
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
}
