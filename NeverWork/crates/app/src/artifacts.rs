//! 对话产物（Agent 写入的文件、生成的图片）的定位与落盘。
//!
//! 模型用 Markdown 图片语法给出的图有三种来源：
//! - 本地绝对路径（它刚写的文件）；
//! - 工作区相对路径；
//! - `data:image/...;base64,...`（部分模型/兼容服务直接把图画进正文）。
//!
//! 前两种直接复用；data URI 解码一次缓存到 `<配置目录>/artifacts/`，
//! 渲染与产物列表都拿这个缓存文件，避免每帧重新解码几十 KB 的 base64。

use std::path::{Path, PathBuf};

/// data URI 落盘目录：`<配置目录>/artifacts`。
pub fn cache_dir() -> PathBuf {
    neverwork_core::config::config_dir().join("artifacts")
}

/// 把 `data:image/...;base64,...` 解码成缓存文件，返回路径。
///
/// 文件名是内容哈希：同一张图重复渲染/登记只写一次盘。非 data URI 返回 `None`。
pub fn data_uri_to_file(src: &str) -> Option<PathBuf> {
    let rest = src.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if !meta.to_ascii_lowercase().contains("base64") {
        return None;
    }
    let mime = meta.split(';').next().unwrap_or("");
    let ext = mime_ext(mime)?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&bytes, &mut hasher);
    let hash = std::hash::Hasher::finish(&hasher);
    let dir = cache_dir();
    let path = dir.join(format!("img_{hash:016x}.{ext}"));
    if !path.exists() {
        std::fs::create_dir_all(&dir).ok()?;
        std::fs::write(&path, &bytes).ok()?;
    }
    Some(path)
}

/// 把 Markdown 图片地址解析成可渲染的本地文件。
///
/// 网络图片返回 `None`（没有下载器，调用方渲染成链接占位）。
pub fn resolve_image(src: &str, root: Option<&Path>) -> Option<PathBuf> {
    let src = src.trim();
    if src.starts_with("data:") {
        return data_uri_to_file(src);
    }
    if src.starts_with("http://") || src.starts_with("https://") {
        return None;
    }
    let p = Path::new(src);
    if p.is_absolute() && p.exists() {
        return Some(p.to_path_buf());
    }
    if let Some(root) = root {
        let joined = root.join(src);
        if joined.exists() {
            return Some(joined);
        }
    }
    None
}

fn mime_ext(mime: &str) -> Option<&'static str> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    }
}
