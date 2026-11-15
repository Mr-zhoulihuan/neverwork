//! 跨文件批量补丁：一次调用改多个文件，整批只审批一次。
//!
//! 为什么需要它：大重构（重命名、改签名）要动十几个文件，逐个 `apply_patch`
//! 意味着十几次审批点击，而且中间被拒会留下「改了一半」的状态。
//! `multi_patch` 把这一批当作**一个事务**：
//!
//! 1. **先全量校验**（路径合法 / 原文唯一 / 体积合规），任何一项不通过就整批不改；
//! 2. 再逐个写入，中途失败则把已写的文件回滚回原内容。
//!
//! 于是用户只看到一次 diff、点一次允许，要么全落地要么全不动。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::diff::{self, DiffLine};
use crate::tools::{self, ToolCall, ToolContext, ToolResult};

/// 单次 multi_patch 最多几个文件（防止模型一口气改几百个）。
pub const MAX_FILES: usize = 20;
/// 单个文件内容上限。
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

/// 一个文件的目标状态。
#[derive(Debug, Clone)]
pub struct FileEdit {
    pub path: String,
    /// 整文件覆盖时的新内容。
    pub content: Option<String>,
    /// 局部替换的原文（必须唯一存在）。
    pub old_string: Option<String>,
    pub new_string: Option<String>,
}

/// 校验通过后的一次待写入。
struct PlannedEdit {
    path: PathBuf,
    display: String,
    old: String,
    new: String,
}

/// 解析 `edits` 数组。
pub fn parse_edits(args: &Value) -> Result<Vec<FileEdit>, String> {
    let Some(arr) = args.get("edits").and_then(|v| v.as_array()) else {
        return Err("缺少 edits 数组".into());
    };
    if arr.is_empty() {
        return Err("edits 不能为空".into());
    }
    if arr.len() > MAX_FILES {
        return Err(format!(
            "一次最多改 {MAX_FILES} 个文件，当前 {} 个。请拆成多批",
            arr.len()
        ));
    }

    let mut out = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        let path = item
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if path.is_empty() {
            return Err(format!("第 {} 项缺少 path", i + 1));
        }
        let content = item
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let old_string = item
            .get("old_string")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let new_string = item
            .get("new_string")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // 两种形态二选一：整文件 content，或 old/new 替换。混用直接报错，
        // 免得模型写出「既覆盖又替换」这种互相矛盾的意图。
        let has_content = content.is_some();
        let has_patch = old_string.is_some() || new_string.is_some();
        if has_content && has_patch {
            return Err(format!(
                "第 {} 项（{path}）同时给了 content 和 old_string/new_string，只能二选一",
                i + 1
            ));
        }
        if !has_content && old_string.is_none() {
            return Err(format!(
                "第 {} 项（{path}）缺少 content 或 old_string",
                i + 1
            ));
        }
        if old_string.is_some() && new_string.is_none() {
            return Err(format!(
                "第 {} 项（{path}）给了 old_string 但没有 new_string（想删除请给空串）",
                i + 1
            ));
        }

        out.push(FileEdit {
            path,
            content,
            old_string,
            new_string,
        });
    }

    // 同一个文件出现两次会让「先校验后写入」的语义变模糊，直接拒绝。
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &out {
        *seen.entry(e.path.as_str()).or_insert(0) += 1;
    }
    if let Some((p, n)) = seen.iter().find(|(_, n)| **n > 1) {
        return Err(format!("文件 {p} 在 edits 里出现了 {n} 次，请合并成一项"));
    }

    Ok(out)
}

/// 第一阶段：全量校验并算出每个文件的目标内容。**不改盘**。
fn plan(
    edits: &[FileEdit],
    ctx: &ToolContext,
) -> Result<Vec<PlannedEdit>, String> {
    let mut planned = Vec::new();
    for e in edits {
        // 写路径：除了越界，还要拦下 `.git` 元数据目录（hooks 等于持久后门）
        let path = ctx.resolve_write(&e.path)?;
        let old = std::fs::read_to_string(&path).unwrap_or_default();

        let new = if let Some(content) = &e.content {
            if content.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "{} 内容过大（{} 字节，上限 {}）",
                    e.path,
                    content.len(),
                    MAX_FILE_BYTES
                ));
            }
            content.clone()
        } else {
            let old_s = e.old_string.as_deref().unwrap_or("");
            let new_s = e.new_string.as_deref().unwrap_or("");
            if old_s.is_empty() {
                return Err(format!("{} 的 old_string 不能为空", e.path));
            }
            if !path.exists() {
                return Err(format!(
                    "{} 不存在。新建文件请用 content 而不是 old_string",
                    e.path
                ));
            }
            let count = old.matches(old_s).count();
            match count {
                0 => {
                    return Err(format!(
                        "{}：old_string 找不到。请先 read_file 确认原始内容（缩进与空行也要一致）",
                        e.path
                    ))
                }
                1 => {}
                n => {
                    return Err(format!(
                        "{}：old_string 出现了 {n} 次，无法确定改哪一处。请给出更多上下文",
                        e.path
                    ))
                }
            }
            old.replacen(old_s, new_s, 1)
        };

        planned.push(PlannedEdit {
            display: ctx.display_rel(&path),
            path,
            old,
            new,
        });
    }
    Ok(planned)
}

/// 预览整批 diff（合并展示，按文件分段）。**不改盘**。
pub fn preview_preview(call: &ToolCall, ctx: &ToolContext) -> Result<Vec<DiffLine>, String> {
    let edits = parse_edits(&call.args())?;
    let planned = plan(&edits, ctx)?;
    Ok(combined_diff(&planned))
}

/// 把所有文件的 diff 拼成一条，文件之间插一行标题。
fn combined_diff(planned: &[PlannedEdit]) -> Vec<DiffLine> {
    let mut out: Vec<DiffLine> = Vec::new();
    for (i, p) in planned.iter().enumerate() {
        if i > 0 {
            out.push(DiffLine {
                kind: diff::DiffKind::Same,
                old_no: None,
                new_no: None,
                text: String::new(),
            });
        }
        out.push(DiffLine {
            kind: diff::DiffKind::Same,
            old_no: None,
            new_no: None,
            text: format!("▸ {}", p.display),
        });
        let mut lines = diff::with_context(&diff::diff_lines(&p.old, &p.new), 2);
        out.append(&mut lines);
    }
    out
}

/// 执行：先全量校验，再逐个写入；中途失败则回滚已写的。
pub fn execute(call: &ToolCall, ctx: &ToolContext) -> ToolResult {
    let edits = match parse_edits(&call.args()) {
        Ok(e) => e,
        Err(msg) => return ToolResult::err_code(call, "BAD_ARGS", msg, false),
    };

    // ---- 第一阶段：校验（任何一项失败 → 整批不动） ----
    let planned = match plan(&edits, ctx) {
        Ok(p) => p,
        Err(msg) => {
            return ToolResult::err_code(call, "PATCH_INVALID", format!("整批未改动：{msg}"), false)
        }
    };

    // ---- 第二阶段：写入，失败回滚 ----
    let mut written: Vec<&PlannedEdit> = Vec::new();
    for p in &planned {
        if let Some(parent) = p.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match tools::write_atomic(&p.path, &p.new) {
            Ok(()) => written.push(p),
            Err(e) => {
                // 回滚已经写过的
                for w in &written {
                    let _ = tools::write_atomic(&w.path, &w.old);
                }
                return ToolResult::err_code(
                    call,
                    "IO_ERROR",
                    format!(
                        "写入 {} 失败：{e}。已回滚 {} 个文件，全部保持原样",
                        p.display,
                        written.len()
                    ),
                    true,
                );
            }
        }
    }

    // ---- 汇总 ----
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut lines: Vec<String> = Vec::new();
    for p in &planned {
        let st = diff::stats(&diff::diff_lines(&p.old, &p.new));
        added += st.added;
        removed += st.removed;
        lines.push(format!("  {} +{} −{}", p.display, st.added, st.removed));
    }

    ToolResult::ok_with_diff(
        call,
        format!(
            "已一次性修改 {} 个文件：+{} −{}\n{}",
            planned.len(),
            added,
            removed,
            lines.join("\n")
        ),
        combined_diff(&planned),
        added,
        removed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> (ToolContext, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "nw-multi-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (ToolContext::new(&dir), dir)
    }

    fn call(edits: Value) -> ToolCall {
        ToolCall {
            id: "m1".into(),
            name: "multi_patch".into(),
            arguments: json!({ "edits": edits }).to_string(),
        }
    }

    fn write(p: &std::path::Path, s: &str) {
        std::fs::write(p, s).unwrap();
    }

    fn read(p: &std::path::Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn applies_all_edits() {
        let (ctx, dir) = ctx();
        write(&dir.join("a.txt"), "alpha\n");
        write(&dir.join("b.txt"), "beta\n");

        let r = execute(
            &call(json!([
                { "path": "a.txt", "old_string": "alpha", "new_string": "ALPHA" },
                { "path": "b.txt", "old_string": "beta", "new_string": "BETA" }
            ])),
            &ctx,
        );
        assert!(r.ok, "{}", r.content);
        assert_eq!(read(&dir.join("a.txt")), "ALPHA\n");
        assert_eq!(read(&dir.join("b.txt")), "BETA\n");
        assert_eq!(r.added, 2);
        assert!(r.content.contains("2 个文件"));
    }

    #[test]
    fn can_create_new_file_with_content() {
        let (ctx, dir) = ctx();
        let r = execute(
            &call(json!([{ "path": "new.txt", "content": "hello\n" }])),
            &ctx,
        );
        assert!(r.ok, "{}", r.content);
        assert_eq!(read(&dir.join("new.txt")), "hello\n");
    }

    #[test]
    fn one_bad_edit_aborts_whole_batch() {
        let (ctx, dir) = ctx();
        write(&dir.join("a.txt"), "alpha\n");
        write(&dir.join("b.txt"), "beta\n");

        // 第二项的 old_string 不存在 → 整批都不能改
        let r = execute(
            &call(json!([
                { "path": "a.txt", "old_string": "alpha", "new_string": "ALPHA" },
                { "path": "b.txt", "old_string": "nope", "new_string": "X" }
            ])),
            &ctx,
        );
        assert!(!r.ok, "应当整体失败");
        assert_eq!(read(&dir.join("a.txt")), "alpha\n", "a.txt 不能被改");
        assert_eq!(read(&dir.join("b.txt")), "beta\n");
        assert!(
            r.content.contains("整批未改动"),
            "错误信息应说明整批未改：{}",
            r.content
        );
    }

    #[test]
    fn ambiguous_old_string_aborts() {
        let (ctx, dir) = ctx();
        write(&dir.join("a.txt"), "dup\ndup\n");
        let r = execute(
            &call(json!([{ "path": "a.txt", "old_string": "dup", "new_string": "x" }])),
            &ctx,
        );
        assert!(!r.ok);
        assert!(r.content.contains("2 次"), "{}", r.content);
    }

    #[test]
    fn rejects_path_escape() {
        let (ctx, _dir) = ctx();
        let r = execute(
            &call(json!([{ "path": "../../evil.txt", "content": "x" }])),
            &ctx,
        );
        assert!(!r.ok);
        assert!(r.content.contains("越出") || r.content.contains("不允许"), "{}", r.content);
    }

    #[test]
    fn rejects_duplicate_paths() {
        let (ctx, _dir) = ctx();
        let r = execute(
            &call(json!([
                { "path": "a.txt", "content": "1" },
                { "path": "a.txt", "content": "2" }
            ])),
            &ctx,
        );
        assert!(!r.ok);
        assert!(r.content.contains("2 次"), "{}", r.content);
    }

    #[test]
    fn rejects_mixed_content_and_patch() {
        let (ctx, _dir) = ctx();
        let r = execute(
            &call(json!([{
                "path": "a.txt",
                "content": "x",
                "old_string": "a",
                "new_string": "b"
            }])),
            &ctx,
        );
        assert!(!r.ok);
        assert!(r.content.contains("二选一"), "{}", r.content);
    }

    #[test]
    fn rejects_empty_edits() {
        let (ctx, _dir) = ctx();
        let r = execute(&call(json!([])), &ctx);
        assert!(!r.ok);
        assert!(r.content.contains("不能为空"));
    }

    #[test]
    fn rejects_too_many_files() {
        let (ctx, _dir) = ctx();
        let many: Vec<Value> = (0..MAX_FILES + 1)
            .map(|i| json!({ "path": format!("f{i}.txt"), "content": "x" }))
            .collect();
        let r = execute(&call(json!(many)), &ctx);
        assert!(!r.ok);
        assert!(r.content.contains("最多改"), "{}", r.content);
    }

    #[test]
    fn preview_does_not_write() {
        let (ctx, dir) = ctx();
        write(&dir.join("a.txt"), "old\n");
        let d = preview_preview(
            &call(json!([{ "path": "a.txt", "old_string": "old", "new_string": "new" }])),
            &ctx,
        )
        .unwrap();
        assert!(!d.is_empty());
        assert_eq!(read(&dir.join("a.txt")), "old\n", "预览不能改盘");
        // 应带文件名标记
        assert!(d.iter().any(|l| l.text.contains("a.txt")));
    }

    #[test]
    fn combined_diff_separates_files() {
        let (ctx, dir) = ctx();
        write(&dir.join("a.txt"), "a\n");
        write(&dir.join("b.txt"), "b\n");
        let d = preview_preview(
            &call(json!([
                { "path": "a.txt", "old_string": "a", "new_string": "A" },
                { "path": "b.txt", "old_string": "b", "new_string": "B" }
            ])),
            &ctx,
        )
        .unwrap();
        let headers = d.iter().filter(|l| l.text.starts_with("▸ ")).count();
        assert_eq!(headers, 2, "每个文件一个标题行");
    }

    #[test]
    fn chinese_content_roundtrip() {
        let (ctx, dir) = ctx();
        write(&dir.join("c.txt"), "第一行\n第二行\n");
        let r = execute(
            &call(json!([{
                "path": "c.txt",
                "old_string": "第二行",
                "new_string": "改过的第二行"
            }])),
            &ctx,
        );
        assert!(r.ok, "{}", r.content);
        assert_eq!(read(&dir.join("c.txt")), "第一行\n改过的第二行\n");
    }
}
