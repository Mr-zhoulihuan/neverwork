//! 真实技能市场：数据全部来自公开源，不再有硬编码的虚构条目。
//!
//! 三个来源：
//! - `Official`：MCP 官方 Registry（`registry.modelcontextprotocol.io`），
//!   条目自带 npm / PyPI / OCI 包信息，可直接转成 stdio 启动配置。
//! - `Tencent` ：腾讯云 MCP 广场（国内直达）。列表页 SSR 里嵌着 30 条真实
//!   条目的 JSON；安装时再抓详情页，从页面里的 `{"mcpServers":{...}}` 取
//!   真实启动命令。
//! - `SkillsSh`：skills.sh（Agent Skills 目录）。首页服务端渲染的热榜用于
//!   浏览，`/api/search` 用于搜索；列表不带正文，安装时按 GitHub 仓库
//!   （或站点 `.well-known/agent-skills`）解析出 `SKILL.md` 再落盘。
//!
//! 网络失败时读 `~/.neverwork/marketplace.json` 缓存并在 UI 标注「上次同步」；
//! 从未成功过就把错误原因交给 UI 显示，而不是回退到编造的条目。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use neverwork_core::{McpServerConfig, Skill, SkillCategory, SkillKind};
use serde::{Deserialize, Serialize};

/// 官方 Registry 列表接口（v0.1，已 freeze）。
pub const OFFICIAL_LIST: &str = "https://registry.modelcontextprotocol.io/v0/servers";
/// 一次拉多少条（v0 默认分页很小，这里取一页足够浏览的量）。
pub const OFFICIAL_LIMIT: usize = 100;
/// 腾讯云 MCP 广场列表页（SSR，带 30 条热门条目的 JSON）。
pub const TENCENT_LIST: &str = "https://cloud.tencent.com/developer/mcp";
/// 腾讯云详情页前缀（`{前缀}{mcpId}`）。
pub const TENCENT_DETAIL: &str = "https://cloud.tencent.com/developer/mcp/server/";
/// skills.sh 站点根（首页热榜与详情页共用）。
pub const SKILLS_SH: &str = "https://www.skills.sh";
/// skills.sh 搜索接口（`q` 至少 2 个字符）。
pub const SKILLS_SH_SEARCH: &str = "https://www.skills.sh/api/search";
/// jsDelivr 目录接口：列 GitHub 仓库某分支下的全部文件，用来定位 SKILL.md。
pub const JSDELIVR_DATA: &str = "https://data.jsdelivr.com/v1/packages/gh";
/// jsDelivr 文件 CDN（国内可达性好；失败回退 raw.githubusercontent.com）。
pub const JSDELIVR_CDN: &str = "https://cdn.jsdelivr.net/gh";
/// 搜索一次最多取多少条（上游上限 200）。
const SKILLS_SH_LIMIT: usize = 200;
/// 首页热榜里安装量单元格的 class 前缀（只取其后到 `<` 的文本）。
const INSTALLS_MARK: &str = "font-mono text-sm text-foreground\">";
/// 首页热榜最多收多少条（首页约 80 行，留一点余量）。
const SKILLS_SH_HOME_MAX: usize = 100;
/// 单个 SKILL.md 正文上限：超过就截断（防止设置文件与提示词过大）。
const SKILL_BODY_LIMIT: usize = 200_000;
/// 单个图标上限：超过就不缓存（有的图标地址指向的是大截图）。
const ICON_MAX_BYTES: u64 = 256 * 1024;
/// 并发下载图标的线程数（TLS 握手占大头，并发能线性提速）。
const ICON_CONCURRENCY: usize = 8;

// ============ 来源 ============

/// 市场数据来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketSource {
    #[serde(rename = "official")]
    Official,
    #[serde(rename = "tencent")]
    Tencent,
    #[serde(rename = "skillssh")]
    SkillsSh,
}

impl MarketSource {
    pub fn all() -> [MarketSource; 3] {
        [MarketSource::Official, MarketSource::Tencent, MarketSource::SkillsSh]
    }

    /// 缓存与状态键。
    pub fn key(&self) -> &'static str {
        match self {
            MarketSource::Official => "official",
            MarketSource::Tencent => "tencent",
            MarketSource::SkillsSh => "skillssh",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            MarketSource::Official => "MCP 官方",
            MarketSource::Tencent => "腾讯云广场",
            MarketSource::SkillsSh => "skills.sh",
        }
    }

    /// 数据来自哪里（状态行展示）。
    pub fn origin(&self) -> &'static str {
        match self {
            MarketSource::Official => "registry.modelcontextprotocol.io",
            MarketSource::Tencent => "cloud.tencent.com",
            MarketSource::SkillsSh => "www.skills.sh",
        }
    }

    /// 来源主色（卡片图标底色；只表示数据从哪来，不代表分类）。
    pub fn hue(&self) -> u32 {
        match self {
            MarketSource::Official => 0xa855f7,
            MarketSource::Tencent => 0x3b82f6,
            MarketSource::SkillsSh => 0x10b981,
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "official" => Some(MarketSource::Official),
            "tencent" => Some(MarketSource::Tencent),
            "skillssh" => Some(MarketSource::SkillsSh),
            _ => None,
        }
    }
}

// ============ 条目与缓存 ============

/// skills.sh 条目的上游身份：来源（`owner/repo` 或站点域名）与技能 id。
///
/// 列表接口只给名字和安装量，正文要等安装时按这对值去仓库/站点解析。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSkillRef {
    pub source: String,
    pub id: String,
}

/// 市场条目：技能本体 + 下载来源标注。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketEntry {
    #[serde(flatten)]
    pub skill: Skill,
    /// skills.sh 条目的上游身份；安装时按它解析 `SKILL.md` 正文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<RemoteSkillRef>,
    /// 只有远程端点（SSE / streamable-http），本项目只支持 stdio，不能装。
    #[serde(default)]
    pub remote_only: bool,
    /// 数据源自带标签（腾讯云的 `categoryList`、官方的包形态、GitHub 仓库名）。
    /// 全部来自上游字段，不做关键词猜测；旧缓存没有该字段时为空。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 上游图标地址（腾讯云 `iconUrl`、GitHub 仓库 owner 头像）。
    /// 刷新时下载到本地缓存，渲染层用 [`icon_path`] 取文件，不在这里存字节。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// 上游详情页（详情抽屉里的「打开来源」）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl MarketEntry {
    pub fn new(skill: Skill) -> Self {
        Self {
            skill,
            remote: None,
            remote_only: false,
            tags: Vec::new(),
            icon_url: None,
            source_url: None,
        }
    }
}

/// 去重后追加一个标签。
fn push_tag(tags: &mut Vec<String>, t: &str) {
    if !t.is_empty() && !tags.iter().any(|x| x == t) {
        tags.push(t.to_string());
    }
}

/// 一个来源的缓存快照。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SourceSnapshot {
    /// 上次成功拉取的时间（秒级时间戳）。
    #[serde(default)]
    pub fetched_at: u64,
    #[serde(default)]
    pub entries: Vec<MarketEntry>,
}

/// 全部来源的缓存。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketCache {
    #[serde(default)]
    pub sources: BTreeMap<String, SourceSnapshot>,
}

impl MarketCache {
    pub fn snapshot(&self, source: MarketSource) -> Option<&SourceSnapshot> {
        self.sources.get(source.key()).filter(|s| !s.entries.is_empty())
    }

    pub fn put(&mut self, source: MarketSource, entries: Vec<MarketEntry>, fetched_at: u64) {
        self.sources.insert(
            source.key().to_string(),
            SourceSnapshot {
                fetched_at,
                entries,
            },
        );
    }
}

pub fn cache_path() -> PathBuf {
    neverwork_core::config::config_dir().join("marketplace.json")
}

pub fn load_cache() -> MarketCache {
    load_cache_from(&cache_path())
}

pub fn load_cache_from(path: &Path) -> MarketCache {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save_cache(cache: &MarketCache) -> Result<(), String> {
    save_cache_to(cache, &cache_path())
}

pub fn save_cache_to(cache: &MarketCache, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建缓存目录失败：{e}"))?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(|e| format!("序列化缓存失败：{e}"))?;
    std::fs::write(path, text).map_err(|e| format!("写入缓存失败：{e}"))
}

/// 「上次同步」的人性化时间：刚刚 / N 分钟前 / N 小时前 / 日期。
pub fn ago_label(fetched_at: u64) -> String {
    let now = neverwork_core::config::now_secs();
    if fetched_at == 0 || fetched_at >= now {
        return "刚刚".into();
    }
    let d = now - fetched_at;
    if d < 60 {
        "刚刚".into()
    } else if d < 3600 {
        format!("{} 分钟前", d / 60)
    } else if d < 86_400 {
        format!("{} 小时前", d / 3600)
    } else {
        format!("{} 天前", d / 86_400)
    }
}

// ============ HTTP ============

fn agent() -> ureq::Agent {
    agent_with_timeout(30)
}

/// 单次请求总超时（GitHub 逐文件下载用短超时，避免一个卡住拖死整批）。
fn agent_with_timeout(secs: u64) -> ureq::Agent {
    let mut b = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(secs)))
        .user_agent("neverwork-market/0.1");
    // 尊重系统代理（ureq 默认不读环境变量，必须显式尝试）
    if let Some(p) = ureq::Proxy::try_from_env() {
        b = b.proxy(Some(p));
    }
    b.build().new_agent()
}

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| http_error(url, e))?;
    resp.body_mut()
        .read_to_string()
        .map_err(|e| format!("读取响应失败：{e}"))
}

fn get_json(agent: &ureq::Agent, url: &str) -> Result<serde_json::Value, String> {
    let mut resp = agent
        .get(url)
        .header("Accept", "application/json")
        .call()
        .map_err(|e| http_error(url, e))?;
    resp.body_mut()
        .read_json()
        .map_err(|e| format!("解析 JSON 失败：{e}"))
}

fn http_error(url: &str, e: ureq::Error) -> String {
    let host = url
        .split('/')
        .nth(2)
        .unwrap_or(url)
        .to_string();
    match e {
        ureq::Error::StatusCode(code) => match code {
            401 | 403 => format!("{host} 拒绝访问（HTTP {code}，可能触发了限流）"),
            404 => format!("{host} 上找不到该条目（HTTP 404）"),
            429 => format!("{host} 请求过于频繁（HTTP 429），稍后再试"),
            _ => format!("{host} 返回 HTTP {code}"),
        },
        other => format!("连接 {host} 失败：{other}"),
    }
}

// ============ 图标缓存 ============

/// 图标缓存目录（`~/.neverwork/market-icons`）。
pub fn icon_cache_dir() -> PathBuf {
    neverwork_core::config::config_dir().join("market-icons")
}

/// URL → 本地缓存文件。FNV-1a 64 位足够避开撞名，且不引入额外 crate。
pub fn icon_path_for(url: &str) -> PathBuf {
    icon_cache_dir().join(format!("{:016x}.img", fnv1a(url)))
}

/// 条目图标的本地文件；没图标或还没下载完时返回 `None`（调用方回退字形）。
pub fn icon_path(entry: &MarketEntry) -> Option<PathBuf> {
    let url = entry.icon_url.as_deref()?;
    let path = icon_path_for(url);
    path.is_file().then_some(path)
}

/// 条目来源的展示文案：由 id 前缀推断（市场条目 id 带源前缀，本地导入没有）。
/// 技能页详情抽屉与团队插件面板共用，避免两处文案不一致。
pub fn source_label(id: &str) -> &'static str {
    if id.starts_with("tencent.") {
        "腾讯云广场"
    } else if id.starts_with("ss.") {
        "skills.sh"
    } else if id.starts_with("gh.") {
        // 旧版 GitHub 技能源装下的条目：市场不再展示，但已装的可继续用
        "GitHub 技能"
    } else if id.starts_with("mcp.") {
        "MCP 官方"
    } else {
        "本地安装"
    }
}

/// 标准 FNV-1a 64 位（prime = 0x100000001B3）。
/// 注意：改哈希算法/幂等错一个字都会让旧缓存名失配，图标会重新下载一轮。
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 按魔数判断是不是图片。
///
/// 扩展名无关：缓存一律写成 `.img`，gpui 也按字节嗅探（`image::guess_format`，
/// 失败再当 SVG 渲染）。SVG 也收：头像/logo 不少是矢量图。
pub fn looks_like_image(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return true;
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return true;
    }
    if bytes.starts_with(b"GIF8") {
        return true;
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return true;
    }
    let head = &bytes[..bytes.len().min(200)];
    let text = String::from_utf8_lossy(head);
    let text = text.trim_start();
    text.starts_with("<svg")
        || text.starts_with("<?xml")
        || (text.starts_with('<') && text.contains("<svg"))
}

/// 把条目的 `icon_url` 下载到本地缓存；已存在跳过，失败静默忽略。
///
/// 图标只是装饰：任何一个失败都不影响列表展示（卡片回退字形）。
pub fn cache_icons(entries: &[MarketEntry]) {
    let mut urls: Vec<&str> = Vec::new();
    for e in entries {
        if let Some(u) = e.icon_url.as_deref() {
            if !urls.contains(&u) {
                urls.push(u);
            }
        }
    }
    let todo: Vec<&str> = urls
        .into_iter()
        .filter(|u| !icon_path_for(u).is_file())
        .collect();
    if todo.is_empty() || std::fs::create_dir_all(icon_cache_dir()).is_err() {
        return;
    }
    let agent = agent_with_timeout(8);
    for chunk in todo.chunks(ICON_CONCURRENCY) {
        std::thread::scope(|scope| {
            for url in chunk {
                let agent = agent.clone();
                scope.spawn(move || {
                    if let Ok(bytes) = fetch_bytes(&agent, url, ICON_MAX_BYTES) {
                        if looks_like_image(&bytes) {
                            let _ = std::fs::write(icon_path_for(url), bytes);
                        }
                    }
                });
            }
        });
    }
}

fn fetch_bytes(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut resp = agent.get(url).call().map_err(|e| http_error(url, e))?;
    resp.body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| format!("读取响应失败：{e}"))
}

// ============ 拉取入口 ============

/// 拉取某个来源的全部条目。
pub fn fetch(source: MarketSource) -> Result<Vec<MarketEntry>, String> {
    match source {
        MarketSource::Official => fetch_official(),
        MarketSource::Tencent => fetch_tencent(),
        MarketSource::SkillsSh => fetch_skills_sh_home(),
    }
}

/// 拉取并写入缓存。返回 (条目, 拉取时间)。
pub fn fetch_and_cache(source: MarketSource) -> Result<(Vec<MarketEntry>, u64), String> {
    let entries = fetch(source)?;
    // 图标在这里落盘：UI 线程拿到条目时本地文件已就绪，卡片首帧就能显示。
    cache_icons(&entries);
    let at = neverwork_core::config::now_secs();
    let mut cache = load_cache();
    cache.put(source, entries.clone(), at);
    // 缓存失败不影响本次展示，但留痕方便排障
    if let Err(e) = save_cache(&cache) {
        tracing::warn!("保存市场缓存失败：{e}");
    }
    Ok((entries, at))
}

// ============ 官方 Registry ============

fn fetch_official() -> Result<Vec<MarketEntry>, String> {
    let agent = agent();
    let url = format!("{OFFICIAL_LIST}?limit={OFFICIAL_LIMIT}");
    let v = get_json(&agent, &url)?;
    let entries = parse_official(&v);
    if entries.is_empty() {
        return Err("官方 Registry 返回了空列表".into());
    }
    Ok(entries)
}

/// 解析官方 Registry 的 `/v0/servers` 响应。
pub fn parse_official(v: &serde_json::Value) -> Vec<MarketEntry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut used_names = HashSet::new();
    let Some(list) = v.get("servers").and_then(|s| s.as_array()) else {
        return out;
    };
    for item in list {
        let Some(server) = item.get("server") else {
            continue;
        };
        let raw_name = server
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .trim();
        if raw_name.is_empty() || !seen.insert(raw_name.to_string()) {
            continue;
        }
        let fallback_title = short_name(raw_name);
        let title = server
            .get("title")
            .and_then(|x| x.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fallback_title.as_str());
        let desc = server
            .get("description")
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        let repo = server
            .get("repository")
            .and_then(|r| r.get("url"))
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        let (cfg, envs) = official_package_config(server, raw_name, &mut used_names);
        let remote_only = cfg.is_none();
        let tags = official_tags(server, remote_only);
        // registry 没有图标字段：有 GitHub 仓库就用 owner 头像，没有就回退字形。
        let icon_url = repo_owner(repo).map(|o| github_avatar(&o));
        let source_url = if repo.trim().is_empty() {
            format!("{OFFICIAL_LIST}?search={}", raw_name.replace(' ', "%20"))
        } else {
            repo.trim().trim_end_matches(".git").to_string()
        };
        let mut summary = clean_text(desc, 160);
        if summary.is_empty() {
            summary = format!("来自 MCP 官方 Registry：{raw_name}");
        }
        if !envs.is_empty() {
            summary.push_str(&format!("（安装后需填环境变量：{}）", envs.join("、")));
        } else if remote_only {
            summary.push_str("（仅提供远程端点，暂不支持安装）");
        }
        let mut skill = Skill {
            id: format!("mcp.{}", slug_id(raw_name)),
            name: clean_text(title, 40),
            summary,
            detail: format!(
                "## 来源\n\nMCP 官方 Registry：`{raw_name}`\n\n## 说明\n\n{}\n",
                if desc.trim().is_empty() { "（上游未提供描述）" } else { desc.trim() }
            ),
            category: SkillCategory::Dev,
            author: clean_text(
                repo_owner(repo).as_deref().unwrap_or(&short_name(raw_name)),
                30,
            ),
            installs: 0,
            stars: 0,
            glyph: "Ⓜ".into(),
            installed: false,
            kind: SkillKind::Mcp,
            mcp: cfg,
        };
        if skill.name.is_empty() {
            skill.name = short_name(raw_name);
        }
        out.push(MarketEntry {
            skill,
            remote: None,
            remote_only,
            tags,
            icon_url,
            source_url: Some(source_url),
        });
    }
    out
}

/// 官方 registry 没有分类字段：标签只反映真实包形态（npm / PyPI / Docker / 仅远程）。
fn official_tags(server: &serde_json::Value, remote_only: bool) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(packages) = server.get("packages").and_then(|p| p.as_array()) {
        for p in packages {
            match p.get("registryType").and_then(|x| x.as_str()).unwrap_or("") {
                "npm" => push_tag(&mut tags, "npm"),
                "pypi" => push_tag(&mut tags, "PyPI"),
                "oci" => push_tag(&mut tags, "Docker"),
                _ => {}
            }
        }
    }
    if remote_only {
        push_tag(&mut tags, "仅远程");
    }
    tags
}

/// 从 `packages[]` 里挑一个能转 stdio 的包，返回 (启动配置, 必填环境变量名)。
fn official_package_config(
    server: &serde_json::Value,
    raw_name: &str,
    used_names: &mut HashSet<String>,
) -> (Option<McpServerConfig>, Vec<String>) {
    let mut envs = Vec::new();
    let Some(packages) = server.get("packages").and_then(|p| p.as_array()) else {
        return (None, envs);
    };
    for p in packages {
        let kind = p.get("registryType").and_then(|x| x.as_str()).unwrap_or("");
        let identifier = p.get("identifier").and_then(|x| x.as_str()).unwrap_or("");
        if identifier.is_empty() {
            continue;
        }
        let hint = p.get("runtimeHint").and_then(|x| x.as_str()).unwrap_or("");
        let mut args: Vec<String> = Vec::new();
        let command = match kind {
            "npm" => {
                args.push("-y".into());
                if hint.is_empty() {
                    "npx".to_string()
                } else {
                    hint.to_string()
                }
            }
            "pypi" => {
                if hint.is_empty() {
                    "uvx".to_string()
                } else {
                    hint.to_string()
                }
            }
            "oci" => {
                args.push("run".into());
                args.push("-i".into());
                args.push("--rm".into());
                if hint.is_empty() {
                    "docker".to_string()
                } else {
                    hint.to_string()
                }
            }
            _ => continue,
        };
        // runtimeArguments 里常有 `-y` 之类的附加参数
        if let Some(extra) = p.get("runtimeArguments").and_then(|x| x.as_array()) {
            for a in extra {
                if let Some(v) = a.get("value").and_then(|x| x.as_str()) {
                    let v = v.trim();
                    if !v.is_empty() && !args.iter().any(|x| x == v) {
                        args.push(v.to_string());
                    }
                }
            }
        }
        args.push(identifier.to_string());
        if let Some(env_list) = p.get("environmentVariables").and_then(|x| x.as_array()) {
            for e in env_list {
                if e.get("isRequired").and_then(|x| x.as_bool()).unwrap_or(false) {
                    if let Some(n) = e.get("name").and_then(|x| x.as_str()) {
                        envs.push(n.to_string());
                    }
                }
            }
        }
        let base = slug_id(&short_name(raw_name));
        let mut name = base.clone();
        let mut n = 2;
        while !used_names.insert(name.clone()) {
            name = format!("{base}-{n}");
            n += 1;
        }
        return (
            Some(McpServerConfig {
                name,
                command,
                args,
                ..Default::default()
            }),
            envs,
        );
    }
    (None, envs)
}

/// `io.github.owner/repo` → `repo`；`com.x/y` → `y`。
fn short_name(raw: &str) -> String {
    raw.rsplit('/').next().unwrap_or(raw).to_string()
}

/// 从仓库 URL 里取 owner：`https://github.com/foo/bar` → `foo`。
fn repo_owner(url: &str) -> Option<String> {
    let cleaned = url.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = cleaned.rsplit('/');
    parts.next()?;
    parts.next().map(|s| s.to_string()).filter(|s| !s.is_empty())
}

/// GitHub owner 头像。
///
/// 必须走 `github.com/{owner}.png`：它会 302 到 `avatars.githubusercontent.com/u/<id>?v=4`
/// 返回真实头像；而 `avatars.githubusercontent.com/{owner}` 不带 id 查询串时
/// 只会返回 GitHub 的默认八爪猫图（实测 5065B，所有 owner 都一样），不能用。
/// ureq 默认跟随重定向（上限 10 次），直接请求即可。
fn github_avatar(owner: &str) -> String {
    format!("https://github.com/{owner}.png")
}

// ============ 腾讯云广场 ============

fn fetch_tencent() -> Result<Vec<MarketEntry>, String> {
    let agent = agent();
    let html = get_text(&agent, TENCENT_LIST)?;
    let entries = parse_tencent_list(&html);
    if entries.is_empty() {
        return Err("腾讯云广场页面里没有解析到条目（页面结构可能变了）".into());
    }
    Ok(entries)
}

/// 解析腾讯云 MCP 广场列表页（SSR 里嵌入的 `"mcp":{"list":[...]}`）。
pub fn parse_tencent_list(html: &str) -> Vec<MarketEntry> {
    // 两种形态：合法 JSON 直接截取；JS 字符串形式的转义 payload 先反转义再截取。
    const PLAIN: &str = "\"mcp\":{\"list\":[";
    const ESCAPED: &str = "\\\"mcp\\\":{\\\"list\\\":[";
    let (json_source, arr_start) = if let Some(pos) = html.find(PLAIN) {
        (std::borrow::Cow::Borrowed(html), pos + PLAIN.len() - 1)
    } else if let Some(pos) = html.find(ESCAPED) {
        let tail = unescape_json(&html[pos..]);
        let Some(p) = tail.find(PLAIN) else {
            return Vec::new();
        };
        let start = p + PLAIN.len() - 1;
        (std::borrow::Cow::Owned(tail), start)
    } else {
        return Vec::new();
    };
    let Some(arr_text) = extract_json_array(&json_source[arr_start..]) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str(&arr_text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let id = item.get("mcpId").and_then(|x| x.as_u64()).unwrap_or(0);
        let name = item
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .trim();
        if id == 0 || name.is_empty() {
            continue;
        }
        let desc = item
            .get("abstract")
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        let author = item
            .get("srcAuthor")
            .and_then(|x| x.as_str())
            .unwrap_or("tencent");
        let read_num = item.get("readNum").and_then(|x| x.as_u64()).unwrap_or(0);
        let hosted = item.get("isHosted").and_then(|x| x.as_u64()).unwrap_or(0) == 1;
        let cats: Vec<String> = item
            .get("categoryList")
            .and_then(|x| x.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|c| c.get("name").and_then(|x| x.as_str()))
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        // 真实分类直接当标签展示（腾讯云自己的 12 类），不再映射到本地枚举
        let mut tags: Vec<String> = Vec::new();
        for c in &cats {
            push_tag(&mut tags, c);
            if tags.len() >= 3 {
                break;
            }
        }
        let mut summary = clean_text(desc, 160);
        if summary.is_empty() {
            summary = format!("腾讯云 MCP 广场：{name}");
        }
        if hosted {
            summary.push_str("（腾讯云托管版，远程端点，暂不支持安装）");
        }
        // 上游自带的图标（qcloudimg CDN 或 github 头像），空串按没有处理。
        let icon_url = item
            .get("iconUrl")
            .and_then(|x| x.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let skill = Skill {
            id: format!("tencent.mcp.{id}"),
            name: clean_text(name, 40),
            summary,
            detail: format!(
                "## 来源\n\n腾讯云 MCP 广场：{}detail/{id}\n\n## 说明\n\n{}\n",
                TENCENT_DETAIL,
                if desc.trim().is_empty() { "（上游未提供描述）" } else { desc.trim() }
            ),
            category: SkillCategory::Dev, // 市场筛选不看它，真实分类在 tags 里
            author: clean_text(author, 30),
            installs: read_num,
            stars: 0,
            glyph: "云".into(),
            installed: false,
            kind: SkillKind::Mcp,
            mcp: None, // 安装时再抓详情页取真实启动配置
        };
        out.push(MarketEntry {
            skill,
            remote: None,
            remote_only: hosted,
            tags,
            icon_url,
            source_url: Some(format!("{TENCENT_DETAIL}{id}")),
        });
    }
    out
}

/// 抓腾讯云条目详情页，解析出真实启动配置。
pub fn fetch_tencent_detail(mcp_id: u64) -> Result<McpServerConfig, String> {
    let agent = agent();
    let url = format!("{TENCENT_DETAIL}{mcp_id}");
    let html = get_text(&agent, &url)?;
    parse_tencent_detail(&html)
}

/// 从详情页里取 `{"mcpServers":{...}}` 的第一条命令配置。
pub fn parse_tencent_detail(html: &str) -> Result<McpServerConfig, String> {
    let text = decode_html_entities(&strip_tags(html));
    let Some(idx) = text.find("\"mcpServers\"") else {
        return Err("该条目没有提供 stdio 安装配置（可能是腾讯云托管版或页面结构已变）".into());
    };
    // 从 "mcpServers" 往前找到外层对象起点
    let start = text[..idx].rfind('{').ok_or("详情页配置片段不完整")?;
    let obj = extract_json_object(&text[start..]).ok_or("详情页配置 JSON 不完整")?;
    let v: serde_json::Value =
        serde_json::from_str(&obj).map_err(|e| format!("解析详情页配置失败：{e}"))?;
    let Some(servers) = v.get("mcpServers").and_then(|x| x.as_object()) else {
        return Err("详情页里没有 mcpServers 配置".into());
    };
    for (name, cfg) in servers {
        let Some(command) = cfg.get("command").and_then(|x| x.as_str()) else {
            continue;
        };
        if command.trim().is_empty() {
            continue;
        }
        let args = cfg
            .get("args")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let env = cfg
            .get("env")
            .and_then(|x| x.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        return Ok(McpServerConfig {
            name: slug_id(name),
            command: command.trim().to_string(),
            args,
            env,
            enabled: true,
        });
    }
    Err("详情页配置里没有可用的 command（该条目可能只提供远程端点）".into())
}

// ============ skills.sh ============

/// 首页热榜：`https://www.skills.sh/` 服务端渲染的表格行。
fn fetch_skills_sh_home() -> Result<Vec<MarketEntry>, String> {
    let agent = agent_with_timeout(20);
    let html = get_text(&agent, &format!("{SKILLS_SH}/"))?;
    let entries = parse_skills_sh_home(&html);
    if entries.is_empty() {
        return Err("skills.sh 首页没有解析到条目（页面结构可能变了）".into());
    }
    Ok(entries)
}

/// skills.sh 搜索：`q` 至少 2 个字符（上游限制）。
pub fn search_skills_sh(query: &str) -> Result<Vec<MarketEntry>, String> {
    let q = query.trim();
    if q.chars().count() < 2 {
        return Err("skills.sh 搜索至少需要 2 个字符".into());
    }
    let agent = agent_with_timeout(20);
    let url = format!("{SKILLS_SH_SEARCH}?q={}&limit={SKILLS_SH_LIMIT}", urlencode(q));
    let v = get_json(&agent, &url)?;
    let entries = parse_skills_sh_search(&v);
    if entries.is_empty() {
        return Err(format!("skills.sh 没有找到与「{q}」相关的技能"));
    }
    Ok(entries)
}

/// 解析首页热榜 HTML。
///
/// 每行的稳定特征：`href="/<source>/<skillId>"` + `<h3 …>name</h3>` +
/// `<p …>source</p>`，安装量在最右列（如 `4.4M`）。
pub fn parse_skills_sh_home(html: &str) -> Vec<MarketEntry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut rest = html;
    while let Some(pos) = rest.find("href=\"/") {
        let after = &rest[pos + 7..];
        let Some(end) = after.find('"') else { break };
        let path = &after[..end];
        rest = &after[end..];
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() < 2 || segs.len() > 3 {
            continue;
        }
        if !segs.iter().all(|s| valid_segment(s)) {
            continue;
        }
        let Some(anchor_end) = rest.find("</a>") else { break };
        let row = &rest[..anchor_end];
        rest = &rest[anchor_end..];
        // 技能行一定有 <h3>名称</h3>；没有就当成普通链接跳过
        let Some(name) = html_between(row, "<h3", "</h3>") else {
            continue;
        };
        let source = html_between(row, "<p", "</p>")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| segs[..segs.len() - 1].join("/"));
        let skill_id = segs[segs.len() - 1];
        if !seen.insert(format!("{source}/{skill_id}")) {
            continue;
        }
        let installs = row
            .find(INSTALLS_MARK)
            .and_then(|i| {
                let tail = &row[i + INSTALLS_MARK.len()..];
                let end = tail.find('<')?;
                Some(parse_installs_label(&tail[..end]))
            })
            .unwrap_or(0);
        out.push(skills_sh_entry(&source, skill_id, &name, installs, false));
        if out.len() >= SKILLS_SH_HOME_MAX {
            break;
        }
    }
    out
}

/// 解析搜索接口的 `{"skills":[{source, skillId, name, installs}]}`。
pub fn parse_skills_sh_search(v: &serde_json::Value) -> Vec<MarketEntry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let Some(list) = v.get("skills").and_then(|x| x.as_array()) else {
        return out;
    };
    for item in list {
        let source = item
            .get("source")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .trim();
        let id = item
            .get("skillId")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .trim();
        if !valid_remote_source(source) || !valid_segment(id) {
            continue;
        }
        if !seen.insert(format!("{source}/{id}")) {
            continue;
        }
        let name = item.get("name").and_then(|x| x.as_str()).unwrap_or(id);
        let installs = item.get("installs").and_then(|x| x.as_u64()).unwrap_or(0);
        let official = item
            .get("isOfficial")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        out.push(skills_sh_entry(source, id, name, installs, official));
    }
    out
}

/// 一个 skills.sh 条目：列表只带展示信息与上游身份，正文等安装时再解析。
fn skills_sh_entry(source: &str, id: &str, name: &str, installs: u64, official: bool) -> MarketEntry {
    let source = source.trim();
    let id = id.trim();
    // 来源已在作者与摘要里展示；不再把仓库名当标签，否则筛选条会被上百个仓库名挤爆
    let mut tags: Vec<String> = Vec::new();
    if official {
        push_tag(&mut tags, "官方");
    }
    let url = skill_page_url(source, id);
    let display = if name.trim().is_empty() { id } else { name.trim() };
    let mut skill = Skill {
        id: format!("ss.{}.{}", slug_id(source), slug_id(id)),
        name: clean_text(display, 40),
        summary: String::new(),
        detail: format!(
            "## 来源\n\nskills.sh：{url}\n\n## 说明\n\n安装时会从上游解析 SKILL.md 正文并写入本地技能目录。\n"
        ),
        category: SkillCategory::Dev,
        author: clean_text(source, 40),
        installs,
        stars: 0,
        glyph: "技".into(),
        installed: false,
        kind: SkillKind::Text,
        mcp: None,
    };
    skill.summary = if installs > 0 {
        format!("skills.sh · {}（累计安装 {}）", source, skill.installs_label())
    } else {
        format!("skills.sh · {source}")
    };
    MarketEntry {
        skill,
        remote: Some(RemoteSkillRef {
            source: source.to_string(),
            id: id.to_string(),
        }),
        remote_only: false,
        tags,
        icon_url: github_owner(source).map(|o| github_avatar(&o)),
        source_url: Some(url),
    }
}

fn skill_page_url(source: &str, id: &str) -> String {
    format!("{SKILLS_SH}/{source}/{id}")
}

/// 取 HTML 里 `<tag …>` 与 `</tag>` 之间的文本（去标签、解码常见实体）。
fn html_between(html: &str, open: &str, close: &str) -> Option<String> {
    let start = html.find(open)?;
    let gt = html[start..].find('>')? + start + 1;
    let end = html[gt..].find(close)? + gt;
    Some(decode_html_entities(&strip_tags(&html[gt..end])))
}

/// `4.4M` / `12.3k` / `48,742` → 数字；解析不到记 0。
fn parse_installs_label(s: &str) -> u64 {
    let t = s.trim().replace(',', "");
    let (num, mult) = match t.chars().last() {
        Some('K') | Some('k') => (&t[..t.len() - 1], 1_000f64),
        Some('M') | Some('m') => (&t[..t.len() - 1], 1_000_000f64),
        Some('B') | Some('b') => (&t[..t.len() - 1], 1_000_000_000f64),
        _ => (t.as_str(), 1f64),
    };
    num.parse::<f64>()
        .ok()
        .map(|n| (n * mult) as u64)
        .unwrap_or(0)
}

/// 安装时解析真实 SKILL.md：GitHub 仓库走 jsDelivr/raw，站点源走 well-known 约定。
pub fn fetch_remote_skill_body(source: &str, id: &str) -> Result<String, String> {
    let agent = agent_with_timeout(25);
    let body = if is_github_source(source) {
        fetch_github_skill_md(&agent, source, id)?
    } else {
        let url = format!(
            "https://{}/.well-known/agent-skills/{}/SKILL.md",
            source.trim(),
            id.trim()
        );
        get_text(&agent, &url)?
    };
    let mut text = body.trim().to_string();
    if text.is_empty() {
        return Err("上游 SKILL.md 是空的".into());
    }
    if text.len() > SKILL_BODY_LIMIT {
        text = text.chars().take(SKILL_BODY_LIMIT).collect();
        text.push_str("\n\n…（正文过长已截断）");
    }
    Ok(text)
}

/// GitHub 仓库里的 SKILL.md。
///
/// 先用 jsDelivr 目录接口定位路径（各仓库布局千差万别），定位不到再按
/// 常见路径直试；分支依次 `main` / `master`。
fn fetch_github_skill_md(agent: &ureq::Agent, source: &str, id: &str) -> Result<String, String> {
    let mut last = String::new();
    for branch in ["main", "master"] {
        if let Ok(files) = list_repo_files(agent, source, branch) {
            if let Some(path) = pick_skill_path(&files, id) {
                match get_github_file(agent, source, branch, &path) {
                    Ok(t) => return Ok(t),
                    Err(e) => last = e,
                }
            }
        }
        for rel in [
            format!("skills/{id}/SKILL.md"),
            format!("{id}/SKILL.md"),
            format!(".claude/skills/{id}/SKILL.md"),
            format!(".agents/skills/{id}/SKILL.md"),
        ] {
            if let Ok(t) = get_github_file(agent, source, branch, &rel) {
                return Ok(t);
            }
        }
    }
    Err(if last.is_empty() {
        format!("在 {source} 里没找到 {id}/SKILL.md（仓库可能改名或技能已删除）")
    } else {
        last
    })
}

fn list_repo_files(agent: &ureq::Agent, source: &str, branch: &str) -> Result<Vec<String>, String> {
    let url = format!("{JSDELIVR_DATA}/{source}@{branch}?structure=flat");
    let v = get_json(agent, &url)?;
    let Some(files) = v.get("files").and_then(|x| x.as_array()) else {
        return Err("jsDelivr 返回的不是文件列表".into());
    };
    Ok(files
        .iter()
        .filter_map(|f| f.get("name").and_then(|x| x.as_str()))
        .map(|s| s.to_string())
        .collect())
}

fn get_github_file(
    agent: &ureq::Agent,
    source: &str,
    branch: &str,
    rel: &str,
) -> Result<String, String> {
    let rel = rel.trim_start_matches('/');
    let cdn = format!("{JSDELIVR_CDN}/{source}@{branch}/{rel}");
    if let Ok(t) = get_text(agent, &cdn) {
        if !t.trim().is_empty() {
            return Ok(t);
        }
    }
    let raw = format!("https://raw.githubusercontent.com/{source}/{branch}/{rel}");
    get_text(agent, &raw)
}

/// 从仓库文件列表里挑 `<…>/<id>/SKILL.md`：优先非隐藏目录、层级最浅的路径。
pub fn pick_skill_path(files: &[String], id: &str) -> Option<String> {
    let mut best: Option<(usize, usize, String)> = None; // (hidden, depth, path)
    for f in files {
        let path = f.trim_start_matches('/');
        if !path.to_ascii_lowercase().ends_with("/skill.md") {
            continue;
        }
        let mut segs: Vec<&str> = path.split('/').collect();
        let Some(file) = segs.pop() else { continue };
        let Some(dir) = segs.last() else { continue };
        if !dir.eq_ignore_ascii_case(id) {
            continue;
        }
        let hidden = segs.iter().any(|s| s.starts_with('.')).into();
        let depth = segs.len();
        let cand = (hidden, depth, format!("{}/{}", segs.join("/"), file));
        if best
            .as_ref()
            .map(|(h, d, _)| (hidden, depth) < (*h, *d))
            .unwrap_or(true)
        {
            best = Some(cand);
        }
    }
    best.map(|(_, _, p)| p)
}

/// source 是不是 GitHub 仓库（恰好 `owner/repo`，owner 不含点）。
pub fn is_github_source(source: &str) -> bool {
    let segs: Vec<&str> = source.trim().split('/').collect();
    segs.len() == 2 && segs.iter().all(|s| !s.is_empty()) && !segs[0].contains('.')
}

fn github_owner(source: &str) -> Option<String> {
    if !is_github_source(source) {
        return None;
    }
    source.trim().split('/').next().map(|s| s.to_string())
}

/// 域名或 `owner/repo` 形式的来源：1~2 段、只含 `[A-Za-z0-9._-]`。
fn valid_remote_source(source: &str) -> bool {
    let segs: Vec<&str> = source.trim().split('/').collect();
    (1..=2).contains(&segs.len()) && segs.iter().all(|s| valid_segment(s))
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// 百分号编码（查询参数用）：只保留 RFC 3986 unreserved 字符。
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ============ 文本工具 ============

/// 去掉首尾空白并按字符截断（中文安全）。
fn clean_text(s: &str, max: usize) -> String {
    let t = s.trim().replace(['\r', '\n'], " ");
    let mut out: String = t.chars().take(max).collect();
    if t.chars().count() > max {
        out.push('…');
    }
    out
}

/// 通用 id 消毒：小写字母数字保留，其余折成 `-`。
pub fn slug_id(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "item".into()
    } else {
        out
    }
}

/// 从 `[` 开始的 JSON 数组（做括号配对，字符串里的括号不算）。
fn extract_json_array(text: &str) -> Option<String> {
    extract_balanced(text, '[', ']')
}

/// 从 `{` 开始的 JSON 对象。
fn extract_json_object(text: &str) -> Option<String> {
    extract_balanced(text, '{', '}')
}

fn extract_balanced(text: &str, open: char, close: char) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.first().copied() != Some(open as u8) {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, b) in bytes.iter().enumerate() {
        if in_str {
            if escaped {
                escaped = false;
            } else if *b == b'\\' {
                escaped = true;
            } else if *b == b'"' {
                in_str = false;
            }
            continue;
        }
        match *b {
            b'"' => in_str = true,
            x if x == open as u8 => depth += 1,
            x if x == close as u8 => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// 反转义 JS 字符串里的 JSON（`\"` → `"`，`\uXXXX` → 字符）。
fn unescape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let hex: String = it.by_ref().take(4).collect();
                if let Ok(v) = u32::from_str_radix(&hex, 16) {
                    if let Some(ch) = char::from_u32(v) {
                        out.push(ch);
                    }
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// 去掉 HTML 标签，保留文本（用于腾讯云详情页的高亮代码块）。
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// 常见 HTML 实体解码（高亮代码块里的引号/尖括号）。
fn decode_html_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

// ============ 测试 ============

#[cfg(test)]
mod tests {
    use super::*;

    fn official_fixture() -> serde_json::Value {
        serde_json::json!({
            "servers": [
                {
                    "server": {
                        "name": "io.github.example/weather",
                        "title": "Weather MCP",
                        "description": "查询天气",
                        "repository": {"url": "https://github.com/example/weather"},
                        "packages": [{
                            "registryType": "npm",
                            "identifier": "@example/weather-mcp",
                            "runtimeHint": "npx",
                            "runtimeArguments": [{"value": "-y"}],
                            "environmentVariables": [
                                {"name": "WEATHER_KEY", "isRequired": true},
                                {"name": "DEBUG", "isRequired": false}
                            ]
                        }]
                    }
                },
                {
                    "server": {
                        "name": "com.example/remote-only",
                        "title": "Remote Only",
                        "description": "只有远程端点",
                        "remotes": [{"type": "streamable-http", "url": "https://x.example/mcp"}],
                        "packages": []
                    }
                },
                {
                    "server": {
                        "name": "com.example/py",
                        "title": "Py Server",
                        "packages": [{
                            "registryType": "pypi",
                            "identifier": "mcp-server-time",
                            "runtimeHint": "uvx"
                        }]
                    }
                }
            ]
        })
    }

    #[test]
    fn official_parses_npm_pypi_and_remote_only() {
        let entries = parse_official(&official_fixture());
        assert_eq!(entries.len(), 3);

        let npm = &entries[0];
        assert_eq!(npm.skill.id, "mcp.io-github-example-weather");
        assert_eq!(npm.skill.name, "Weather MCP");
        assert_eq!(npm.skill.author, "example");
        assert!(!npm.remote_only);
        let cfg = npm.skill.mcp.as_ref().expect("npm 包应有启动配置");
        assert_eq!(cfg.command, "npx");
        assert_eq!(cfg.args, vec!["-y", "@example/weather-mcp"]);
        assert!(npm.skill.summary.contains("WEATHER_KEY"));
        assert!(!npm.skill.summary.contains("DEBUG"), "非必填变量不该提示");
        assert_eq!(npm.tags, vec!["npm"]);
        assert_eq!(
            npm.icon_url.as_deref(),
            Some("https://github.com/example.png"),
            "有 GitHub 仓库就取 owner 头像（走 302 拿真实头像）"
        );
        assert_eq!(
            npm.source_url.as_deref(),
            Some("https://github.com/example/weather")
        );

        let remote = &entries[1];
        assert!(remote.remote_only);
        assert!(remote.skill.mcp.is_none());
        assert_eq!(remote.tags, vec!["仅远程"]);
        assert!(remote.icon_url.is_none(), "没有仓库就没有图标");
        assert!(remote
            .source_url
            .as_deref()
            .unwrap()
            .starts_with(OFFICIAL_LIST), "没仓库时回落到 registry 搜索");

        let py = &entries[2];
        let cfg = py.skill.mcp.as_ref().unwrap();
        assert_eq!(cfg.command, "uvx");
        assert_eq!(cfg.args, vec!["mcp-server-time"]);
        assert_eq!(py.tags, vec!["PyPI"]);
    }

    #[test]
    fn official_dedups_server_names() {
        let mut v = official_fixture();
        let dup = v["servers"][0].clone();
        v["servers"].as_array_mut().unwrap().push(dup);
        assert_eq!(parse_official(&v).len(), 3);
    }

    // 真实页面里这段 JSON 是合法的（无转义），直接从 HTML 里截取。
    const TENCENT_HTML: &str = r#"<html><script>{"mcp":{"list":[{"abstract":"云开发的一站式后端","categoryList":[{"categoryId":100,"name":"腾讯产品MCP"}],"iconUrl":"https://developer.qcloudimg.com/x/icon.png","isHosted":0,"mcpId":11698,"mcpName":"CloudBase-AI-ToolKit","readNum":35176,"srcAuthor":"TencentCloudBase","title":"云开发MCP"},{"abstract":"托管的浏览器工具","categoryList":[{"categoryId":101,"name":"开发者工具"}],"iconUrl":"","isHosted":1,"mcpId":10011,"mcpName":"edgeone-pages-mcp","readNum":31911,"srcAuthor":"TencentEdgeOne","title":"EdgeOne Pages"}]}}</script></html>"#;

    #[test]
    fn tencent_list_parses_escaped_payload() {
        // 上游改版成 JS 字符串形式（\" 转义）时也要能解析
        let escaped = TENCENT_HTML.replace('"', "\\\"");
        let entries = parse_tencent_list(&escaped);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].skill.id, "tencent.mcp.11698");
        assert_eq!(entries[0].skill.name, "云开发MCP");
    }

    #[test]
    fn tencent_list_parses_entries() {
        let entries = parse_tencent_list(TENCENT_HTML);
        assert_eq!(entries.len(), 2);
        let first = &entries[0];
        assert_eq!(first.skill.id, "tencent.mcp.11698");
        assert_eq!(first.skill.name, "云开发MCP");
        assert_eq!(first.skill.author, "TencentCloudBase");
        assert_eq!(first.skill.installs, 35176);
        assert!(!first.remote_only);
        assert!(first.skill.mcp.is_none(), "详情配置要等安装时再抓");
        assert_eq!(first.tags, vec!["腾讯产品MCP"], "标签直接用上游分类名");
        assert_eq!(entries[1].tags, vec!["开发者工具"]);
        assert!(entries[1].remote_only, "托管版应标注不可安装");
        assert_eq!(
            first.icon_url.as_deref(),
            Some("https://developer.qcloudimg.com/x/icon.png")
        );
        assert_eq!(
            first.source_url.as_deref(),
            Some("https://cloud.tencent.com/developer/mcp/server/11698")
        );
        assert!(entries[1].icon_url.is_none(), "空 iconUrl 按没有处理");
    }

    const DETAIL_HTML: &str = r#"<details><summary>使用uvx</summary><pre><code class="hljs"><span class="hljs-punctuation">{</span>
  <span class="hljs-attr">&quot;mcpServers&quot;</span><span class="hljs-punctuation">:</span> <span class="hljs-punctuation">{</span>
    <span class="hljs-attr">&quot;time&quot;</span><span class="hljs-punctuation">:</span> <span class="hljs-punctuation">{</span>
      <span class="hljs-attr">&quot;command&quot;</span><span class="hljs-punctuation">:</span> <span class="hljs-string">&quot;uvx&quot;</span><span class="hljs-punctuation">,</span>
      <span class="hljs-attr">&quot;args&quot;</span><span class="hljs-punctuation">:</span> <span class="hljs-punctuation">[</span><span class="hljs-string">&quot;mcp-server-time&quot;</span><span class="hljs-punctuation">]</span>
    <span class="hljs-punctuation">}</span>
  <span class="hljs-punctuation">}</span>
<span class="hljs-punctuation">}</span>
</code></pre></details>"#;

    #[test]
    fn tencent_detail_extracts_command() {
        let cfg = parse_tencent_detail(DETAIL_HTML).unwrap();
        assert_eq!(cfg.command, "uvx");
        assert_eq!(cfg.args, vec!["mcp-server-time"]);
        assert_eq!(cfg.name, "time");
    }

    #[test]
    fn tencent_detail_without_stdio_is_error() {
        let html = "<p>该服务器仅提供 SSE 端点</p>";
        assert!(parse_tencent_detail(html).is_err());
    }

    // 首页 SSR 表格行的最小结构（真实页面每行还带很多 class，这里只留解析依赖的部分）。
    const SKILLS_SH_HOME_HTML: &str = r#"<a class="group grid grid-cols-[auto_1fr_auto]" href="/vercel-labs/skills/find-skills"><div><h3 class="font-medium">find-skills</h3><p class="text-sm text-muted-foreground">vercel-labs/skills</p></div><span class="font-mono text-sm text-foreground">4.4M</span></a><a href="/github/awesome-copilot/git-commit"><h3>git-commit</h3><p>github/awesome-copilot</p><span class="font-mono text-sm text-foreground">48,742</span></a><a href="/trending">Trending</a>"#;

    #[test]
    fn skills_sh_home_parses_rows() {
        let entries = parse_skills_sh_home(SKILLS_SH_HOME_HTML);
        assert_eq!(entries.len(), 2, "非技能链接（/trending）应被跳过");
        let first = &entries[0];
        assert_eq!(first.skill.id, "ss.vercel-labs-skills.find-skills");
        assert_eq!(first.skill.name, "find-skills");
        assert_eq!(first.skill.author, "vercel-labs/skills");
        assert_eq!(first.skill.installs, 4_400_000, "4.4M 要还原成数字");
        assert_eq!(first.skill.kind, SkillKind::Text);
        assert!(!first.remote_only, "skills.sh 条目安装时解析正文，不是仅远程");
        let remote = first.remote.as_ref().expect("要记住上游身份才能装正文");
        assert_eq!(remote.source, "vercel-labs/skills");
        assert_eq!(remote.id, "find-skills");
        assert!(first.tags.is_empty(), "仓库名不当标签（避免筛选条爆炸）");
        assert_eq!(
            first.source_url.as_deref(),
            Some("https://www.skills.sh/vercel-labs/skills/find-skills")
        );
        assert_eq!(entries[1].skill.installs, 48_742);
    }

    #[test]
    fn skills_sh_search_parses_dedups_and_skips_invalid() {
        let v = serde_json::json!({
            "skills": [
                {"id":"github/awesome-copilot/git-commit","source":"github/awesome-copilot","skillId":"git-commit","name":"git-commit","installs":48742},
                {"id":"github/awesome-copilot/git-commit","source":"github/awesome-copilot","skillId":"git-commit","name":"git-commit","installs":48742},
                {"source":"open.feishu.cn","skillId":"lark-doc","name":"Lark Doc","installs":10,"isOfficial":true},
                {"source":"..","skillId":"x"},
                {"source":"a/b/c","skillId":"x"},
                {"source":"github/foo","skillId":"bad id!"}
            ]
        });
        let entries = parse_skills_sh_search(&v);
        assert_eq!(entries.len(), 2, "去重后只剩 2 条合法条目");
        assert_eq!(entries[0].skill.id, "ss.github-awesome-copilot.git-commit");
        assert_eq!(entries[0].skill.installs, 48_742);
        assert!(entries[0].tags.is_empty());
        assert_eq!(entries[1].skill.author, "open.feishu.cn");
        assert!(entries[1].tags.contains(&"官方".to_string()));
        assert!(entries[1].icon_url.is_none(), "站点源没有 GitHub 头像");
    }

    #[test]
    fn pick_skill_path_prefers_shallow_non_hidden() {
        let files = vec![
            "/.claude/skills/git-commit/SKILL.md".to_string(),
            "/skills/git-commit/SKILL.md".to_string(),
            "/other/SKILL.md".to_string(),
        ];
        assert_eq!(
            pick_skill_path(&files, "git-commit").as_deref(),
            Some("skills/git-commit/SKILL.md")
        );
        let root = vec!["/git-commit/SKILL.md".to_string()];
        assert_eq!(
            pick_skill_path(&root, "git-commit").as_deref(),
            Some("git-commit/SKILL.md"),
            "根级目录也要认"
        );
        let hidden_only = vec!["/.claude/skills/git-commit/SKILL.md".to_string()];
        assert_eq!(
            pick_skill_path(&hidden_only, "git-commit").as_deref(),
            Some(".claude/skills/git-commit/SKILL.md"),
            "只有隐藏目录时也要回退到它"
        );
        assert!(pick_skill_path(&files, "missing").is_none());
    }

    #[test]
    fn github_source_detection() {
        assert!(is_github_source("github/awesome-copilot"));
        assert!(is_github_source("obra/superpowers"));
        assert!(!is_github_source("open.feishu.cn"));
        assert!(!is_github_source("a/b/c"));
        assert!(!is_github_source("../../evil"));
    }

    #[test]
    fn urlencode_keeps_unreserved() {
        assert_eq!(urlencode("git flow"), "git%20flow");
        assert_eq!(urlencode("中文"), "%E4%B8%AD%E6%96%87");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn install_labels_parse() {
        assert_eq!(parse_installs_label("4.4M"), 4_400_000);
        assert_eq!(parse_installs_label("12.3k"), 12_300);
        assert_eq!(parse_installs_label("48,742"), 48_742);
        assert_eq!(parse_installs_label("n/a"), 0);
    }

    #[test]
    fn cache_roundtrip_and_ago() {
        let dir = std::env::temp_dir().join(format!("nw-market-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("marketplace.json");

        let mut cache = MarketCache::default();
        let entries = parse_official(&official_fixture());
        cache.put(MarketSource::Official, entries.clone(), 1_700_000_000);
        save_cache_to(&cache, &path).unwrap();

        let back = load_cache_from(&path);
        let snap = back.snapshot(MarketSource::Official).unwrap();
        assert_eq!(snap.entries.len(), 3);
        assert_eq!(snap.entries[0].skill.id, "mcp.io-github-example-weather");
        assert_eq!(snap.fetched_at, 1_700_000_000);
        assert!(back.snapshot(MarketSource::SkillsSh).is_none());

        assert_eq!(ago_label(neverwork_core::config::now_secs()), "刚刚");
        assert!(ago_label(1).contains("天前"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_without_tags_field_still_loads() {
        // 旧版本写的 marketplace.json 没有 tags 字段，不能因此整库丢失
        let mut v = serde_json::to_value(parse_official(&official_fixture())).unwrap();
        for e in v.as_array_mut().unwrap() {
            let obj = e.as_object_mut().unwrap();
            obj.remove("tags");
            obj.remove("icon_url");
            obj.remove("source_url");
        }
        let entries: Vec<MarketEntry> = serde_json::from_value(v).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries[0].tags.is_empty());
        assert!(entries[0].icon_url.is_none());
        assert!(entries[0].source_url.is_none());
    }

    #[test]
    fn image_sniffing_accepts_real_headers_only() {
        assert!(looks_like_image(&[0x89, b'P', b'N', b'G', 0x0d]));
        assert!(looks_like_image(&[0xff, 0xd8, 0xff, 0xe0]));
        assert!(looks_like_image(b"GIF89a..."));
        assert!(looks_like_image(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(looks_like_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"));
        assert!(looks_like_image(b"  <?xml version=\"1.0\"?><svg></svg>"));
        assert!(!looks_like_image(b"<!DOCTYPE html><html></html>"));
        assert!(!looks_like_image(b"not an image"));
        assert!(!looks_like_image(b"\x89PN"));
    }

    #[test]
    fn icon_cache_path_is_stable_and_distinct() {
        let a = icon_path_for("https://avatars.githubusercontent.com/anthropics");
        let b = icon_path_for("https://avatar.githubusercontent.com/anthropics");
        assert_eq!(
            a,
            icon_path_for("https://avatars.githubusercontent.com/anthropics"),
            "同一 URL 必须映射到同一文件"
        );
        assert_ne!(a, b);
        assert_eq!(a.extension().and_then(|e| e.to_str()), Some("img"));
    }

    #[test]
    fn extract_handles_braces_inside_strings() {
        let s = r#"[{"a":"}{"} , {"b":2}]"#;
        assert_eq!(extract_json_array(s).unwrap(), s);
    }

    #[test]
    fn slug_id_is_stable_and_safe() {
        assert_eq!(slug_id("io.github.keep/api"), "io-github-keep-api");
        assert_eq!(slug_id("../../evil"), "evil");
        assert_eq!(slug_id(""), "item");
    }

    /// 真实网络冒烟：`cargo test -p neverwork skills_sh_live -- --ignored --nocapture`
    #[test]
    #[ignore = "manual: hits the real network"]
    fn skills_sh_live_fetch_smoke() {
        let t = std::time::Instant::now();
        let home = fetch(MarketSource::SkillsSh).expect("skills.sh 首页应能解析出热榜");
        eprintln!("skills.sh home {} entries in {:?}", home.len(), t.elapsed());
        assert!(home.len() >= 20);
        assert!(home.iter().all(|e| e.skill.id.starts_with("ss.")));
        assert!(home.iter().all(|e| e.remote.is_some()));

        let found = search_skills_sh("git").expect("搜索接口应有结果");
        eprintln!("skills.sh search git: {} entries", found.len());
        assert!(found
            .iter()
            .any(|e| e.remote.as_ref().is_some_and(|r| r.id == "git-commit")));

        let body = fetch_remote_skill_body("github/awesome-copilot", "git-commit")
            .expect("GitHub 源应能定位并下载 SKILL.md");
        eprintln!("github body {} bytes", body.len());
        assert!(body.to_lowercase().contains("git"));

        let site = fetch_remote_skill_body("open.feishu.cn", "lark-doc")
            .expect("站点源应走 well-known 约定");
        eprintln!("feishu body {} bytes", site.len());
        assert!(!site.trim().is_empty());
    }

    /// 腾讯云详情页冒烟（11698 = 云开发MCP，启动命令为 npx）。
    #[test]
    #[ignore = "manual: hits the real network"]
    fn tencent_detail_live_smoke() {
        let cfg = fetch_tencent_detail(11698).expect("详情页应能解析出 stdio 启动配置");
        eprintln!("TENCENT detail: {cfg:?}");
        assert!(!cfg.command.trim().is_empty());
        assert!(!cfg.name.is_empty());
    }
}
