//! 内嵌资源：给 gpui 的资源系统提供图标 SVG。
//!
//! gpui 默认的 `AssetSource` 是 `()`（永远返回 `None`），`svg()` 元素
//! 必须挂了自定义资源源才能取到文件。图标只有几个且要跟 exe 走，
//! 这里直接 `include_bytes!` 内嵌，不依赖启动时的当前目录。
//!
//! 渲染时 gpui 只取 SVG 的**蒙版**（alpha 通道），颜色由元素的
//! `text_color` 决定——所以 SVG 里写死白色不影响深浅主题。

use std::borrow::Cow;

/// 图标栏与界面内联使用的矢量图标。
pub struct EmbeddedAssets;

impl gpui::AssetSource for EmbeddedAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(match path {
            "icons/rail-chat.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/icons/rail-chat.svg")))
            }
            "icons/rail-team.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/icons/rail-team.svg")))
            }
            "icons/rail-skills.svg" => {
                Some(Cow::Borrowed(include_bytes!("../assets/icons/rail-skills.svg")))
            }
            _ => None,
        })
    }

    fn list(&self, _path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        Ok(Vec::new())
    }
}
