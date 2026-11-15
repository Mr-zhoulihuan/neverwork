//! 真实文件系统操作：目录遍历（带忽略规则与按需展开）、文件读取、原子写入。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use neverwork_core::{AppError, AppResult, FileNode};

/// 目录遍历时忽略的目录名。
const IGNORE_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".cache",
    ".idea",
    ".vscode",
    "__pycache__",
    ".venv",
    "venv",
];

/// 单个文件最大读取字节数（超过则截断，防止卡死）。
const MAX_FILE_BYTES: usize = 512 * 1024;

/// 节点总数上限，防止误展开超深目录时卡死 UI。
const MAX_NODES: usize = 8000;

/// 从根目录构建文件树。只有 `expanded` 里的目录会被递归展开。
pub fn build_tree(root: &Path, expanded: &HashSet<PathBuf>) -> AppResult<Vec<FileNode>> {
    let mut out = Vec::new();
    walk(root, root, 0, expanded, &mut out)?;
    Ok(out)
}

/// 默认展开状态：根目录下的第一层子目录。
pub fn default_expanded(root: &Path) -> HashSet<PathBuf> {
    let mut set = HashSet::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if !IGNORE_DIRS.contains(&name.as_str()) {
                    set.insert(entry.path());
                }
            }
        }
    }
    set
}

fn walk(
    base: &Path,
    dir: &Path,
    level: u8,
    expanded: &HashSet<PathBuf>,
    out: &mut Vec<FileNode>,
) -> AppResult<()> {
    if out.len() >= MAX_NODES {
        return Ok(());
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    let mut items: Vec<(bool, String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && IGNORE_DIRS.contains(&name.as_str()) {
            continue;
        }
        items.push((is_dir, name, entry.path()));
    }

    // 目录在前，然后按名称排序（忽略大小写）。
    items.sort_by(|a, b| match (a.0, b.0) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
    });

    for (is_dir, name, path) in items {
        if out.len() >= MAX_NODES {
            return Ok(());
        }
        let is_expanded = is_dir && expanded.contains(&path);
        let rel = path
            .strip_prefix(base)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| name.clone());
        out.push(FileNode {
            name,
            is_dir,
            level,
            expanded: is_expanded,
            modified: false,
            owner: None,
            path: rel,
        });
        if is_expanded {
            walk(base, &path, level + 1, expanded, out)?;
        }
    }
    Ok(())
}

/// 读取文件内容（UTF-8，失败则 lossy；超限截断）。
pub fn read_file(path: &Path) -> AppResult<String> {
    let bytes = fs::read(path)?;
    let truncated = bytes.len() > MAX_FILE_BYTES;
    let slice = if truncated { &bytes[..MAX_FILE_BYTES] } else { &bytes[..] };
    let mut text = String::from_utf8_lossy(slice).to_string();
    if truncated {
        text.push_str("\n\n... (file too large, truncated)");
    }
    Ok(text)
}

/// 原子写入文件（临时文件 + rename），避免写一半损坏用户文件。
pub fn write_file(path: &Path, content: &str) -> AppResult<()> {
    let tmp = path.with_extension("nw-tmp");
    fs::write(&tmp, content).map_err(AppError::Io)?;
    // Windows 上 rename 到已存在文件会失败，先删目标。
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        AppError::Io(e)
    })?;
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_set_controls_recursion() {
        let dir = std::env::temp_dir().join(format!("nw-fs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "a").unwrap();
        fs::write(dir.join("sub/b.txt"), "b").unwrap();

        let collapsed = build_tree(&dir, &HashSet::new()).unwrap();
        assert!(collapsed.iter().any(|n| n.name == "sub"));
        assert!(!collapsed.iter().any(|n| n.name == "b.txt"));

        let mut expanded = HashSet::new();
        expanded.insert(dir.join("sub"));
        let open = build_tree(&dir, &expanded).unwrap();
        assert!(open.iter().any(|n| n.name == "b.txt"));
        // 路径必须是相对工作区根的，不能只是文件名。
        let b = open.iter().find(|n| n.name == "b.txt").unwrap();
        assert_eq!(b.path, "sub/b.txt");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_file_overwrites_existing() {
        let dir = std::env::temp_dir().join(format!("nw-fs-write-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.txt");
        fs::write(&f, "old").unwrap();
        write_file(&f, "new").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "new");
        let _ = fs::remove_dir_all(&dir);
    }
}
