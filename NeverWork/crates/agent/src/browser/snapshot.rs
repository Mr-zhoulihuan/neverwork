//! 可访问性树（AXTree）→ 紧凑文本快照。
//!
//! 模型不看截图，靠这份文本「看见」页面：交互元素按 DOM 顺序编号成 `[n]`，
//! `browser_click` / `browser_type` 用编号回查 `backendDOMNodeId` 定位元素。
//! 编号只存在于内存映射里，换页/重新快照就重建。
//!
//! 预算策略：交互行永远保留（它们才是可操作对象），纯文本行在预算内尽量多留；
//! 任何一行被丢弃都会在末尾标注「已截断」，让模型知道还有内容没看到。

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// 默认快照预算（字符）。
pub const DEFAULT_LIMIT: usize = 10_000;
/// 上限：再大也塞不进上下文。
pub const MAX_LIMIT: usize = 20_000;
/// 单行文本上限，防止一篇文章把快照挤爆。
const LINE_CLAMP: usize = 200;

/// 交互角色：这些节点可以被点/输入，分配编号。
const INTERACTIVE: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "tab",
    "switch",
    "slider",
    "spinbutton",
    "option",
];

/// 一次快照的结果。
pub struct Snapshot {
    /// 给模型看的文本。
    pub text: String,
    /// 编号 → `backendDOMNodeId`（点击/输入时回查）。
    pub refs: HashMap<u32, i64>,
}

/// 从 `Accessibility.getFullAXTree` 的返回体构建快照。
pub fn build(ax: &Value, title: &str, url: &str, limit: usize) -> Snapshot {
    let limit = limit.clamp(500, MAX_LIMIT);
    let nodes = ax
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let by_id: HashMap<String, &Value> = nodes
        .iter()
        .filter_map(|n| {
            n.get("nodeId")
                .and_then(Value::as_str)
                .map(|id| (id.to_string(), n))
        })
        .collect();

    let mut out = String::new();
    out.push_str(&format!("页面：{title}（{url}）\n"));
    let mut refs = HashMap::new();
    let mut next_ref = 1u32;
    let mut budget = limit.saturating_sub(out.chars().count());
    let mut truncated = false;
    let mut visited: HashSet<String> = HashSet::new();

    if let Some(root) = nodes.first() {
        walk(
            root,
            &by_id,
            &mut out,
            &mut refs,
            &mut next_ref,
            &mut budget,
            &mut truncated,
            &mut visited,
        );
    }
    if out.trim_end().ends_with(&format!("页面：{title}（{url}）")) {
        out.push_str("（页面没有可读内容）\n");
    }
    if truncated {
        out.push_str("\n…（已截断：交互元素优先保留；可用更具体的 limit 重试）");
    }
    Snapshot { text: out, refs }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    node: &Value,
    by_id: &HashMap<String, &Value>,
    out: &mut String,
    refs: &mut HashMap<u32, i64>,
    next_ref: &mut u32,
    budget: &mut usize,
    truncated: &mut bool,
    visited: &mut HashSet<String>,
) {
    let id = node
        .get("nodeId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !id.is_empty() && !visited.insert(id.clone()) {
        return; // 防环
    }
    let ignored = node.get("ignored").and_then(Value::as_bool).unwrap_or(false);
    if !ignored {
        emit_node(node, out, refs, next_ref, budget, truncated);
    }
    if let Some(children) = node.get("childIds").and_then(Value::as_array) {
        for c in children {
            if let Some(cid) = c.as_str() {
                if let Some(child) = by_id.get(cid) {
                    walk(child, by_id, out, refs, next_ref, budget, truncated, visited);
                }
            }
        }
    }
}

fn emit_node(
    node: &Value,
    out: &mut String,
    refs: &mut HashMap<u32, i64>,
    next_ref: &mut u32,
    budget: &mut usize,
    truncated: &mut bool,
) {
    let role = node
        .get("role")
        .and_then(|r| r.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = node
        .get("name")
        .and_then(|r| r.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        // 许多容器节点没有名字（generic / none / list 等），递归仍会继续。
        return;
    }
    let name = clamp(name);
    let backend = node.get("backendDOMNodeId").and_then(Value::as_i64);
    let interactive = INTERACTIVE.contains(&role) && backend.is_some();

    let line = if interactive {
        let n = *next_ref;
        *next_ref += 1;
        refs.insert(n, backend.unwrap_or_default());
        format!("- {role} \"{name}\" [{n}]\n")
    } else {
        match role {
            "StaticText" | "text" | "InlineTextBox" | "paragraph" | "listitem" => {
                format!("{name}\n")
            }
            "heading" => format!("- heading \"{name}\"\n"),
            "image" => format!("- image \"{name}\"\n"),
            _ => format!("- {role} \"{name}\"\n"),
        }
    };
    let cost = line.chars().count();
    if !interactive && cost > *budget {
        *truncated = true;
        return;
    }
    *budget = budget.saturating_sub(cost);
    out.push_str(&line);
}

fn clamp(s: &str) -> String {
    if s.chars().count() <= LINE_CLAMP {
        return s.to_string();
    }
    let mut t: String = s.chars().take(LINE_CLAMP).collect();
    t.push('…');
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_tree() -> Value {
        json!({
            "nodes": [
                {"nodeId": "root", "ignored": false,
                 "role": {"value": "RootWebArea"}, "name": {"value": "登录页"},
                 "childIds": ["1", "2", "3", "4", "5", "6", "7"]},
                {"nodeId": "1", "ignored": false, "role": {"value": "heading"},
                 "name": {"value": "欢迎登录"}, "backendDOMNodeId": 11, "childIds": []},
                {"nodeId": "2", "ignored": false, "role": {"value": "textbox"},
                 "name": {"value": "用户名"}, "backendDOMNodeId": 12, "childIds": []},
                {"nodeId": "3", "ignored": false, "role": {"value": "textbox"},
                 "name": {"value": "密码"}, "backendDOMNodeId": 13, "childIds": []},
                {"nodeId": "4", "ignored": false, "role": {"value": "button"},
                 "name": {"value": "登录"}, "backendDOMNodeId": 14, "childIds": []},
                {"nodeId": "5", "ignored": true, "role": {"value": "button"},
                 "name": {"value": "隐藏按钮"}, "backendDOMNodeId": 15, "childIds": []},
                {"nodeId": "6", "ignored": false, "role": {"value": "StaticText"},
                 "name": {"value": "没有账号？"}, "backendDOMNodeId": 16, "childIds": []},
                {"nodeId": "7", "ignored": false, "role": {"value": "button"},
                 "name": {"value": ""}, "backendDOMNodeId": 17, "childIds": []}
            ]
        })
    }

    #[test]
    fn interactive_nodes_get_refs_in_order() {
        let s = build(&sample_tree(), "登录页", "http://localhost:3000/login", DEFAULT_LIMIT);
        assert!(s.text.contains("页面：登录页（http://localhost:3000/login）"));
        assert!(s.text.contains("textbox \"用户名\" [1]"), "{}", s.text);
        assert!(s.text.contains("textbox \"密码\" [2]"), "{}", s.text);
        assert!(s.text.contains("button \"登录\" [3]"), "{}", s.text);
        assert_eq!(s.refs.get(&1), Some(&12));
        assert_eq!(s.refs.get(&3), Some(&14));
    }

    #[test]
    fn ignored_and_empty_name_nodes_have_no_ref() {
        let s = build(&sample_tree(), "t", "u", DEFAULT_LIMIT);
        assert!(!s.text.contains("隐藏按钮"), "{}", s.text);
        assert_eq!(s.refs.len(), 3, "空名按钮不编号");
    }

    #[test]
    fn static_text_kept_as_plain_line() {
        let s = build(&sample_tree(), "t", "u", DEFAULT_LIMIT);
        assert!(s.text.contains("没有账号？"), "{}", s.text);
    }

    #[test]
    fn over_limit_marks_truncated_but_keeps_interactive() {
        let long = "很长的文本".repeat(400);
        let tree = json!({"nodes": [
            {"nodeId": "r", "ignored": false, "role": {"value": "RootWebArea"},
             "name": {"value": "页"}, "childIds": ["a", "b", "c", "d"]},
            {"nodeId": "a", "ignored": false, "role": {"value": "StaticText"},
             "name": {"value": long}, "backendDOMNodeId": 1, "childIds": []},
            {"nodeId": "b", "ignored": false, "role": {"value": "StaticText"},
             "name": {"value": long}, "backendDOMNodeId": 2, "childIds": []},
            {"nodeId": "c", "ignored": false, "role": {"value": "StaticText"},
             "name": {"value": long}, "backendDOMNodeId": 3, "childIds": []},
            {"nodeId": "d", "ignored": false, "role": {"value": "button"},
             "name": {"value": "提交"}, "backendDOMNodeId": 4, "childIds": []}
        ]});
        let s = build(&tree, "页", "http://x", 600);
        assert!(s.text.contains("已截断"), "{}", s.text);
        assert!(s.text.contains("button \"提交\" [1]"), "{}", s.text);
    }

    #[test]
    fn empty_page_note() {
        let tree = json!({"nodes": [
            {"nodeId": "r", "ignored": false, "role": {"value": "RootWebArea"},
             "name": {"value": ""}, "childIds": []}
        ]});
        let s = build(&tree, "空白", "http://x", DEFAULT_LIMIT);
        assert!(s.text.contains("没有可读内容"), "{}", s.text);
    }
}
