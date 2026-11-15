//! 本地检索（RAG）：分块 → BM25 → 重排 → 带引用返回。
//!
//! 为什么不用 embedding：
//! - 要调外部服务或跑本地模型，离线不可用、首字延迟高、还要维护向量存储；
//! - 代码检索的查询通常就是标识符或短语，词项匹配已经很准；
//! - 全流程纯函数，可单测，不会因为模型换版本而结果漂移。
//!
//! 为什么每次调用都重建索引：工作区在 Agent 手里随时会变，缓存要么失效
//! 不及时（搜到已删除的文件），要么要写一堆 mtime 失效逻辑。代码库规模
//! 受 `MAX_FILES` / `MAX_TOTAL_BYTES` 约束，扫一遍是毫秒级。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 每块行数。太大检索粒度粗、太小会把函数切成两半。
pub const CHUNK_LINES: usize = 60;
/// 相邻块重叠行数，避免答案正好落在边界被切断。
pub const CHUNK_OVERLAP: usize = 10;
/// 建索引的文件数上限。
pub const MAX_FILES: usize = 2_000;
/// 单文件字节上限（超过的多半是生成物或数据文件）。
pub const MAX_FILE_BYTES: usize = 256 * 1024;
/// 索引总字节上限，防止一次检索把 8G 的 target 拖进来。
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// 默认返回块数。
pub const DEFAULT_TOP_K: usize = 8;
/// 返回块数上限（模型传再大也不给）。
pub const MAX_TOP_K: usize = 20;
/// 单块返回给模型的字符上限。
pub const MAX_CHUNK_CHARS: usize = 2_000;
/// 单行字符上限：超过就当作数据文件整文件跳过。
///
/// minified JSON、单行日志、打包产物都是「几十 KB 一行」，代码几乎不可能。
/// 这类文件往往整篇都是查询词（如会话库里存着用户原话），BM25 会给出
/// 极高分数，把真正的实现排到后面——真实踩过：`search_code` 的第一名
/// 命中的是工作区里残留的会话 JSON，而它恰巧包含了完整的问题原文。
pub const MAX_LINE_CHARS: usize = 2_000;

/// 建索引时跳过的目录（与 `search_files` 保持一致，另加 vendor）。
///
/// 后半段是各类工具链的缓存与产物目录：不是源码，却常常
/// 含大量与查询重复的字串（覆盖率报告、依赖清单、日志），
/// 让它们入库只会把真代码挤下去。
const SKIP_DIRS: [&str; 15] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    ".next",
    "__pycache__",
    "vendor",
    "build",
    ".venv",
    "venv",
    "coverage",
    ".idea",
    ".vscode",
    ".cache",
    "logs",
];

/// 一个文本块。
#[derive(Debug, Clone)]
pub struct Chunk {
    /// 相对工作区根的路径，斜杠分隔。
    pub path: String,
    /// 起始行号（1 基，含）。
    pub start: usize,
    /// 结束行号（1 基，含）。
    pub end: usize,
    pub text: String,
    /// 词项（保留重复，BM25 要用词频）。
    pub terms: Vec<String>,
}

/// 一条检索结果。
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub score: f32,
    /// 已按 `MAX_CHUNK_CHARS` 裁剪过的正文。
    pub text: String,
    /// 是否被裁剪过（调用方要在输出里说明）。
    pub truncated: bool,
}

/// 把标识符拆成可检索的词项。
///
/// 为什么要拆：模型搜 `history` 时，文件里写的是 `history_for_model`；
/// 只看整词会漏掉，而这类「按片段找定义」正是代码检索最常见的用法。
fn push_ident(out: &mut Vec<String>, word: &str) {
    if word.is_empty() {
        return;
    }
    let lower = word.to_lowercase();
    // 单字符（x、i、a）噪声太大，只留长度 >= 2 的
    if lower.chars().count() >= 2 {
        out.push(lower.clone());
    }
    // snake_case / kebab-case
    for seg in word.split(['_', '-']) {
        let seg = seg.to_lowercase();
        if seg.chars().count() >= 2 && seg != lower {
            out.push(seg);
        }
    }
    // camelCase / PascalCase：在小写→大写、字母→数字的边界切
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in word.chars() {
        let is_upper = c.is_uppercase();
        let boundary = (is_upper && prev_lower) || c.is_ascii_digit();
        if boundary && cur.chars().count() >= 2 {
            out.push(cur.to_lowercase());
            cur.clear();
        }
        if c.is_alphanumeric() {
            cur.push(c);
            prev_lower = c.is_lowercase();
        } else {
            cur.clear();
            prev_lower = false;
        }
    }
    if cur.chars().count() >= 2 && cur.to_lowercase() != lower {
        out.push(cur.to_lowercase());
    }
}

/// 分词：ASCII 按标识符（含拆分），CJK 按单字 + 相邻双字（bigram）。
///
/// 没有中文分词词典：bigram 足够覆盖「上下文预算」「断点续跑」这类
/// 技术短语，且不需要维护词表。
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut prev_cjk: Option<char> = None;

    let flush = |word: &mut String, out: &mut Vec<String>| {
        if !word.is_empty() {
            push_ident(out, word);
            word.clear();
        }
    };

    for c in text.chars() {
        if is_cjk(c) {
            flush(&mut word, &mut out);
            // 单字：短查询（「树」）也能命中
            out.push(c.to_string());
            if let Some(p) = prev_cjk {
                out.push(format!("{p}{c}"));
            }
            prev_cjk = Some(c);
        } else if c.is_alphanumeric() || c == '_' {
            prev_cjk = None;
            word.push(c);
        } else {
            prev_cjk = None;
            flush(&mut word, &mut out);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF   // 基本汉字
        | 0x3400..=0x4DBF // 扩展 A
        | 0x3040..=0x30FF // 日文假名（注释里会出现）
        | 0xAC00..=0xD7AF // 韩文
    )
}

/// 把文本按行切块，带重叠。
pub fn chunk_text(path: &str, text: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let step = CHUNK_LINES.saturating_sub(CHUNK_OVERLAP).max(1);
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let body = lines[start..end].join("\n");
        out.push(Chunk {
            path: path.to_string(),
            start: start + 1,
            end,
            terms: tokenize(&body),
            text: body,
        });
        if end >= lines.len() {
            break;
        }
        start += step;
    }
    out
}

/// 扫描工作区建索引。递归、跳过二进制与生成目录，受上限约束。
pub fn index_workspace(root: &Path) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut files = 0usize;
    let mut bytes = 0usize;
    walk(root, root, &mut chunks, &mut files, &mut bytes);
    chunks
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<Chunk>, files: &mut usize, bytes: &mut usize) {
    if *files >= MAX_FILES || *bytes >= MAX_TOTAL_BYTES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if *files >= MAX_FILES || *bytes >= MAX_TOTAL_BYTES {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            walk(&p, root, out, files, bytes);
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        if meta.len() > MAX_FILE_BYTES as u64 {
            continue;
        }
        let Ok(data) = std::fs::read(&p) else { continue };
        // 含 NUL 的是二进制（图片、可执行文件、打包产物）
        if data.iter().take(4096).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&data);
        // 单行超长的多半是 minified 数据：整文件命中会让分数虚高
        if text.lines().any(|l| l.len() > MAX_LINE_CHARS) {
            continue;
        }
        *files += 1;
        *bytes += data.len();
        let rel = rel_path(&p, root);
        out.extend(chunk_text(&rel, &text));
    }
}

fn rel_path(p: &Path, root: &Path) -> String {
    p.strip_prefix(root)
        .map(|x| x.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        })
}

/// 检索。
///
/// `top_k` 会被夹到 `[1, MAX_TOP_K]`；查询被拆词后为空（纯符号/空白）
/// 时返回空结果，而不是把全库倒出来。
pub fn search(root: &Path, query: &str, top_k: usize) -> Vec<SearchHit> {
    let q_unique = unique_terms(query);
    if q_unique.is_empty() {
        return Vec::new();
    }
    let chunks = index_workspace(root);
    rank(chunks, &q_unique, top_k.clamp(1, MAX_TOP_K))
}

/// 对已建好的块打分排序（与 IO 解耦，方便单测）。
pub fn rank(chunks: Vec<Chunk>, q_unique: &[String], top_k: usize) -> Vec<SearchHit> {
    if chunks.is_empty() || q_unique.is_empty() {
        return Vec::new();
    }
    let n = chunks.len() as f32;
    let avgdl = chunks.iter().map(|c| c.terms.len()).max().unwrap_or(1) as f32;

    // 文档频率
    let mut df: HashMap<&str, usize> = HashMap::new();
    for c in &chunks {
        let mut seen: HashMap<&str, ()> = HashMap::new();
        for t in &c.terms {
            if seen.insert(t.as_str(), ()).is_none() {
                *df.entry(t.as_str()).or_insert(0) += 1;
            }
        }
    }

    let k1 = 1.2_f32;
    let b = 0.75_f32;
    let mut scored: Vec<SearchHit> = Vec::new();

    for c in &chunks {
        let mut tf: HashMap<&str, usize> = HashMap::new();
        for t in &c.terms {
            *tf.entry(t.as_str()).or_insert(0) += 1;
        }
        let dl = c.terms.len() as f32;
        let mut score = 0.0_f32;
        let mut matched = 0usize;
        for q in q_unique {
            let Some(&f) = tf.get(q.as_str()) else { continue };
            matched += 1;
            let dfi = *df.get(q.as_str()).unwrap_or(&0) as f32;
            // 概率式 IDF（+1 避免负分）
            let idf = ((n - dfi + 0.5) / (dfi + 0.5) + 1.0).ln();
            // avgdl 用最长块近似，避免 dl/avgdl 被超长块拉爆
            let denom = f as f32 + k1 * (1.0 - b + b * dl / avgdl.max(1.0));
            score += idf * (f as f32 * (k1 + 1.0)) / denom;
        }
        if score <= 0.0 {
            continue;
        }
        // 覆盖率：命中「不同查询词」越多越可信，防止某个高频词刷分
        let coverage = matched as f32 / q_unique.len() as f32;
        score *= 0.5 + 0.5 * coverage;

        // 路径命中：查 `state` 时 `state.rs` 的块应排在无关文件前面
        let path_low = c.path.to_lowercase();
        if q_unique.iter().any(|q| path_low.contains(q.as_str())) {
            score *= 1.6;
        }
        // 块前 5 行命中：多半是定义处，比中间的引用更有用
        let head: String = c.text.lines().take(5).collect::<Vec<_>>().join("\n");
        let head_low = head.to_lowercase();
        if q_unique.iter().any(|q| head_low.contains(q.as_str())) {
            score *= 1.2;
        }

        let (text, truncated) = clip(&c.text);
        scored.push(SearchHit {
            path: c.path.clone(),
            start: c.start,
            end: c.end,
            score,
            text,
            truncated,
        });
    }

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            // 同分按路径稳定排序，结果可复现
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.start.cmp(&b.start))
    });
    scored.truncate(top_k.clamp(1, MAX_TOP_K));
    scored
}

/// 按字符裁剪（中文安全），超出时保留开头并标注。
fn clip(text: &str) -> (String, bool) {
    if text.chars().count() <= MAX_CHUNK_CHARS {
        return (text.to_string(), false);
    }
    let head: String = text.chars().take(MAX_CHUNK_CHARS).collect();
    (format!("{head}\n…（本块已截断，需要全文请用 read_file）"), true)
}

/// 把结果渲染成给模型看的文本（带可引用的行列区间）。
pub fn render(query: &str, hits: &[SearchHit], indexed: usize) -> String {
    if hits.is_empty() {
        return format!(
            "检索「{query}」：没有匹配。可以换个更短的词，或先用 list_dir / search_files 定位。"
        );
    }
    let mut out = format!(
        "检索「{query}」：命中 {} 段（索引 {indexed} 块，按相关度排序）\n",
        hits.len()
    );
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!(
            "\n{}. {}:{}-{}  score {:.2}\n",
            i + 1,
            h.path,
            h.start,
            h.end,
            h.score
        ));
        for (n, line) in h.text.lines().enumerate() {
            out.push_str(&format!("{:>5} | {line}\n", h.start + n));
        }
    }
    out.push_str("\n（以上为检索片段，完整内容用 read_file 带 offset/limit 读取）\n");
    out
}

/// 供工具层调用的入口：建一次索引，同时用于检索与报告块数。
pub fn search_and_render(root: &Path, query: &str, top_k: usize) -> String {
    let q_unique = unique_terms(query);
    let chunks = index_workspace(root);
    let indexed = chunks.len();
    let hits = rank(chunks, &q_unique, top_k.clamp(1, MAX_TOP_K));
    render(query, &hits, indexed)
}

/// 查询拆词后去重（保留首次出现的顺序）。
fn unique_terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tokenize(query) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// 只读它的块数上限，给工具层提示用。
pub fn index_cap() -> (usize, usize) {
    (MAX_FILES, MAX_TOTAL_BYTES)
}

/// 路径是否位于跳过目录内（测试与调用方共用同一份判定）。
pub fn is_skipped(rel: &str) -> bool {
    rel.split('/').any(|seg| SKIP_DIRS.contains(&seg))
}

/// 把相对路径解析到 root 下（只读场景，不跟随符号链接逃逸）。
/// 检索工具本身不接收路径参数，这个函数只给测试与未来扩展用。
pub fn within(root: &Path, rel: &str) -> Option<PathBuf> {
    let p = root.join(rel);
    let norm = p.canonicalize().ok()?;
    let root_norm = root.canonicalize().ok()?;
    norm.starts_with(&root_norm).then_some(norm)
}
