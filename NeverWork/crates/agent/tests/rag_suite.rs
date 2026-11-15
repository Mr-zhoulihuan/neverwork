//! 本地检索（RAG）回归：分词、分块、打分重排、引用、工具契约。
//!
//! 这些用例盯的是「检索退化但不报错」的情况——排序错了、行号偏了、
//! 二进制被当文本搜，都不会让程序崩，只会让模型拿到错误上下文。

use std::path::PathBuf;

use neverwork_agent::rag::{self, Chunk};
use neverwork_agent::tools::{self, ToolCall, ToolContext};
use neverwork_agent::ToolResult;

fn ws(tag: &str) -> (ToolContext, PathBuf) {
    let dir = std::env::temp_dir().join(format!("nw-rag-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    (ToolContext::new(&dir), dir)
}

fn write(path: &std::path::Path, body: &str) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn chunk(path: &str, start: usize, text: &str) -> Chunk {
    let lines = text.lines().count().max(1);
    Chunk {
        path: path.into(),
        start,
        end: start + lines - 1,
        terms: rag::tokenize(text),
        text: text.into(),
    }
}

fn run(ctx: &ToolContext, name: &str, args: &str) -> ToolResult {
    tools::execute(
        &ToolCall {
            id: "r".into(),
            name: name.into(),
            arguments: args.into(),
        },
        ctx,
    )
}

// ============ 分词 ============

#[test]
fn identifiers_are_split_for_partial_matches() {
    let t = rag::tokenize("history_for_model MessageKind maxTokens");
    for want in ["history_for_model", "history", "model"] {
        assert!(t.iter().any(|x| x == want), "缺少 {want}: {t:?}");
    }
    for want in ["messagekind", "message", "kind"] {
        assert!(t.iter().any(|x| x == want), "缺少 {want}: {t:?}");
    }
    for want in ["maxtokens", "max", "tokens"] {
        assert!(t.iter().any(|x| x == want), "缺少 {want}: {t:?}");
    }
}

#[test]
fn chinese_uses_bigrams_so_technical_phrases_match() {
    let t = rag::tokenize("上下文预算压缩：断点续跑");
    for want in ["上下", "下文", "文预", "预算", "断点", "续跑"] {
        assert!(t.iter().any(|x| x == want), "缺少 {want}: {t:?}");
    }
    // 单字也要留，短查询「树」才能命中
    assert!(t.iter().any(|x| x == "上"));
}

#[test]
fn single_letters_are_dropped_as_noise() {
    let t = rag::tokenize("a b x = 1");
    assert!(!t.iter().any(|x| x == "a"), "单字母是噪声: {t:?}");
    assert!(!t.iter().any(|x| x == "b"));
}

// ============ 分块 ============

#[test]
fn chunks_overlap_and_cover_every_line() {
    let text = (1..=130)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let cs = rag::chunk_text("a.rs", &text);
    assert!(cs.len() >= 3, "130 行应切成多块，实际 {}", cs.len());
    assert_eq!(cs[0].start, 1, "第一块从 1 开始");
    assert_eq!(cs.last().unwrap().end, 130, "最后一块要盖到最后一行");
    for w in cs.windows(2) {
        assert_eq!(
            w[1].start,
            w[0].start + rag::CHUNK_LINES - rag::CHUNK_OVERLAP,
            "相邻块应按重叠步进"
        );
        assert!(w[1].start <= w[0].end, "重叠区不能断开");
    }
}

#[test]
fn empty_file_yields_no_chunks() {
    assert!(rag::chunk_text("a.rs", "").is_empty());
}

// ============ 打分与重排 ============

#[test]
fn path_hit_outranks_equal_content() {
    // 两段内容完全相同：唯一差别是路径命中查询词。
    // 查询同时包含内容词，保证两段都有正分——路径只是加分项，
    // 不能让「内容完全无关」的块凭空上榜。
    let body = "// budget 的一些无关说明\n// 再补一行，让词数稳定";
    let hits = rag::rank(
        vec![chunk("src/other.rs", 1, body), chunk("src/state.rs", 1, body)],
        &["budget".to_string(), "state".to_string()],
        2,
    );
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].path, "src/state.rs", "路径命中应排前");
    assert!(hits[0].score > hits[1].score);
}

#[test]
fn hit_in_first_lines_outranks_hit_at_tail() {
    // 开头 5 行命中多半是定义处，比文件末尾的引用更有用
    let head = "// state\n// a\n// b\n// c\n// d\n// e";
    let tail = "// a\n// b\n// c\n// d\n// e\n// state";
    let hits = rag::rank(
        vec![chunk("tail.rs", 1, tail), chunk("head.rs", 1, head)],
        &["state".to_string()],
        2,
    );
    assert_eq!(hits[0].path, "head.rs", "{:?}", hits);
}

#[test]
fn irrelevant_chunks_are_dropped_not_zero_scored() {
    let hits = rag::rank(
        vec![
            chunk("a.rs", 1, "let budget = 1;"),
            chunk("b.rs", 1, "// 完全无关"),
        ],
        &["budget".to_string()],
        5,
    );
    assert_eq!(hits.len(), 1, "零分块不该占位置: {hits:?}");
    assert_eq!(hits[0].path, "a.rs");
}

#[test]
fn top_k_is_clamped_to_hard_limit() {
    let chunks: Vec<Chunk> = (0..50)
        .map(|i| chunk(&format!("f{i}.rs"), 1, "// needle"))
        .collect();
    let hits = rag::rank(chunks, &["needle".to_string()], 1_000);
    assert_eq!(hits.len(), rag::MAX_TOP_K, "再大也不超过上限");
}

#[test]
fn empty_query_returns_nothing_instead_of_dumping_everything() {
    let (ctx, dir) = ws("empty");
    write(&dir.join("a.txt"), "anything");
    assert!(rag::search(&ctx.root, "", 8).is_empty());
    assert!(rag::search(&ctx.root, "   ", 8).is_empty());
    assert!(rag::search(&ctx.root, "!!! ???", 8).is_empty());
}

// ============ 索引范围 ============

#[test]
fn skipped_dirs_and_binaries_are_not_indexed() {
    let (ctx, dir) = ws("idx");
    write(&dir.join("src/ok.rs"), "// SERIALIZE_NEEDLE");
    write(&dir.join("target/gen.rs"), "// SERIALIZE_NEEDLE");
    write(&dir.join("node_modules/pkg/i.js"), "// SERIALIZE_NEEDLE");
    // 工具链缓存/产物：不是源码，却常含大量重复字串
    write(&dir.join(".venv/lib/x.py"), "# SERIALIZE_NEEDLE");
    write(&dir.join("logs/app.log"), "SERIALIZE_NEEDLE");
    write(&dir.join(".idea/workspace.xml"), "<x>SERIALIZE_NEEDLE</x>");
    let mut blob = vec![0u8; 512];
    blob.extend_from_slice(b"SERIALIZE_NEEDLE");
    std::fs::write(dir.join("blob.bin"), &blob).unwrap();

    let hits = rag::search(&ctx.root, "SERIALIZE_NEEDLE", 10);
    assert!(!hits.is_empty(), "至少要找到 src/ok.rs");
    assert!(
        hits.iter().all(|h| h.path == "src/ok.rs"),
        "生成目录与二进制不该入库: {:?}",
        hits.iter().map(|h| &h.path).collect::<Vec<_>>()
    );
}

#[test]
fn single_line_data_files_do_not_outrank_real_code() {
    // 真实踩过：工作区里残留的会话 JSON 是单行 minified，里面存着用户原话，
    // 于是 search_code 的第一名是它自己，而不是实现代码。
    let (ctx, dir) = ws("minified");
    let mut blob = String::from("{\"sessions\":[");
    for i in 0..400 {
        blob.push_str(&format!(
            "{{\"id\":{i},\"body\":\"上下文预算压缩在哪里实现\"}},"
        ));
    }
    blob.push_str("{}]}");
    write(&dir.join("dump.json"), &blob);
    write(
        &dir.join("src/budget.rs"),
        "// 上下文预算压缩：超过上限就停\nfn check() {}\n",
    );

    let hits = rag::search(&ctx.root, "上下文预算压缩", 10);
    assert!(!hits.is_empty(), "真代码要能被搜到");
    assert!(
        hits.iter().all(|h| h.path != "dump.json"),
        "单行数据文件不该入库: {:?}",
        hits.iter().map(|h| &h.path).collect::<Vec<_>>()
    );
    assert_eq!(hits[0].path, "src/budget.rs");
}

#[test]
fn paths_use_forward_slashes() {
    let (ctx, dir) = ws("slash");
    write(&dir.join("a/b/c.rs"), "// SLASH_NEEDLE");
    let hits = rag::search(&ctx.root, "SLASH_NEEDLE", 3);
    assert_eq!(hits[0].path, "a/b/c.rs", "引用要跨平台一致");
}

// ============ 引用 ============

#[test]
fn line_numbers_point_at_the_real_line() {
    let mut text = String::new();
    for i in 1..=70 {
        if i == 65 {
            text.push_str("// LINE65_NEEDLE\n");
        } else {
            text.push_str(&format!("// filler {i}\n"));
        }
    }
    let (ctx, dir) = ws("line");
    write(&dir.join("deep.rs"), &text);
    let hits = rag::search(&ctx.root, "LINE65_NEEDLE", 3);
    assert_eq!(hits.len(), 1);
    let h = &hits[0];
    assert!(
        h.start <= 65 && 65 <= h.end,
        "行号区间要盖住 65，实际 {}-{}",
        h.start,
        h.end
    );
    let rendered = rag::render("LINE65_NEEDLE", &hits, 1);
    assert!(rendered.contains("deep.rs:"), "{rendered}");
    assert!(rendered.contains("  65 | "), "渲染的行号要能对上：{rendered}");
}

#[test]
fn no_hit_message_suggests_next_step() {
    let out = rag::render("找不到的东西", &[], 12);
    assert!(out.contains("没有匹配"), "{out}");
    assert!(out.contains("search_files"), "要给出下一步：{out}");
}

#[test]
fn oversized_chunk_is_clipped_and_says_so() {
    let long = "x".repeat(rag::MAX_CHUNK_CHARS + 500);
    let hits = rag::rank(vec![chunk("big.rs", 1, &format!("needle {long}"))], &["needle".into()], 1);
    assert_eq!(hits.len(), 1);
    assert!(hits[0].truncated);
    assert!(hits[0].text.contains("已截断"), "要提示去看全文");
    assert!(hits[0].text.chars().count() < rag::MAX_CHUNK_CHARS + 100);
}

// ============ 工具契约 ============

#[test]
fn search_code_tool_returns_citations() {
    let (ctx, dir) = ws("tool");
    write(
        &dir.join("src/budget.rs"),
        "// 上下文预算：超过上限就停止调用工具\nfn check() {}\n",
    );
    let r = run(&ctx, "search_code", r#"{"query":"上下文预算"}"#);
    assert!(r.ok, "{}", r.content);
    assert!(r.content.contains("src/budget.rs:"), "要带引用：{}", r.content);
    assert!(r.content.contains("score"), "{}", r.content);
}

#[test]
fn search_code_without_query_is_bad_args() {
    let (ctx, _dir) = ws("badargs");
    let r = run(&ctx, "search_code", r#"{"top_k":5}"#);
    assert_eq!(r.error_code, Some("BAD_ARGS"), "{}", r.content);
}

#[test]
fn search_code_is_listed_and_does_not_need_approval() {
    let names: Vec<String> = tools::tool_specs().iter().map(|t| t.name.clone()).collect();
    assert!(names.iter().any(|n| n == "search_code"), "{names:?}");
    assert!(!tools::requires_approval("search_code"));
}
