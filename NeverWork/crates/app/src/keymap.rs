//! 键盘命令路由：把 GPUI 的 KeyStroke 翻译成编辑动作。
//!
//! 关键约束：**文本插入不在这里处理**。可打印字符一律交给
//! `EntityInputHandler`（`replace_text_in_range`），否则会和 IME（中文输入法）
//! 重复插入，造成「打一个字出现两个」或丢失。本模块只负责非文本按键。

use gpui::Keystroke;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    Home,
    End,
    DocStart,
    DocEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// 光标移动；`extend` 表示按住 Shift 扩展选区。
    Motion { motion: Motion, extend: bool },
    Backspace,
    Delete,
    SelectAll,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    Save,
    /// Ctrl+F：打开查找。
    Find,
    /// Ctrl+H：打开查找并聚焦替换框。
    Replace,
    /// 回车：对话里是发送，编辑器里是换行。
    Enter,
    /// Tab / Shift+Tab：编辑器里缩进/反缩进。
    Tab { shift: bool },
    Escape,
}

/// 把按键翻译为编辑动作。返回 `None` 表示应交给输入法处理（可打印字符）。
pub fn route(ks: &Keystroke) -> Option<KeyAction> {
    let ctrl = ks.modifiers.control || ks.modifiers.platform;
    let shift = ks.modifiers.shift;
    let key = ks.key.as_str();

    if ctrl {
        return match key {
            "s" => Some(KeyAction::Save),
            "f" => Some(KeyAction::Find),
            "h" => Some(KeyAction::Replace),
            "a" => Some(KeyAction::SelectAll),
            "z" => Some(if shift { KeyAction::Redo } else { KeyAction::Undo }),
            "y" => Some(KeyAction::Redo),
            "c" => Some(KeyAction::Copy),
            "x" => Some(KeyAction::Cut),
            "v" => Some(KeyAction::Paste),
            "left" => Some(KeyAction::Motion { motion: Motion::WordLeft, extend: shift }),
            "right" => Some(KeyAction::Motion { motion: Motion::WordRight, extend: shift }),
            "home" => Some(KeyAction::Motion { motion: Motion::DocStart, extend: shift }),
            "end" => Some(KeyAction::Motion { motion: Motion::DocEnd, extend: shift }),
            "up" => Some(KeyAction::Motion { motion: Motion::DocStart, extend: shift }),
            "down" => Some(KeyAction::Motion { motion: Motion::DocEnd, extend: shift }),
            _ => None,
        };
    }

    match key {
        "backspace" => Some(KeyAction::Backspace),
        "delete" => Some(KeyAction::Delete),
        "enter" => Some(KeyAction::Enter),
        "tab" => Some(KeyAction::Tab { shift }),
        "escape" => Some(KeyAction::Escape),
        "left" => Some(KeyAction::Motion { motion: Motion::Left, extend: shift }),
        "right" => Some(KeyAction::Motion { motion: Motion::Right, extend: shift }),
        "up" => Some(KeyAction::Motion { motion: Motion::Up, extend: shift }),
        "down" => Some(KeyAction::Motion { motion: Motion::Down, extend: shift }),
        "home" => Some(KeyAction::Motion { motion: Motion::Home, extend: shift }),
        "end" => Some(KeyAction::Motion { motion: Motion::End, extend: shift }),
        // 可打印字符：交给输入法。
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn ks(key: &str, control: bool, shift: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control,
                shift,
                ..Default::default()
            },
            key: key.into(),
            key_char: None,
        }
    }

    #[test]
    fn printable_chars_go_to_ime() {
        assert_eq!(route(&ks("a", false, false)), None);
        assert_eq!(route(&ks("中", false, false)), None);
        assert_eq!(route(&ks("space", false, false)), None);
    }

    #[test]
    fn shortcuts() {
        assert_eq!(route(&ks("s", true, false)), Some(KeyAction::Save));
        assert_eq!(route(&ks("a", true, false)), Some(KeyAction::SelectAll));
        assert_eq!(route(&ks("z", true, false)), Some(KeyAction::Undo));
        assert_eq!(route(&ks("z", true, true)), Some(KeyAction::Redo));
        assert_eq!(route(&ks("y", true, false)), Some(KeyAction::Redo));
        assert_eq!(route(&ks("c", true, false)), Some(KeyAction::Copy));
        assert_eq!(route(&ks("f", true, false)), Some(KeyAction::Find));
        assert_eq!(route(&ks("h", true, false)), Some(KeyAction::Replace));
    }

    #[test]
    fn motion_and_extend() {
        assert_eq!(
            route(&ks("left", false, true)),
            Some(KeyAction::Motion { motion: Motion::Left, extend: true })
        );
        assert_eq!(
            route(&ks("left", true, false)),
            Some(KeyAction::Motion { motion: Motion::WordLeft, extend: false })
        );
        assert_eq!(route(&ks("enter", false, false)), Some(KeyAction::Enter));
    }
}
