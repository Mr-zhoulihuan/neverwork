//! Agent 工具集：让模型真的能读写工作区文件。
//!
//! 安全模型（重要）：
//! - **所有路径都被限制在工作区根内**。`..` 逃逸、绝对路径越界、符号链接逃逸
//!   一律拒绝，返回明确错误而不是默默失败。
//! - 读操作自动执行；**写操作必须经用户批准**（`requires_approval = true`），
//!   批准前不会写盘，界面上会先展示 diff。
//! - 不提供 shell 执行工具。执行任意命令的风险和收益不成正比，
//!   要跑命令请另开终端，由用户自己决定。

use std::fs;
use std::path::{Component, Path, PathBuf};

use neverwork_core::AppResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::diff::{self, DiffLine};

/// 单文件读取上限（超过就截断并告知模型）。
const MAX_READ_BYTES: usize = 256 * 1024;
/// 写文件上限，防止模型一次吐出海量内容把磁盘写满。
const MAX_WRITE_BYTES: usize = 2 * 1024 * 1024;
/// 目录列表上限。
const MAX_LIST_ENTRIES: usize = 400;
/// 搜索返回上限。
const MAX_SEARCH_HITS: usize = 60;

/// 工具描述（发给模型的 JSON Schema）。
#[derive(Debug, Clone, Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    /// 是否需要用户批准（写操作 = true）。不发给模型，仅供本地策略使用。
    #[serde(skip)]
    pub requires_approval: bool,
}

/// 模型发起的一次工具调用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    /// 调用 id，回填 tool 消息时必须原样带上。
    pub id: String,
    pub name: String,
    /// 参数的 JSON 字符串（模型可能给空串，解析时按 `{}` 处理）。
    pub arguments: String,
}

impl ToolCall {
    /// 解析参数；非法 JSON 时返回空对象。
    pub fn args(&self) -> Value {
        let t = self.arguments.trim();
        if t.is_empty() {
            return json!({});
        }
        serde_json::from_str(t).unwrap_or_else(|_| json!({}))
    }

    /// 目标文件（相对路径）；没有 path 参数的工具返回空串。
    pub fn path_arg(&self) -> String {
        self.args()
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// 参数的一行摘要，用于界面展示。
    pub fn summary(&self) -> String {
        let a = self.args();
        match self.name.as_str() {
            "read_file" | "list_dir" => a
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or(".")
                .to_string(),
            "write_file" => {
                let p = a.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                let n = a
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|c| c.lines().count())
                    .unwrap_or(0);
                format!("{p}（写入 {n} 行）")
            }
            "apply_patch" => a
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string(),
            "multi_patch" => {
                let n = a
                    .get("edits")
                    .and_then(|v| v.as_array())
                    .map(|x| x.len())
                    .unwrap_or(0);
                let names: Vec<String> = a
                    .get("edits")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .take(3)
                            .filter_map(|e| e.get("path").and_then(|p| p.as_str()))
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                if names.len() < n {
                    format!("{n} 个文件（{} 等）", names.join("、"))
                } else {
                    format!("{n} 个文件（{}）", names.join("、"))
                }
            }
            "search_files" => {
                let q = a.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                format!("\"{q}\"")
            }
            _ => String::new(),
        }
    }
}

/// 工具执行结果。
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub ok: bool,
    /// 回填给模型的文本（可以是错误信息）。
    pub content: String,
    /// 写操作的变化，供界面渲染 diff。
    pub diff: Vec<DiffLine>,
    /// 变更行数摘要。
    pub added: usize,
    pub removed: usize,
}

impl ToolResult {
    fn ok(call: &ToolCall, content: impl Into<String>) -> Self {
        Self {
            call_id: call.id.clone(),
            name: call.name.clone(),
            ok: true,
            content: content.into(),
            diff: Vec::new(),
            added: 0,
            removed: 0,
        }
    }

    fn err(call: &ToolCall, msg: impl Into<String>) -> Self {
        Self {
            call_id: call.id.clone(),
            name: call.name.clone(),
            ok: false,
            content: format!("错误：{}", msg.into()),
            diff: Vec::new(),
            added: 0,
            removed: 0,
        }
    }
}

/// 工具执行上下文。
pub struct ToolContext {
    /// 工作区根，所有路径都被限制在其内。
    pub root: PathBuf,
}

impl ToolContext {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = fs::canonicalize(&root).unwrap_or(root);
        Self { root }
    }

    /// 把模型给的路径解析成工作区内的真实路径。
    ///
    /// 拒绝：绝对路径越界、`..` 逃逸、解析后不在 root 内。
    pub fn resolve(&self, rel: &str) -> Result<PathBuf, String> {
        let raw = rel.trim();
        let raw = if raw.is_empty() { "." } else { raw };
        let p = Path::new(raw);

        // 拒绝 Windows 盘符绝对路径（跨盘）
        if p.is_absolute() && !p.starts_with(&self.root) {
            return Err(format!("不允许访问工作区外的绝对路径：{raw}"));
        }

        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        };

        // 词法归一化：逐个组件处理 `..`，不依赖文件是否存在
        let normalized = normalize(&joined);
        if !normalized.starts_with(&self.root) {
            return Err(format!("路径越出工作区：{raw}"));
        }

        // 存在的路径再做一次 canonicalize，防符号链接逃逸
        if normalized.exists() {
            if let Ok(canon) = fs::canonicalize(&normalized) {
                if !canon.starts_with(&self.root) {
                    return Err(format!("路径经符号链接逃出工作区：{raw}"));
                }
                return Ok(canon);
            }
        }
        Ok(normalized)
    }

    /// 相对工作区的展示路径。
    pub fn display_rel(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().to_string())
    }
}

/// 词法归一化（不触碰文件系统）。
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 全部工具的 schema。
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file",
            description: "读取工作区内一个文本文件的内容。返回带行号的文本，便于引用具体行。",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "相对工作区根的路径，如 src/main.rs" },
                    "offset": { "type": "integer", "description": "从第几行开始（1 基），可选" },
                    "limit": { "type": "integer", "description": "最多读多少行，可选，默认 400" }
                },
                "required": ["path"]
            }),
            requires_approval: false,
        },
        ToolSpec {
            name: "list_dir",
            description: "列出工作区某个目录下的文件与子目录。",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "相对路径，省略则为工作区根" }
                }
            }),
            requires_approval: false,
        },
        ToolSpec {
            name: "search_files",
            description: "在工作区内按子串搜索文件内容，返回命中的文件、行号与该行内容。",
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "要搜索的文本（区分大小写）" },
                    "path": { "type": "string", "description": "限定目录，可选" }
                },
                "required": ["pattern"]
            }),
            requires_approval: false,
        },
        ToolSpec {
            name: "write_file",
            description: "把内容整体写入一个文件（新建或覆盖）。需要用户批准后才会写盘。",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "相对工作区根的路径" },
                    "content": { "type": "string", "description": "完整文件内容" }
                },
                "required": ["path", "content"]
            }),
            requires_approval: true,
        },
        ToolSpec {
            name: "multi_patch",
            description: "一次性修改多个文件（跨文件重构用）。整批作为事务：全部校验通过才写，中途失败会回滚。一次审批即可。edits 里每项用 content 整文件覆盖，或用 old_string/new_string 局部替换（二选一）。",
            parameters: json!({
                "type": "object",
                "properties": {
                    "edits": {
                        "type": "array",
                        "description": "要改的文件列表，同一文件只能出现一次",
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string", "description": "相对工作区根的路径" },
                                "content": { "type": "string", "description": "整文件新内容（与 old_string 二选一）" },
                                "old_string": { "type": "string", "description": "要被替换的原文，必须在文件中恰好出现一次" },
                                "new_string": { "type": "string", "description": "替换后的文本" }
                            },
                            "required": ["path"]
                        }
                    }
                },
                "required": ["edits"]
            }),
            requires_approval: true,
        },
        ToolSpec {
            name: "apply_patch",
            description: "把文件中一段唯一存在的原文替换成新文本。适合小改动，比整文件重写更安全。需要用户批准。",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "相对工作区根的路径" },
                    "old_string": { "type": "string", "description": "要被替换的原文，必须在文件中恰好出现一次" },
                    "new_string": { "type": "string", "description": "替换后的文本" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
            requires_approval: true,
        },
    ]
}

/// 某工具是否需要批准。
pub fn requires_approval(name: &str) -> bool {
    tool_specs()
        .iter()
        .find(|t| t.name == name)
        .map(|t| t.requires_approval)
        .unwrap_or(true) // 未知工具一律要求批准
}

/// 预览一次写操作会产生什么变化（不写盘），用于审批展示。
pub fn preview(call: &ToolCall, ctx: &ToolContext) -> Result<Vec<DiffLine>, String> {
    let a = call.args();
    match call.name.as_str() {
        "write_file" => {
            let rel = a.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let content = a.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let path = ctx.resolve(rel)?;
            let old = fs::read_to_string(&path).unwrap_or_default();
            Ok(diff::with_context(&diff::diff_lines(&old, content), 3))
        }
        "multi_patch" => crate::multi_patch::preview_preview(call, ctx),
        "apply_patch" => {
            let rel = a.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let old_s = a.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
            let new_s = a.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
            let path = ctx.resolve(rel)?;
            let old = fs::read_to_string(&path).map_err(|e| format!("读取失败：{e}"))?;
            match replace_once(&old, old_s, new_s) {
                Ok(updated) => Ok(diff::with_context(&diff::diff_lines(&old, &updated), 3)),
                Err(e) => Err(e),
            }
        }
        _ => Ok(Vec::new()),
    }
}

/// 执行一次工具调用。
pub fn execute(call: &ToolCall, ctx: &ToolContext) -> ToolResult {
    let a = call.args();
    match call.name.as_str() {
        "read_file" => read_file(call, &a, ctx),
        "list_dir" => list_dir(call, &a, ctx),
        "search_files" => search_files(call, &a, ctx),
        "write_file" => write_file(call, &a, ctx),
        "apply_patch" => apply_patch(call, &a, ctx),
        "multi_patch" => crate::multi_patch::execute(call, ctx),
        other => ToolResult::err(call, format!("未知工具 `{other}`")),
    }
}

fn read_file(call: &ToolCall, a: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = a.get("path").and_then(|v| v.as_str()).unwrap_or("");
    if rel.trim().is_empty() {
        return ToolResult::err(call, "缺少 path 参数");
    }
    let path = match ctx.resolve(rel) {
        Ok(p) => p,
        Err(e) => return ToolResult::err(call, e),
    };
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => return ToolResult::err(call, format!("读取失败：{e}")),
    };
    let truncated = bytes.len() > MAX_READ_BYTES;
    let slice = if truncated {
        &bytes[..MAX_READ_BYTES]
    } else {
        &bytes[..]
    };
    let text = String::from_utf8_lossy(slice);

    let offset = a
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(1)
        .max(1);
    let limit = a
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(400)
        .clamp(1, 2000);

    let mut out = String::new();
    let mut shown = 0usize;
    let total = text.lines().count();
    for (i, line) in text.lines().enumerate() {
        let no = i + 1;
        if no < offset {
            continue;
        }
        if shown >= limit {
            break;
        }
        out.push_str(&format!("{no:>5}| {line}\n"));
        shown += 1;
    }

    let rel_disp = ctx.display_rel(&path);
    let mut header = format!("文件 {rel_disp}（共 {total} 行");
    if shown < total {
        header.push_str(&format!("，显示 {offset}–{}", offset + shown - 1));
    }
    header.push('）');
    if truncated {
        header.push_str("；内容过大已截断");
    }
    ToolResult::ok(call, format!("{header}\n{out}"))
}

fn list_dir(call: &ToolCall, a: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = a.get("path").and_then(|v| v.as_str()).unwrap_or(".");
    let path = match ctx.resolve(rel) {
        Ok(p) => p,
        Err(e) => return ToolResult::err(call, e),
    };
    let entries = match fs::read_dir(&path) {
        Ok(e) => e,
        Err(e) => return ToolResult::err(call, format!("无法读取目录：{e}")),
    };
    let mut items: Vec<(bool, String)> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name == ".git" || name == "target" || name == "node_modules" {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        items.push((is_dir, name));
    }
    items.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.to_lowercase().cmp(&y.1.to_lowercase())));
    let total = items.len();
    items.truncate(MAX_LIST_ENTRIES);

    let mut out = format!(
        "目录 {}（{total} 项）\n",
        ctx.display_rel(&path)
    );
    for (is_dir, name) in &items {
        out.push_str(if *is_dir { "  " } else { "  " });
        out.push_str(name);
        if *is_dir {
            out.push('/');
        }
        out.push('\n');
    }
    if total > MAX_LIST_ENTRIES {
        out.push_str("…（已截断）\n");
    }
    ToolResult::ok(call, out)
}

fn search_files(call: &ToolCall, a: &Value, ctx: &ToolContext) -> ToolResult {
    let pattern = a.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
    if pattern.is_empty() {
        return ToolResult::err(call, "缺少 pattern 参数");
    }
    let base = match a.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => match ctx.resolve(p) {
            Ok(x) => x,
            Err(e) => return ToolResult::err(call, e),
        },
        _ => ctx.root.clone(),
    };

    let mut hits: Vec<String> = Vec::new();
    walk_search(&base, &ctx.root, pattern, &mut hits);
    let mut out = format!("搜索 \"{pattern}\"：命中 {} 处\n", hits.len());
    for h in &hits {
        out.push_str(h);
        out.push('\n');
    }
    if hits.len() >= MAX_SEARCH_HITS {
        out.push_str("…（已达上限，请缩小范围）\n");
    }
    if hits.is_empty() {
        out.push_str("（没有找到匹配内容）\n");
    }
    ToolResult::ok(call, out)
}

fn walk_search(dir: &Path, root: &Path, pattern: &str, out: &mut Vec<String>) {
    if out.len() >= MAX_SEARCH_HITS {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if out.len() >= MAX_SEARCH_HITS {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            ".git" | "target" | "node_modules" | "dist" | ".next" | "__pycache__"
        ) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            walk_search(&p, root, pattern, out);
        } else {
            // 只看文本类文件，跳过明显二进制与超大文件
            let Ok(meta) = e.metadata() else { continue };
            if meta.len() > MAX_READ_BYTES as u64 {
                continue;
            }
            let Ok(bytes) = fs::read(&p) else { continue };
            if bytes.iter().take(4096).any(|b| *b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (i, line) in text.lines().enumerate() {
                if line.contains(pattern) {
                    let rel = p
                        .strip_prefix(root)
                        .map(|x| x.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|_| name.clone());
                    let trimmed = line.trim();
                    let shown: String = if trimmed.chars().count() > 160 {
                        trimmed.chars().take(160).collect::<String>() + "…"
                    } else {
                        trimmed.to_string()
                    };
                    out.push(format!("{}:{}: {}", rel, i + 1, shown));
                    if out.len() >= MAX_SEARCH_HITS {
                        return;
                    }
                }
            }
        }
    }
}

fn write_file(call: &ToolCall, a: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = a.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let content = a.get("content").and_then(|v| v.as_str()).unwrap_or("");
    if rel.trim().is_empty() {
        return ToolResult::err(call, "缺少 path 参数");
    }
    if content.len() > MAX_WRITE_BYTES {
        return ToolResult::err(
            call,
            format!("内容过大（{} 字节，上限 {}）", content.len(), MAX_WRITE_BYTES),
        );
    }
    let path = match ctx.resolve(rel) {
        Ok(p) => p,
        Err(e) => return ToolResult::err(call, e),
    };
    let old = fs::read_to_string(&path).unwrap_or_default();
    let lines = diff::diff_lines(&old, content);
    let st = diff::stats(&lines);

    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return ToolResult::err(call, format!("创建目录失败：{e}"));
        }
    }
    if let Err(e) = write_atomic(&path, content) {
        return ToolResult::err(call, format!("写入失败：{e}"));
    }

    let rel_disp = ctx.display_rel(&path);
    let existed = !old.is_empty();
    ToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        ok: true,
        content: format!(
            "已{} {rel_disp}：+{} −{} 行",
            if existed { "更新" } else { "创建" },
            st.added,
            st.removed
        ),
        diff: diff::with_context(&lines, 3),
        added: st.added,
        removed: st.removed,
    }
}

fn apply_patch(call: &ToolCall, a: &Value, ctx: &ToolContext) -> ToolResult {
    let rel = a.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let old_s = a.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
    let new_s = a.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
    if rel.trim().is_empty() {
        return ToolResult::err(call, "缺少 path 参数");
    }
    if old_s.is_empty() {
        return ToolResult::err(call, "old_string 不能为空");
    }
    let path = match ctx.resolve(rel) {
        Ok(p) => p,
        Err(e) => return ToolResult::err(call, e),
    };
    let old = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => return ToolResult::err(call, format!("读取失败：{e}")),
    };
    let updated = match replace_once(&old, old_s, new_s) {
        Ok(u) => u,
        Err(e) => return ToolResult::err(call, e),
    };
    let lines = diff::diff_lines(&old, &updated);
    let st = diff::stats(&lines);
    if let Err(e) = write_atomic(&path, &updated) {
        return ToolResult::err(call, format!("写入失败：{e}"));
    }
    let rel_disp = ctx.display_rel(&path);
    ToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        ok: true,
        content: format!("已修改 {rel_disp}：+{} −{} 行", st.added, st.removed),
        diff: diff::with_context(&lines, 3),
        added: st.added,
        removed: st.removed,
    }
}

/// 严格替换：old 必须在原文中**恰好出现一次**。
/// 出现 0 次会提示模型重读文件；多次出现要求模型给出更长的上下文。
fn replace_once(text: &str, old: &str, new: &str) -> Result<String, String> {
    let count = text.matches(old).count();
    match count {
        0 => Err("old_string 在文件中找不到。请先 read_file 确认原始内容（注意缩进与空行也要一致）".into()),
        1 => Ok(text.replacen(old, new, 1)),
        n => Err(format!(
            "old_string 在文件中出现了 {n} 次，无法确定要改哪一处。请给出更多上下文使其唯一"
        )),
    }
}

/// 原子写：临时文件 + rename，避免写一半把文件弄坏。
pub(crate) fn write_atomic(path: &Path, content: &str) -> AppResult<()> {
    let tmp = path.with_extension("nw-tool-tmp");
    fs::write(&tmp, content)?;
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (ToolContext, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "nw-tools-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        (ToolContext::new(&dir), dir)
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: args.to_string(),
        }
    }

    #[test]
    fn specs_cover_all_tools() {
        let names: Vec<&str> = tool_specs().iter().map(|t| t.name).collect();
        for n in [
            "read_file",
            "list_dir",
            "search_files",
            "write_file",
            "apply_patch",
            "multi_patch",
        ] {
            assert!(names.contains(&n), "缺少工具 {n}");
        }
    }

    #[test]
    fn write_tools_require_approval_reads_do_not() {
        assert!(!requires_approval("read_file"));
        assert!(!requires_approval("list_dir"));
        assert!(!requires_approval("search_files"));
        assert!(requires_approval("write_file"));
        assert!(requires_approval("apply_patch"));
        assert!(requires_approval("multi_patch"));
        // 未知工具按最严格处理
        assert!(requires_approval("rm_rf"));
    }

    #[test]
    fn rejects_parent_escape() {
        let (ctx, _dir) = ctx();
        let err = ctx.resolve("../../etc/passwd").unwrap_err();
        assert!(err.contains("越出工作区") || err.contains("不允许"), "{err}");
    }

    #[test]
    fn rejects_absolute_outside_root() {
        let (ctx, _dir) = ctx();
        let err = ctx.resolve("C:\\Windows\\System32\\drivers\\etc\\hosts").unwrap_err();
        assert!(err.contains("不允许") || err.contains("越出"), "{err}");
    }

    #[test]
    fn resolves_relative_inside_root() {
        let (ctx, dir) = ctx();
        let p = ctx.resolve("src/main.rs").unwrap();
        assert!(p.starts_with(fs::canonicalize(&dir).unwrap()) || p.starts_with(&dir));
    }

    #[test]
    fn write_then_read_roundtrip() {
        let (ctx, _dir) = ctx();
        let w = execute(
            &call("write_file", json!({"path": "src/a.rs", "content": "fn main() {}\n"})),
            &ctx,
        );
        assert!(w.ok, "{}", w.content);
        assert_eq!(w.added, 2);

        let r = execute(&call("read_file", json!({"path": "src/a.rs"})), &ctx);
        assert!(r.ok);
        assert!(r.content.contains("fn main() {}"));
        assert!(r.content.contains("1|"), "应带行号: {}", r.content);
    }

    #[test]
    fn write_reports_diff() {
        let (ctx, _dir) = ctx();
        execute(
            &call("write_file", json!({"path": "x.txt", "content": "a\nb\nc"})),
            &ctx,
        );
        let w = execute(
            &call("write_file", json!({"path": "x.txt", "content": "a\nB\nc"})),
            &ctx,
        );
        assert!(w.ok);
        assert_eq!(w.added, 1);
        assert_eq!(w.removed, 1);
        assert!(!w.diff.is_empty(), "应带 diff 供界面展示");
    }

    #[test]
    fn apply_patch_requires_unique_match() {
        let (ctx, _dir) = ctx();
        execute(
            &call("write_file", json!({"path": "p.txt", "content": "dup\ndup\n"})),
            &ctx,
        );
        let r = execute(
            &call(
                "apply_patch",
                json!({"path": "p.txt", "old_string": "dup", "new_string": "x"}),
            ),
            &ctx,
        );
        assert!(!r.ok, "重复出现时不应修改");
        assert!(r.content.contains("2 次"), "{}", r.content);
    }

    #[test]
    fn apply_patch_not_found_is_explicit() {
        let (ctx, _dir) = ctx();
        execute(
            &call("write_file", json!({"path": "q.txt", "content": "hello\n"})),
            &ctx,
        );
        let r = execute(
            &call(
                "apply_patch",
                json!({"path": "q.txt", "old_string": "nope", "new_string": "x"}),
            ),
            &ctx,
        );
        assert!(!r.ok);
        assert!(r.content.contains("找不到"), "{}", r.content);
    }

    #[test]
    fn apply_patch_success() {
        let (ctx, _dir) = ctx();
        execute(
            &call("write_file", json!({"path": "r.txt", "content": "let a = 1;\nlet b = 2;\n"})),
            &ctx,
        );
        let r = execute(
            &call(
                "apply_patch",
                json!({"path": "r.txt", "old_string": "let b = 2;", "new_string": "let b = 20;"}),
            ),
            &ctx,
        );
        assert!(r.ok, "{}", r.content);
        let read = execute(&call("read_file", json!({"path": "r.txt"})), &ctx);
        assert!(read.content.contains("let b = 20;"));
    }

    #[test]
    fn preview_does_not_write() {
        let (ctx, _dir) = ctx();
        execute(&call("write_file", json!({"path": "pv.txt", "content": "old"})), &ctx);
        let d = preview(
            &call("write_file", json!({"path": "pv.txt", "content": "new"})),
            &ctx,
        )
        .unwrap();
        assert!(!d.is_empty());
        let read = execute(&call("read_file", json!({"path": "pv.txt"})), &ctx);
        assert!(read.content.contains("old"), "预览不应改动文件");
    }

    #[test]
    fn list_dir_skips_noise() {
        let (ctx, dir) = ctx();
        fs::create_dir_all(dir.join("target")).unwrap();
        fs::write(dir.join("src").join("m.rs"), "x").unwrap();
        let r = execute(&call("list_dir", json!({"path": "."})), &ctx);
        assert!(r.ok);
        assert!(r.content.contains("src/"));
        assert!(!r.content.contains("target"), "应跳过 target");
    }

    #[test]
    fn search_finds_matches() {
        let (ctx, _dir) = ctx();
        execute(
            &call("write_file", json!({"path": "s1.txt", "content": "alpha\nneedle here\n"})),
            &ctx,
        );
        let r = execute(&call("search_files", json!({"pattern": "needle"})), &ctx);
        assert!(r.ok);
        assert!(r.content.contains("s1.txt"), "{}", r.content);
        assert!(r.content.contains("2:"), "应带行号: {}", r.content);
    }

    #[test]
    fn unknown_tool_is_error() {
        let (ctx, _dir) = ctx();
        let r = execute(&call("magic", json!({})), &ctx);
        assert!(!r.ok);
        assert!(r.content.contains("未知工具"));
    }

    #[test]
    fn empty_arguments_do_not_panic() {
        let c = ToolCall {
            id: "x".into(),
            name: "read_file".into(),
            arguments: String::new(),
        };
        let a = c.args();
        assert!(a.is_object());
    }

    #[test]
    fn summary_is_readable() {
        let c = call("write_file", json!({"path": "a/b.rs", "content": "1\n2\n3"}));
        let s = c.summary();
        assert!(s.contains("a/b.rs"));
        assert!(s.contains("3 行"), "{s}");
    }
}
