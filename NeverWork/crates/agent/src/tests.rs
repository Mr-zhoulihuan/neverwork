use crate::sse::for_each_data;

#[test]
fn parses_sse_data_lines() {
    let input = "data: {\"a\":1}\n\ndata: {\"b\":2}\n\ndata: [DONE]\n\n";
    let mut got = Vec::new();
    for_each_data(input.as_bytes(), |d| got.push(d.to_string()));
    assert_eq!(got, vec!["{\"a\":1}", "{\"b\":2}", "[DONE]"]);
}

#[test]
fn ignores_comments_and_empty() {
    let input = ": keepalive\n\ndata: hello\n";
    let mut got = Vec::new();
    for_each_data(input.as_bytes(), |d| got.push(d.to_string()));
    assert_eq!(got, vec!["hello"]);
}

// ---- 系统提示词：必须区分「问答」和「改代码」 ----

#[test]
fn prompt_tells_model_qa_needs_no_tools() {
    let p = crate::agent_loop::build_system_prompt("", &crate::tools::ToolContext::new("E:/go"));
    // 明确指出问答类诉求不要动工具
    assert!(p.contains("直接回答，不要动工具"), "缺少「问答不用工具」的指令");
    assert!(p.contains("不要读文件"), "缺少「不要读文件」的指令");
    // 明确给出判断依据
    assert!(p.contains("想要结果，还是想要信息"), "缺少区分类别的判据");
    // 工具部分必须限定只在动手类任务用
    assert!(p.contains("仅在 B 类任务时使用"), "工具应限定用于动手类任务");
}

#[test]
fn prompt_notes_missing_workspace() {
    let p = crate::agent_loop::build_system_prompt("", &crate::tools::ToolContext::unrestricted());
    assert!(p.contains("没有指定工作区"), "无工作区要明确告知模型");
    assert!(p.contains("绝对路径"), "无工作区时要允许绝对路径");
}

#[test]
fn prompt_still_documents_write_tools() {
    let ctx = crate::tools::ToolContext::new("E:/go");
    let p = crate::agent_loop::build_system_prompt("自定义人格", &ctx);
    // 改代码的能力不能因为强调问答而丢失
    assert!(p.contains("apply_patch"));
    assert!(p.contains("multi_patch"));
    assert!(p.contains("会弹给用户批准"), "写操作仍需说明要批准");
    // 人格与工作区都要带上。工作区按 `describe_root` 显示，
    // 不能硬编码 `E:/go`：目录真实存在时会被 canonicalize 改写成绝对路径。
    assert!(p.contains("自定义人格"));
    assert!(
        p.contains(&ctx.describe_root()),
        "提示词里没有带上工作区根: {}",
        ctx.describe_root()
    );
}

#[test]
fn prompt_does_not_demand_reading_before_answering() {
    // 旧提示词写的是「改文件之前先用 read_file」，被误读成「回答之前也要读」
    // 这里确认新提示词把读文件限定在动手类任务
    let p = crate::agent_loop::build_system_prompt("", &crate::tools::ToolContext::new("E:/go"));
    let occurrences = p.matches("read_file").count();
    assert!(occurrences >= 1);
    // 「B 类任务先 read_file」这个限定出现了
    assert!(p.contains("B 类任务先 read_file"));
}

#[test]
fn prompt_marks_remote_workspace() {
    let target = neverwork_core::SshTarget {
        host: "dev".into(),
        port: 22,
        root: "/srv/app".into(),
    };
    let p = crate::agent_loop::build_system_prompt("", &crate::tools::ToolContext::remote(target));
    assert!(p.contains("远程工作区"), "远程要明确告知模型");
    assert!(p.contains("dev:/srv/app"), "要带上远端根：{p}");
    assert!(p.contains("search_files"), "远程检索降级提示要给出替代");
}

// ---- 验证门禁：改完代码必须跑构建/测试 ----

#[test]
fn prompt_requires_verification_after_edits() {
    let p = crate::agent_loop::build_system_prompt("", &crate::tools::ToolContext::new("E:/go"));
    assert!(p.contains("改完代码必须验证"), "缺少「改完必须验证」的要求");
    assert!(p.contains("cargo test"), "要给出常见工具链的示例命令");
    assert!(
        p.contains("验证通过前不要声称「已完成」"),
        "验证之前不得声称完成"
    );
    assert!(p.contains("未验证"), "无法验证时要如实标注");
}
