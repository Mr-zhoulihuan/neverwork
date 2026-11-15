//! 行级 diff：算出「改了什么」，供审批界面展示。
//!
//! 策略：先做前缀/后缀修剪（绝大多数真实编辑只动一小段，这步就能把规模压到很小），
//! 再对中间剩下的部分跑 LCS。规模过大时退化成「整段替换」，
//! 宁可展示粗一点，也不让 UI 卡住。

use serde::{Deserialize, Serialize};

/// 一行的变化类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    Same,
    Add,
    Remove,
}

/// diff 中的一行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffKind,
    /// 旧文件行号（1 基）；新增行为 None。
    pub old_no: Option<usize>,
    /// 新文件行号（1 基）；删除行为 None。
    pub new_no: Option<usize>,
    pub text: String,
}

impl DiffLine {
    /// 前缀字符：`+` / `-` / 空格。
    pub fn marker(&self) -> char {
        match self.kind {
            DiffKind::Add => '+',
            DiffKind::Remove => '-',
            DiffKind::Same => ' ',
        }
    }
}

/// diff 结果概要。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiffStats {
    pub added: usize,
    pub removed: usize,
    /// 是否有变化。
    pub changed: bool,
}

/// LCS 单边最大规模（中间段行数乘积上限）。
/// 超过就退化成整段替换，避免 O(n*m) 把后台线程拖死。
const LCS_BUDGET: usize = 400_000;

/// 计算两个文本的行级 diff。
/// 按行切分。
///
/// 空字符串是 **0 行**（不是 1 个空行）——否则 `""` 与 `"a\nb"` 的 diff
/// 会凭空多出一条「删掉空行」，看起来像改了东西。末尾换行仍算一个空行，
/// 这样「只加一个结尾换行」也能被检出。
fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split('\n').collect()
    }
}

pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = split_lines(old);
    let b: Vec<&str> = split_lines(new);

    // 1) 前缀修剪
    let mut head = 0usize;
    while head < a.len() && head < b.len() && a[head] == b[head] {
        head += 1;
    }
    // 2) 后缀修剪
    let mut tail = 0usize;
    while tail < a.len() - head && tail < b.len() - head && a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
    {
        tail += 1;
    }

    let mut out: Vec<DiffLine> = Vec::new();
    for i in 0..head {
        out.push(DiffLine {
            kind: DiffKind::Same,
            old_no: Some(i + 1),
            new_no: Some(i + 1),
            text: a[i].to_string(),
        });
    }

    let a_mid = &a[head..a.len() - tail];
    let b_mid = &b[head..b.len() - tail];

    if a_mid.len() * b_mid.len() > LCS_BUDGET {
        // 太大：整段替换
        for (k, line) in a_mid.iter().enumerate() {
            out.push(DiffLine {
                kind: DiffKind::Remove,
                old_no: Some(head + k + 1),
                new_no: None,
                text: line.to_string(),
            });
        }
        for (k, line) in b_mid.iter().enumerate() {
            out.push(DiffLine {
                kind: DiffKind::Add,
                old_no: None,
                new_no: Some(head + k + 1),
                text: line.to_string(),
            });
        }
    } else {
        for (kind, ai, bi) in lcs_pairs(a_mid, b_mid) {
            match kind {
                DiffKind::Same => out.push(DiffLine {
                    kind,
                    old_no: Some(head + ai + 1),
                    new_no: Some(head + bi + 1),
                    text: a_mid[ai].to_string(),
                }),
                DiffKind::Remove => out.push(DiffLine {
                    kind,
                    old_no: Some(head + ai + 1),
                    new_no: None,
                    text: a_mid[ai].to_string(),
                }),
                DiffKind::Add => out.push(DiffLine {
                    kind,
                    old_no: None,
                    new_no: Some(head + bi + 1),
                    text: b_mid[bi].to_string(),
                }),
            }
        }
    }

    for k in 0..tail {
        let oi = a.len() - tail + k;
        let ni = b.len() - tail + k;
        out.push(DiffLine {
            kind: DiffKind::Same,
            old_no: Some(oi + 1),
            new_no: Some(ni + 1),
            text: a[oi].to_string(),
        });
    }

    out
}

/// 对两段做 LCS，返回有序的 (类型, a 下标, b 下标)。
fn lcs_pairs(a: &[&str], b: &[&str]) -> Vec<(DiffKind, usize, usize)> {
    let n = a.len();
    let m = b.len();
    if n == 0 && m == 0 {
        return Vec::new();
    }
    // dp[i][j] = a[i..] 与 b[j..] 的 LCS 长度
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut out = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((DiffKind::Same, i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push((DiffKind::Remove, i, j));
            i += 1;
        } else {
            out.push((DiffKind::Add, i, j));
            j += 1;
        }
    }
    while i < n {
        out.push((DiffKind::Remove, i, j));
        i += 1;
    }
    while j < m {
        out.push((DiffKind::Add, i, j));
        j += 1;
    }
    out
}

/// 统计增删行数。
pub fn stats(lines: &[DiffLine]) -> DiffStats {
    let mut s = DiffStats::default();
    for l in lines {
        match l.kind {
            DiffKind::Add => s.added += 1,
            DiffKind::Remove => s.removed += 1,
            DiffKind::Same => {}
        }
    }
    s.changed = s.added > 0 || s.removed > 0;
    s
}

/// 只保留变化处及其上下文，减少展示体积。
///
/// `context` 是变化前后各保留多少行。
pub fn with_context(lines: &[DiffLine], context: usize) -> Vec<DiffLine> {
    let changes: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind != DiffKind::Same)
        .map(|(i, _)| i)
        .collect();
    if changes.is_empty() {
        return Vec::new();
    }

    let mut keep = vec![false; lines.len()];
    for &c in &changes {
        let lo = c.saturating_sub(context);
        let hi = (c + context + 1).min(lines.len());
        for k in lo..hi {
            keep[k] = true;
        }
    }

    let mut out = Vec::new();
    let mut skipped = false;
    for (i, line) in lines.iter().enumerate() {
        if keep[i] {
            if skipped {
                out.push(DiffLine {
                    kind: DiffKind::Same,
                    old_no: None,
                    new_no: None,
                    text: "…".to_string(),
                });
                skipped = false;
            }
            out.push(line.clone());
        } else {
            skipped = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_has_no_changes() {
        let d = diff_lines("a\nb\nc", "a\nb\nc");
        let s = stats(&d);
        assert!(!s.changed);
        assert_eq!(s.added, 0);
        assert_eq!(s.removed, 0);
        assert_eq!(d.len(), 3);
        assert!(d.iter().all(|l| l.kind == DiffKind::Same));
    }

    #[test]
    fn single_line_replacement() {
        let d = diff_lines("a\nb\nc", "a\nB\nc");
        let s = stats(&d);
        assert!(s.changed);
        assert_eq!(s.added, 1);
        assert_eq!(s.removed, 1);
        // 前缀 a 与后缀 c 应保持 Same
        let same_count = d.iter().filter(|l| l.kind == DiffKind::Same).count();
        assert_eq!(same_count, 2);
    }

    #[test]
    fn append_at_end() {
        let d = diff_lines("a\nb", "a\nb\nc");
        let s = stats(&d);
        assert_eq!(s.added, 1);
        assert_eq!(s.removed, 0);
    }

    #[test]
    fn delete_from_middle() {
        let d = diff_lines("a\nb\nc\nd", "a\nd");
        let s = stats(&d);
        assert_eq!(s.removed, 2);
        assert_eq!(s.added, 0);
        assert!(!s.changed == false);
    }

    #[test]
    fn line_numbers_are_one_based_and_consistent() {
        let d = diff_lines("x\nold\ny", "x\nnew\ny");
        let removes: Vec<_> = d.iter().filter(|l| l.kind == DiffKind::Remove).collect();
        let adds: Vec<_> = d.iter().filter(|l| l.kind == DiffKind::Add).collect();
        assert_eq!(removes.len(), 1);
        assert_eq!(adds.len(), 1);
        assert_eq!(removes[0].old_no, Some(2));
        assert_eq!(removes[0].new_no, None);
        assert_eq!(adds[0].old_no, None);
        assert_eq!(adds[0].new_no, Some(2));
    }

    #[test]
    fn from_empty_file() {
        let d = diff_lines("", "a\nb");
        let s = stats(&d);
        assert!(s.added >= 2);
        assert_eq!(s.removed, 0);
    }

    #[test]
    fn to_empty_file() {
        let d = diff_lines("a\nb", "");
        let s = stats(&d);
        assert!(s.removed >= 2);
        assert_eq!(s.added, 0);
    }

    #[test]
    fn marker_chars() {
        let d = diff_lines("a", "b");
        let markers: String = d.iter().map(|l| l.marker()).collect();
        assert!(markers.contains('-'));
        assert!(markers.contains('+'));
    }

    #[test]
    fn with_context_collapses_far_unchanged() {
        // 20 行，只有第 10 行变化
        let old: String = (0..20).map(|i| format!("l{i}\n")).collect();
        let mut lines: Vec<&str> = old.split('\n').collect();
        lines[9] = "CHANGED";
        let new = lines.join("\n");
        let d = diff_lines(&old, &new);
        let compact = with_context(&d, 1);
        assert!(compact.len() < d.len(), "应折叠未变化部分");
        assert!(compact.iter().any(|l| l.text == "…"));
        assert!(compact.iter().any(|l| l.text == "CHANGED"));
    }

    #[test]
    fn with_context_empty_when_unchanged() {
        let d = diff_lines("a\nb", "a\nb");
        assert!(with_context(&d, 2).is_empty());
    }

    #[test]
    fn huge_change_falls_back_without_hanging() {
        // 两侧各 1000 行且完全不同：乘积 100 万 > 预算
        let old: String = (0..1000).map(|i| format!("a{i}\n")).collect();
        let new: String = (0..1000).map(|i| format!("b{i}\n")).collect();
        let d = diff_lines(&old, &new);
        let s = stats(&d);
        assert!(s.added > 0 && s.removed > 0);
        assert!(d.len() < 2100);
    }
}
