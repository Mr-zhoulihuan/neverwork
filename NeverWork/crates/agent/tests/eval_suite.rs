//! 工具层 eval 回归集（金标用例）。
//!
//! 为什么是「工具层」而不是「问模型」：问模型的断言要联网、要密钥、还会
//! 因为模型换版本漂移，跑一次几十秒，回归时没人愿意跑。工具层是 Agent 的
//! 手脚，它错了后面全错，而且完全确定性——毫秒级、离线、能进 CI。
//!
//! 每条用例对应一条「已知必须成立」的行为契约：
//! - 成功路径：输出形状（行号、命中格式、diff 统计）不能漂
//! - 失败路径：错误码与 `retryable` 不能漂（模型据此决定重试还是改参数）
//! - 安全边界：路径逃逸一律 `PATH_DENIED`，写操作一律 `requires_approval`
//! - 幂等：重放同一笔写不产生二次改动
//!
//! 需要真实模型的端到端验证在 `deepseek_tools.rs`，那边缺密钥会跳过。

use std::path::{Path, PathBuf};

use neverwork_agent::tools::{self, ToolCall, ToolContext};
use neverwork_agent::ToolResult;

/// 建一个隔离的临时工作区（每个用例一个，避免互相污染）。
fn ws(tag: &str) -> (ToolContext, PathBuf) {
    let dir = std::env::temp_dir().join(format!("nw-eval-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ctx = ToolContext::new(&dir);
    (ctx, dir)
}

fn call(name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: format!("eval-{name}"),
        name: name.into(),
        arguments: args.into(),
    }
}

fn run(ctx: &ToolContext, name: &str, args: &str) -> ToolResult {
    tools::execute(&call(name, args), ctx)
}

fn write(path: &Path, body: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

// ============ 读取类 ============

#[test]
fn e01_read_file_numbers_every_line() {
    let (ctx, dir) = ws("r01");
    write(&dir.join("a.txt"), "alpha\nbeta\n");

    let r = run(&ctx, "read_file", r#"{"path":"a.txt"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("共 2 行"), "要报总行数：{}", r.content);
    assert!(r.content.contains("| alpha"), "每行带行号：{}", r.content);
    assert!(r.content.contains("2| beta"), "{}", r.content);
}

#[test]
fn e02_read_file_missing_arg_is_bad_args() {
    let (ctx, _dir) = ws("r02");
    let r = run(&ctx, "read_file", r#"{"path":"   "}"#);
    assert!(!r.ok);
    assert_eq!(r.error_code, Some("BAD_ARGS"));
}

#[test]
fn e03_read_file_absent_is_not_found_and_not_retryable() {
    let (ctx, _dir) = ws("r03");
    let r = run(&ctx, "read_file", r#"{"path":"nope.txt"}"#);
    assert_eq!(r.error_code, Some("NOT_FOUND"), "{}", r.content);
    // 找不到再试一百次也一样，模型该换路径而不是重试
    assert!(
        r.content.contains("\"retryable\":false"),
        "NOT_FOUND 不该标可重试：{}",
        r.content
    );
}

#[test]
fn e04_read_file_honours_offset_and_limit() {
    let (ctx, dir) = ws("r04");
    let body: String = (1..=50).map(|i| format!("line{i}\n")).collect();
    write(&dir.join("many.txt"), &body);

    let r = run(&ctx, "read_file", r#"{"path":"many.txt","offset":10,"limit":3}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("10| line10"), "{}", r.content);
    assert!(r.content.contains("12| line12"), "{}", r.content);
    assert!(!r.content.contains("13| line13"), "limit 之外不应出现：{}", r.content);
    assert!(r.content.contains("显示 10–12"), "要标明窗口：{}", r.content);
}

#[test]
fn e05_read_file_truncates_oversize() {
    let (ctx, dir) = ws("r05");
    // 256KB 上限：写 300KB
    write(&dir.join("big.txt"), &"x".repeat(300 * 1024));
    let r = run(&ctx, "read_file", r#"{"path":"big.txt"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(
        r.content.contains("内容过大已截断"),
        "截断要明说，否则模型以为读全了：{}",
        &r.content[..r.content.len().min(200)]
    );
}

#[test]
fn e06_read_file_rejects_parent_escape() {
    let (ctx, _dir) = ws("r06");
    let r = run(&ctx, "read_file", r#"{"path":"../../etc/passwd"}"#);
    assert_eq!(r.error_code, Some("PATH_DENIED"), "{}", r.content);
}

#[test]
fn e07_read_file_rejects_cross_drive_absolute() {
    let (ctx, _dir) = ws("r07");
    let other = if cfg!(windows) { "D:/secret.txt" } else { "/etc/passwd" };
    let args = format!(r#"{{"path":"{other}"}}"#);
    let r = run(&ctx, "read_file", &args);
    assert_eq!(r.error_code, Some("PATH_DENIED"), "{}", r.content);
}

#[test]
fn e08_read_file_survives_invalid_utf8() {
    let (ctx, dir) = ws("r08");
    // 非 UTF-8 字节不能 panic，也不能直接报错：用有损解码给模型看
    std::fs::write(dir.join("bin.dat"), b"ok\xff\xfeend").unwrap();
    let r = run(&ctx, "read_file", r#"{"path":"bin.dat"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("ok"), "{}", r.content);
    assert!(r.content.contains("end"), "{}", r.content);
}

// ============ 目录与搜索 ============

#[test]
fn e09_list_dir_marks_directories_with_slash() {
    let (ctx, dir) = ws("l01");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    write(&dir.join("README.md"), "x\n");

    let r = run(&ctx, "list_dir", r#"{"path":"."}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("src/"), "目录要带斜杠：{}", r.content);
    assert!(r.content.contains("README.md"), "{}", r.content);
}

#[test]
fn e10_list_dir_path_escape_denied() {
    let (ctx, _dir) = ws("l02");
    let r = run(&ctx, "list_dir", r#"{"path":"../.."}"#);
    assert_eq!(r.error_code, Some("PATH_DENIED"), "{}", r.content);
}

#[test]
fn e11_search_files_reports_path_line_and_text() {
    let (ctx, dir) = ws("s01");
    write(&dir.join("src/main.rs"), "fn main() {\n    let needle = 1;\n}\n");

    let r = run(&ctx, "search_files", r#"{"pattern":"needle"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("命中 1 处"), "{}", r.content);
    assert!(r.content.contains("src/main.rs:2:"), "要带文件:行号：{}", r.content);
}

#[test]
fn e12_search_files_no_match_is_explicit() {
    let (ctx, dir) = ws("s02");
    write(&dir.join("a.txt"), "hello\n");
    let r = run(&ctx, "search_files", r#"{"pattern":"绝不存在的内容"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("（没有找到匹配内容）"), "{}", r.content);
}

#[test]
fn e13_search_files_skips_vendor_dirs() {
    let (ctx, dir) = ws("s03");
    write(&dir.join("node_modules/pkg/index.js"), "MAGIC_TOKEN\n");
    write(&dir.join("src/app.js"), "MAGIC_TOKEN\n");

    let r = run(&ctx, "search_files", r#"{"pattern":"MAGIC_TOKEN"}"#);
    assert!(r.content.contains("命中 1 处"), "只该命中源码那一处：{}", r.content);
    assert!(!r.content.contains("node_modules"), "{}", r.content);
}

#[test]
fn e14_search_files_missing_pattern_is_bad_args() {
    let (ctx, _dir) = ws("s04");
    let r = run(&ctx, "search_files", r#"{"pattern":""}"#);
    assert_eq!(r.error_code, Some("BAD_ARGS"));
}

// ============ 写入类 ============

#[test]
fn e15_write_file_creates_and_reports_stats() {
    let (ctx, dir) = ws("w01");
    let r = run(&ctx, "write_file", r#"{"path":"sub/new.txt","content":"a\nb\n"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("创建"), "首次写应是「创建」：{}", r.content);
    // 具体行数由 diff 实现决定（尾随换行算不算一行），这里只卡「有新增、无删除」
    assert!(r.added >= 2, "要报新增行数：{}（added={}）", r.content, r.added);
    assert_eq!(r.removed, 0, "新建文件不该有删除行");
    assert_eq!(std::fs::read_to_string(dir.join("sub/new.txt")).unwrap(), "a\nb\n");
}

#[test]
fn e16_write_file_updates_existing() {
    let (ctx, dir) = ws("w02");
    write(&dir.join("f.txt"), "old\n");
    let r = run(&ctx, "write_file", r#"{"path":"f.txt","content":"new\n"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("更新"), "{}", r.content);
    assert_eq!(std::fs::read_to_string(dir.join("f.txt")).unwrap(), "new\n");
}

#[test]
fn e17_write_file_oversize_is_too_large() {
    let (ctx, _dir) = ws("w03");
    // 上限 2MB：造 2MB+1 的内容
    let content = "y".repeat(2 * 1024 * 1024 + 1);
    let args = serde_json::json!({ "path": "huge.txt", "content": content }).to_string();
    let r = run(&ctx, "write_file", &args);
    assert_eq!(r.error_code, Some("TOO_LARGE"), "{}", r.content);
    assert!(r.content.contains("\"retryable\":false"), "{}", r.content);
}

#[test]
fn e18_write_ops_require_approval_reads_do_not() {
    for name in ["write_file", "apply_patch", "multi_patch"] {
        assert!(tools::requires_approval(name), "{name} 是写操作，必须审批");
    }
    for name in ["read_file", "list_dir", "search_files"] {
        assert!(!tools::requires_approval(name), "{name} 是只读，不该拦");
    }
    // 未知工具宁拦勿放
    assert!(tools::requires_approval("no_such_tool"));
}

#[test]
fn e19_preview_never_touches_disk() {
    let (ctx, dir) = ws("w04");
    write(&dir.join("p.txt"), "before\n");
    let c = call("write_file", r#"{"path":"p.txt","content":"after\n"}"#);
    let diff = tools::preview(&c, &ctx).expect("预览应成功");
    assert!(!diff.is_empty(), "应有 diff 行");
    assert_eq!(
        std::fs::read_to_string(dir.join("p.txt")).unwrap(),
        "before\n",
        "预览阶段绝不能落盘"
    );
}

// ============ 补丁与幂等 ============

#[test]
fn e20_apply_patch_replaces_unique_snippet() {
    let (ctx, dir) = ws("p01");
    write(&dir.join("a.rs"), "let x = 1;\nlet y = 2;\n");
    let r = run(
        &ctx,
        "apply_patch",
        r#"{"path":"a.rs","old_string":"let x = 1;","new_string":"let x = 42;"}"#,
    );
    assert!(r.ok, "{}", r.content);
    assert_eq!(
        std::fs::read_to_string(dir.join("a.rs")).unwrap(),
        "let x = 42;\nlet y = 2;\n"
    );
}

#[test]
fn e21_apply_patch_ambiguous_match_is_patch_no_match() {
    let (ctx, dir) = ws("p02");
    write(&dir.join("a.rs"), "dup\ndup\n");
    let r = run(
        &ctx,
        "apply_patch",
        r#"{"path":"a.rs","old_string":"dup","new_string":"once"}"#,
    );
    assert_eq!(r.error_code, Some("PATCH_NO_MATCH"), "{}", r.content);
    assert!(r.content.contains("2 次"), "要说清出现几次：{}", r.content);
    // 文件必须原样
    assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "dup\ndup\n");
}

#[test]
fn e22_apply_patch_empty_old_is_bad_args() {
    let (ctx, dir) = ws("p03");
    write(&dir.join("a.rs"), "x\n");
    let r = run(
        &ctx,
        "apply_patch",
        r#"{"path":"a.rs","old_string":"","new_string":"y"}"#,
    );
    assert_eq!(r.error_code, Some("BAD_ARGS"), "{}", r.content);
}

#[test]
fn e23_replayed_patch_is_idempotent() {
    let (ctx, dir) = ws("p04");
    write(&dir.join("a.rs"), "let x = 1;\n");
    let args = r#"{"path":"a.rs","old_string":"let x = 1;","new_string":"let x = 2;"}"#;
    let first = run(&ctx, "apply_patch", args);
    assert!(first.ok, "第一次应成功：{}", first.content);
    // 断点续跑会把最后一笔重放一次：不能报错，也不能再改一遍
    let second = run(&ctx, "apply_patch", args);
    assert!(second.ok, "重放应幂等成功：{}", second.content);
    assert!(second.content.contains("幂等跳过"), "{}", second.content);
    assert_eq!(
        std::fs::read_to_string(dir.join("a.rs")).unwrap(),
        "let x = 2;\n"
    );
}

#[test]
fn e24_replayed_write_is_idempotent() {
    let (ctx, dir) = ws("w05");
    let args = r#"{"path":"same.txt","content":"fixed\n"}"#;
    assert!(run(&ctx, "write_file", args).ok);
    let second = run(&ctx, "write_file", args);
    assert!(second.ok, "{}", second.content);
    assert!(second.content.contains("幂等跳过"), "{}", second.content);
    assert_eq!(std::fs::read_to_string(dir.join("same.txt")).unwrap(), "fixed\n");
}

#[test]
fn e25_multi_patch_is_all_or_nothing() {
    let (ctx, dir) = ws("mp01");
    write(&dir.join("a.rs"), "AAA\n");
    write(&dir.join("b.rs"), "BBB\n");

    let args = serde_json::json!({
        "edits": [
            { "path": "a.rs", "old_string": "AAA", "new_string": "A2" },
            // 故意让第二项匹配不到：整批必须回滚
            { "path": "b.rs", "old_string": "NOT_THERE", "new_string": "B2" }
        ]
    })
    .to_string();
    let r = run(&ctx, "multi_patch", &args);
    assert!(!r.ok, "{}", r.content);
    assert_eq!(
        std::fs::read_to_string(dir.join("a.rs")).unwrap(),
        "AAA\n",
        "整批失败时第一个文件也必须回滚"
    );
    assert_eq!(std::fs::read_to_string(dir.join("b.rs")).unwrap(), "BBB\n");
}

#[test]
fn e26_multi_patch_applies_all_when_valid() {
    let (ctx, dir) = ws("mp02");
    write(&dir.join("a.rs"), "AAA\n");
    write(&dir.join("c.rs"), "CCC\n");
    let args = serde_json::json!({
        "edits": [
            { "path": "a.rs", "old_string": "AAA", "new_string": "A9" },
            { "path": "new.rs", "content": "NEW\n" }
        ]
    })
    .to_string();
    let r = run(&ctx, "multi_patch", &args);
    assert!(r.ok, "{}", r.content);
    assert_eq!(std::fs::read_to_string(dir.join("a.rs")).unwrap(), "A9\n");
    assert_eq!(std::fs::read_to_string(dir.join("new.rs")).unwrap(), "NEW\n");
}

// ============ 契约与清单 ============

#[test]
fn e27_unknown_tool_is_unknown_tool() {
    let (ctx, _dir) = ws("x01");
    let r = run(&ctx, "rm_rf", r#"{"path":"."}"#);
    assert_eq!(r.error_code, Some("UNKNOWN_TOOL"), "{}", r.content);
}

#[test]
fn e28_broken_json_args_is_bad_args() {
    let (ctx, _dir) = ws("x02");
    // 模型偶尔会吐半个 JSON：不能 panic，要给出可读错误
    let r = run(&ctx, "read_file", r#"{"path":"#);
    assert!(!r.ok, "半截 JSON 不该被当成空参数执行");
    assert_eq!(r.error_code, Some("BAD_ARGS"), "{}", r.content);
}

#[test]
fn e29_error_contract_shape_is_stable() {
    let (ctx, _dir) = ws("x03");
    let r = run(&ctx, "read_file", r#"{"path":"missing.txt"}"#);
    let v: serde_json::Value = serde_json::from_str(&r.content).expect("错误必须是 JSON");
    assert_eq!(v["ok"], false);
    assert!(v["errorCode"].is_string(), "{}", r.content);
    assert!(v["errorMsg"].is_string(), "{}", r.content);
    assert!(v["retryable"].is_boolean(), "{}", r.content);
}

#[test]
fn e30_tool_specs_expose_the_full_read_write_set() {
    let specs = neverwork_agent::tools::tool_specs();
    let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
    for want in [
        "read_file",
        "list_dir",
        "search_files",
        "search_code",
        "write_file",
        "apply_patch",
        "multi_patch",
    ] {
        assert!(names.iter().any(|n| n == want), "工具清单缺 {want}：{names:?}");
    }
    // 每个工具都得有非空参数 schema，否则模型不知道要传什么
    for s in &specs {
        assert!(!s.description.trim().is_empty(), "{} 缺描述", s.name);
        assert!(
            s.parameters.is_object(),
            "{} 的参数 schema 必须是对象",
            s.name
        );
    }
}
