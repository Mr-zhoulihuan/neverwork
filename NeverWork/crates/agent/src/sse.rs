//! 极简 SSE（Server-Sent Events）解析：从字节流按行读取 `data:` 负载。

use std::io::{BufRead, BufReader, Read};

/// 从 reader 逐行读取 SSE，每遇到 `data: xxx` 行就回调 `on_data(payload)`。
/// `[DONE]` 视为结束信号（回调 on_data 后返回）。
pub fn for_each_data<R: Read>(reader: R, mut on_data: impl FnMut(&str)) {
    let buf = BufReader::new(reader);
    for line in buf.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            let data = rest.trim();
            if data == "[DONE]" {
                on_data("[DONE]");
                break;
            }
            on_data(data);
        }
        // event: / id: 等其它字段忽略
    }
}
