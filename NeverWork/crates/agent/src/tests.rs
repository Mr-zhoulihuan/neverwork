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
