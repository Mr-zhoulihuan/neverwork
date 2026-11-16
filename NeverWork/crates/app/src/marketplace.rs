//! 真实技能市场：数据全部来自公开源，不再有硬编码的虚构条目。
//!
//! 三个来源：
//! - `Official`：MCP 官方 Registry（`registry.modelcontextprotocol.io`），
//!   条目自带 npm / PyPI / OCI 包信息，可直接转成 stdio 启动配置。
//! - `Tencent` ：腾讯云 MCP 广场（国内直达）。列表页 SSR 里嵌着 30 条真实
//!   条目的 JSON；安装时再抓详情页，从页面里的 `{"mcpServers":{...}}` 取
//!   真实启动命令。
//! - `GitHub`  ：真实技能仓库（anthropics/skills、vercel-labs/agent-skills）
//!   里 `skills/<name>/SKILL.md` 的全文；正文缓存在条目里，安装即落盘。
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
/// 真实技能仓库：owner / repo / 分支。只收录已验证存在 `skills/` 目录的仓库。
pub const GITHUB_REPOS: &[(&str, &str, &str)] = &[
    ("anthropics", "skills", "main"),
    ("vercel-labs", "agent-skills", "main"),
];
/// 每个仓库最多收录多少个技能（并行下载正文，太多会拖慢首次刷新）。
const GITHUB_MAX_PER_REPO: usize = 24;
/// 单个仓库内并行下载 SKILL.md 的线程数（TLS 握手是主要耗时，并发能线性提速）。
const GITHUB_CONCURRENCY: usize = 12;
/// 正文缓存上限：超过就截断（只用于安装初始正文，避免缓存文件过大）。
const GITHUB_BODY_LIMIT: usize = 60_000;
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
    #[serde(rename = "github")]
    GitHub,
}

impl MarketSource {
    pub fn all() -> [MarketSource; 3] {
        [MarketSource::Official, MarketSource::Tencent, MarketSource::GitHub]
    }

    /// 缓存与状态键。
    pub fn key(&self) -> &'static str {
        match self {
            MarketSource::Official => "official",
            MarketSource::Tencent => "tencent",
            MarketSource::GitHub => "github",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            MarketSource::Official => "MCP 官方",
            MarketSource::Tencent => "腾讯云广场",
            MarketSource::GitHub => "GitHub 技能",
        }
    }

    /// 数据来自哪里（状态行展示）。
    pub fn origin(&self) -> &'static str {
        match self {
            MarketSource::Official => "registry.modelcontextprotocol.io",
            MarketSource::Tencent => "cloud.tencent.com",
            MarketSource::GitHub => "raw.githubusercontent.com（jsDelivr 加速）",
        }
    }

    /// 来源主色（卡片图标底色；只表示数据从哪来，不代表分类）。
    pub fn hue(&self) -> u32 {
        match self {
            MarketSource::Official => 0xa855f7,
            MarketSource::Tencent => 0x3b82f6,
            MarketSource::GitHub => 0x6b7280,
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "official" => Some(MarketSource::Official),
            "tencent" => Some(MarketSource::Tencent),
            "github" => Some(MarketSource::GitHub),
            _ => None,
        }
    }
}

// ============ 条目与缓存 ============

/// 市场条目：技能本体 + 下载来源标注。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketEntry {
    #[serde(flatten)]
    pub skill: Skill,
    /// GitHub 技能的仓库路径（`owner/repo/skills/<dir>`），重新下载时用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_path: Option<String>,
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
            github_path: None,
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
    } else if id.starts_with("gh.") {
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
        MarketSource::GitHub => fetch_github(),
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
            github_path: None,
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
            github_path: None,
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

// ============ GitHub 技能仓库 ============

fn fetch_github() -> Result<Vec<MarketEntry>, String> {
    let agent = agent_with_timeout(15);
    let mut out: Vec<MarketEntry> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for (owner, repo, branch) in GITHUB_REPOS {
        match fetch_github_repo(&agent, owner, repo, branch) {
            Ok(mut items) => out.append(&mut items),
            Err(e) => errors.push(format!("{owner}/{repo}：{e}")),
        }
    }
    if out.is_empty() {
        return Err(if errors.is_empty() {
            "GitHub 技能仓库没有返回条目".into()
        } else {
            errors.join("；")
        });
    }
    Ok(out)
}

fn fetch_github_repo(
    agent: &ureq::Agent,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<Vec<MarketEntry>, String> {
    let dirs = list_github_skill_dirs(agent, owner, repo)?;
    let dirs: Vec<String> = dirs.into_iter().take(GITHUB_MAX_PER_REPO).collect();
    if dirs.is_empty() {
        return Err("skills/ 目录下没有技能".into());
    }
    let mut out = Vec::new();
    let mut errs = Vec::new();
    // 并发下载，既快又不至于把匿名 CDN 打崩
    for chunk in dirs.chunks(GITHUB_CONCURRENCY) {
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for dir in chunk {
                let agent = agent.clone();
                let owner = owner.to_string();
                let repo = repo.to_string();
                let branch = branch.to_string();
                let dir = dir.clone();
                handles.push(scope.spawn(move || {
                    download_github_skill(&agent, &owner, &repo, &branch, &dir)
                }));
            }
            for (dir, h) in chunk.iter().zip(handles) {
                match h.join() {
                    Ok(Ok(entry)) => out.push(entry),
                    Ok(Err(e)) => errs.push(format!("{dir}：{e}")),
                    Err(_) => errs.push(format!("{dir}：下载线程异常退出")),
                }
            }
        });
    }
    if out.is_empty() {
        return Err(errs.join("；"));
    }
    Ok(out)
}

/// 列出仓库 `skills/` 下的子目录名（GitHub contents API）。
fn list_github_skill_dirs(
    agent: &ureq::Agent,
    owner: &str,
    repo: &str,
) -> Result<Vec<String>, String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/contents/skills");
    let v = get_json(agent, &url)?;
    let Some(items) = v.as_array() else {
        return Err("GitHub 返回的不是目录列表（仓库可能不存在或已改名）".into());
    };
    Ok(items
        .iter()
        .filter(|it| it.get("type").and_then(|x| x.as_str()) == Some("dir"))
        .filter_map(|it| it.get("name").and_then(|x| x.as_str()))
        .map(|s| s.to_string())
        .collect())
}

fn download_github_skill(
    agent: &ureq::Agent,
    owner: &str,
    repo: &str,
    branch: &str,
    dir: &str,
) -> Result<MarketEntry, String> {
    // jsDelivr 国内可达性好；失败再直连 raw
    let cdn = format!("https://cdn.jsdelivr.net/gh/{owner}/{repo}@{branch}/skills/{dir}/SKILL.md");
    let text = match get_text(agent, &cdn) {
        Ok(t) => t,
        Err(_) => {
            let raw =
                format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/skills/{dir}/SKILL.md");
            get_text(agent, &raw)?
        }
    };
    let mut skill = parse_github_skill(&text, owner, repo, dir);
    // 正文缓存截断，避免 marketplace.json 过大
    if skill.detail.len() > GITHUB_BODY_LIMIT {
        skill.detail = skill.detail.chars().take(GITHUB_BODY_LIMIT).collect();
        skill.detail.push_str("\n\n…（正文过长已截断）");
    }
    Ok(MarketEntry {
        github_path: Some(format!("{owner}/{repo}/skills/{dir}")),
        skill,
        remote_only: false,
        tags: vec![format!("{owner}/{repo}")],
        icon_url: Some(github_avatar(owner)),
        source_url: Some(format!(
            "https://github.com/{owner}/{repo}/tree/{branch}/skills/{dir}"
        )),
    })
}

/// SKILL.md → Skill。id 固定为 `gh.<owner>.<repo>.<dir>`，卸载/重装稳定。
pub fn parse_github_skill(text: &str, owner: &str, repo: &str, dir: &str) -> Skill {
    let mut s = crate::skills_ops::parse_md(text, dir);
    s.id = format!(
        "gh.{}.{}.{}",
        slug_id(owner),
        slug_id(repo),
        slug_id(dir)
    );
    s.author = format!("{owner}/{repo}");
    s.installed = false;
    s.summary = clean_text(&s.summary, 160);
    // 上游 SKILL.md 没有分类字段：分类只认 front-matter 里的声明，不猜
    if s.glyph.trim().is_empty() {
        s.glyph = "技".into();
    }
    s
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

    #[test]
    fn github_skill_md_becomes_installable_skill() {
        let md = "---\nname: Frontend Design\ndescription: 生成有品味的界面\nlicense: MIT\n---\n\n# Frontend Design\n\n先定信息层级。\n";
        let s = parse_github_skill(md, "anthropics", "skills", "frontend-design");
        assert_eq!(s.id, "gh.anthropics.skills.frontend-design");
        assert_eq!(s.name, "Frontend Design");
        assert_eq!(s.summary, "生成有品味的界面");
        assert_eq!(s.category, SkillCategory::Dev, "上游无分类声明时不猜");
        assert!(!s.installed, "市场条目不该预置已安装");
        assert!(s.detail.contains("先定信息层级"));
        assert_eq!(s.author, "anthropics/skills");
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
        assert!(back.snapshot(MarketSource::GitHub).is_none());

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

    /// 真实网络冒烟：`cargo test -p neverwork live_fetch -- --ignored --nocapture`
    #[test]
    #[ignore = "manual: hits the real network"]
    fn github_live_fetch_smoke() {
        let t = std::time::Instant::now();
        match fetch(MarketSource::GitHub) {
            Ok(v) => {
                eprintln!("GITHUB OK {} entries in {:?}", v.len(), t.elapsed());
                assert!(v.iter().all(|e| e.skill.id.starts_with("gh.")));
            }
            Err(e) => panic!("GITHUB failed in {:?}: {e}", t.elapsed()),
        }
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
