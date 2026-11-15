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
    let p = crate::agent_loop::build_system_prompt("", std::path::Path::new("E:/go"));
    // 明确指出问答类诉求不要动工具
    assert!(p.contains("直接回答，不要动工具"), "缺少「问答不用工具」的指令");
    assert!(p.contains("不要读文件"), "缺少「不要读文件」的指令");
    // 明确给出判断依据
    assert!(p.contains("想要结果，还是想要信息"), "缺少区分类别的判据");
    // 工具部分必须限定只在动手类任务用
    assert!(p.contains("仅在 B 类任务时使用"), "工具应限定用于动手类任务");
}

#[test]
fn prompt_still_documents_write_tools() {
    let p = crate::agent_loop::build_system_prompt("自定义人格", std::path::Path::new("E:/go"));
    // 改代码的能力不能因为强调问答而丢失
    assert!(p.contains("apply_patch"));
    assert!(p.contains("multi_patch"));
    assert!(p.contains("会弹给用户批准"), "写操作仍需说明要批准");
    // 人格与工作区都要带上
    assert!(p.contains("自定义人格"));
    assert!(p.contains("E:/go"));
}

#[test]
fn prompt_does_not_demand_reading_before_answering() {
    // 旧提示词写的是「改文件之前先用 read_file」，被误读成「回答之前也要读」
    // 这里确认新提示词把读文件限定在动手类任务
    let p = crate::agent_loop::build_system_prompt("", std::path::Path::new("E:/go"));
    let occurrences = p.matches("read_file").count();
    assert!(occurrences >= 1);
    // 「B 类任务先 read_file」这个限定出现了
    assert!(p.contains("B 类任务先 read_file"));
}
