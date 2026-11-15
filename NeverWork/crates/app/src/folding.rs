//! 代码折叠：按缩进层级判断可折叠的块。
//!
//! 不引 tree-sitter 的理由：那个依赖让构建慢一个量级，而我们只需要
//! 「把一段连续缩进的行收起来」这一个能力。缩进法对 Python / YAML / Rust
//! 带花括号的块都够用，代价是极端风格（同行多块、全平铺）识别不出，
//! 那时只是少一个折叠点，不影响编辑。
//!
//! 折叠状态用「起始行号」表示。行号在编辑后会失效，所以每次同步内容时
//! 用 `reconcile` 把失效的折叠点丢掉，避免折叠错位。

use std::collections::BTreeSet;

/// 从行文本集合里找出所有可折叠区域的起始行（0 基）。
///
/// 判定：某行以 `{` / `:` / `(` 等收尾，或它的缩进小于后续行的缩进
/// 且后续有更多缩进的行，则这一行是一个可折叠块的起点。
pub fn foldable_starts(lines: &[&str]) -> Vec<usize> {
    let mut out = Vec::new();
    let n = lines.len();
    if n < 2 {
        return out;
    }

    for i in 0..n - 1 {
        let cur = lines[i];
        if cur.trim().is_empty() {
            continue;
        }
        let cur_indent = indent_of(cur);
        // 找下一条非空行
        let Some(next) = (i + 1..n).find(|&j| !lines[j].trim().is_empty()) else {
            break;
        };
        let next_indent = indent_of(lines[next]);

        let grows = next_indent > cur_indent;
        // 括号/冒号收尾也算（支持 `fn main() {` 这种缩进不变的内容）
        let trimmed = cur.trim_end();
        let opens = trimmed.ends_with('{')
            || trimmed.ends_with('(')
            || trimmed.ends_with('[')
            || trimmed.ends_with(':');

        if grows || (opens && next_indent >= cur_indent) {
            // 至少要覆盖到下一行，否则折叠没意义
            if next > i {
                out.push(i);
            }
        }
    }
    out
}

/// 一行前导空格的视觉宽度（tab 记 4）。
pub fn indent_of(line: &str) -> usize {
    let mut w = 0;
    for c in line.chars() {
        match c {
            ' ' => w += 1,
            '\t' => w += 4,
            _ => break,
        }
    }
    w
}

/// 折叠区的结束行（不含）。
///
/// 从 `start` 往下走，直到遇到缩进 <= 起始行缩进的非空行为止。
/// 若起始行以括号收尾，则用括号配平收尾（更准）。
pub fn fold_end(lines: &[&str], start: usize) -> usize {
    if start >= lines.len() {
        return lines.len();
    }
    let base = indent_of(lines[start]);
    let trimmed = lines[start].trim_end();

    // 括号法优先：能正确处理 `}` 与起始行同缩进的情况
    if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
        let (open, close) = if trimmed.ends_with('{') {
            ('{', '}')
        } else if trimmed.ends_with('(') {
            ('(', ')')
        } else {
            ('[', ']')
        };
        let mut depth = 0i32;
        for (i, line) in lines.iter().enumerate().skip(start) {
            for c in line.chars() {
                if c == open {
                    depth += 1;
                } else if c == close {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
            }
        }
        return lines.len();
    }

    // 缩进法
    let mut end = start + 1;
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim().is_empty() {
            end = i + 1;
            continue;
        }
        if indent_of(line) <= base {
            return i;
        }
        end = i + 1;
    }
    end
}

/// 每次内容变化后清理失效的折叠点。
pub fn reconcile(folds: &mut BTreeSet<usize>, lines: &[&str]) {
    let valid: BTreeSet<usize> = foldable_starts(lines).into_iter().collect();
    folds.retain(|k| valid.contains(k));
}

/// 某个折叠区被收起时，从可见行序列里要跳过哪些行。
///
/// 返回 (起始行, 结束行) 列表，供渲染时隐藏。
pub fn collapsed_ranges(folds: &BTreeSet<usize>, lines: &[&str]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for &start in folds {
        let end = fold_end(lines, start);
        if end > start + 1 {
            out.push((start + 1, end));
        }
    }
    out
}

/// 某一行是否被某个折叠区隐藏。
pub fn is_hidden(line: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|(s, e)| line >= *s && line < *e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<&str> {
        s.split('\n').collect()
    }

    #[test]
    fn finds_brace_block() {
        let src = "fn main() {\n    let a = 1;\n    let b = 2;\n}\nlet c = 3;";
        let l = lines(src);
        let starts = foldable_starts(&l);
        assert!(starts.contains(&0), "fn main 块应可折叠: {starts:?}");
    }

    #[test]
    fn fold_end_matches_closing_brace() {
        let src = "fn main() {\n    let a = 1;\n}\nlet c = 3;";
        let l = lines(src);
        // 第 0 行开始，到第 3 行（不含）结束 → 覆盖 0..3
        assert_eq!(fold_end(&l, 0), 3);
    }

    #[test]
    fn nested_blocks_each_foldable() {
        let src = "fn a() {\n    if x {\n        y();\n    }\n}\n";
        let l = lines(src);
        let starts = foldable_starts(&l);
        assert!(starts.contains(&0));
        assert!(starts.contains(&1), "内部 if 也应可折叠: {starts:?}");
    }

    #[test]
    fn python_indent_block() {
        let src = "def f():\n    a = 1\n    b = 2\n\nx = 3";
        let l = lines(src);
        let starts = foldable_starts(&l);
        assert!(starts.contains(&0), "python def 应可折叠: {starts:?}");
        // 结束在缩进回到 0 的那一行
        assert_eq!(fold_end(&l, 0), 4);
    }

    #[test]
    fn flat_code_has_no_folds() {
        let src = "let a = 1;\nlet b = 2;\nlet c = 3;";
        let l = lines(src);
        assert!(foldable_starts(&l).is_empty());
    }

    #[test]
    fn empty_and_single_line_safe() {
        assert!(foldable_starts(&[]).is_empty());
        assert!(foldable_starts(&["a"]).is_empty());
        assert!(foldable_starts(&[""]).is_empty());
    }

    #[test]
    fn indent_counts_tabs_as_four() {
        assert_eq!(indent_of("\tx"), 4);
        assert_eq!(indent_of("    x"), 4);
        assert_eq!(indent_of("x"), 0);
        assert_eq!(indent_of("  \t x"), 7);
    }

    #[test]
    fn collapsed_ranges_cover_body() {
        let src = "fn main() {\n    a();\n    b();\n}\nafter();";
        let l = lines(src);
        let mut folds = BTreeSet::new();
        folds.insert(0usize);
        let ranges = collapsed_ranges(&folds, &l);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (1, 4), "隐藏 1..4 行");
        assert!(is_hidden(1, &ranges));
        assert!(is_hidden(3, &ranges));
        assert!(!is_hidden(0, &ranges), "起始行本身要显示");
        assert!(!is_hidden(4, &ranges), "结束后的行要显示");
    }

    #[test]
    fn reconcile_drops_stale_folds() {
        let src = "fn main() {\n    a();\n}\n";
        let l = lines(src);
        let mut folds = BTreeSet::new();
        folds.insert(0usize);
        folds.insert(99usize); // 失效
        reconcile(&mut folds, &l);
        assert_eq!(folds.len(), 1);
        assert!(folds.contains(&0));
    }

    #[test]
    fn reconcile_keeps_after_edit_that_preserves_block() {
        let l1 = lines("fn main() {\n    a();\n}\n");
        let mut folds = BTreeSet::new();
        folds.insert(0usize);
        reconcile(&mut folds, &l1);
        // 内容改了一行但仍是一个块
        let l2 = lines("fn main() {\n    b();\n    c();\n}\n");
        reconcile(&mut folds, &l2);
        assert_eq!(folds.len(), 1, "块结构没变，折叠应保留");
    }

    #[test]
    fn fold_end_without_braces_uses_indent() {
        let src = "root:\n  a: 1\n  b: 2\nother: 3";
        let l = lines(src);
        assert_eq!(fold_end(&l, 0), 3);
    }
}
