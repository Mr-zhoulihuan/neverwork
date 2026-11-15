//! 文本按像素宽度截断（自动补省略号）。
//!
//! 为什么不用 gpui 的 `.truncate()`：它依赖首帧测量时已有确定宽度，在
//! `flex_1` / 嵌套 flex 的行里常常拿不到确定宽度，结果文字被父级硬裁、
//! 不出现省略号（左侧会话卡片就是这样溢出到中栏的）。
//! 这里按字符宽度预算截断，结果稳定可测。

/// 估宽：中文/全角字符按 1 个字号宽，ASCII 按 0.56 个字号宽。
pub(crate) fn char_width(ch: char, font_px: f32) -> f32 {
    let code = ch as u32;
    let wide = matches!(code,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD);
    if wide {
        font_px
    } else {
        // 实测 Microsoft YaHei UI 下 ASCII 更接近 0.5 个字号宽；0.56 会低估可用字数。
        font_px * 0.5
    }
}

/// 等宽字体里 ASCII 字符的前进宽度占字号的比例。
pub(crate) const MONO_ASCII_RATIO: f32 = 0.6;

/// 同 [`char_width`]，但按等宽字体的前进宽度估算。
pub(crate) fn char_width_mono(ch: char, font_px: f32) -> f32 {
    let code = ch as u32;
    let wide = matches!(code,
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD);
    if wide {
        // 等宽字体里的 CJK 回退字形通常占两个 ASCII 位
        font_px * 2.0 * MONO_ASCII_RATIO
    } else {
        font_px * MONO_ASCII_RATIO
    }
}

/// 把 `text` 截到不超过 `max_px` 宽，必要时以 `…` 结尾。
/// `reserve` 是省略号预留宽度（传 0 表示刚好卡满）。
pub fn fit_text(text: &str, max_px: f32, font_px: f32) -> String {
    if max_px <= 0.0 {
        return String::new();
    }
    let total: f32 = text.chars().map(|c| char_width(c, font_px)).sum();
    if total <= max_px {
        return text.to_string();
    }
    let ellipsis_w = char_width('…', font_px);
    let budget = (max_px - ellipsis_w).max(0.0);
    let mut used = 0.0;
    let mut out = String::new();
    for ch in text.chars() {
        let w = char_width(ch, font_px);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(ch);
    }
    // 避免把首字也吃掉后只剩省略号。
    if out.is_empty() && !text.is_empty() {
        if let Some(first) = text.chars().next() {
            out.push(first);
        }
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_unchanged() {
        assert_eq!(fit_text("重构支付回调", 200.0, 14.0), "重构支付回调");
    }

    #[test]
    fn long_chinese_gets_ellipsis() {
        let s = "每天 07:30 搜索 30 条今日时间线并发送微信";
        let out = fit_text(s, 188.0, 14.0);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() < s.chars().count());
        // 估算宽度不能超过预算太多。
        let w: f32 = out.chars().map(|c| char_width(c, 14.0)).sum();
        assert!(w <= 195.0, "宽度 {} 超预算", w);
    }

    #[test]
    fn ascii_is_narrower_than_cjk() {
        assert!(char_width('a', 14.0) < char_width('中', 14.0));
    }

    #[test]
    fn zero_width_returns_empty() {
        assert_eq!(fit_text("abc", 0.0, 14.0), "");
    }

    #[test]
    fn never_empty_for_nonempty_input() {
        let out = fit_text("abcdef", 3.0, 14.0);
        assert!(!out.is_empty());
    }
}
