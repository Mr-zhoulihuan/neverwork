//! Git 集成：读状态、分支、提交历史、单文件 diff。
//!
//! 全部走 `git` 命令行，不引 libgit2。
//! 理由：libgit2 要编 C 依赖，构建时间和体积都上一个量级；而我们只需要**只读**的
//! 展示能力（status / log / diff），git 命令输出稳定、解析简单。
//!
//! 所有命令都带 `-C <repo>`，因此不依赖进程 cwd；非 git 目录返回 `NotARepo`
//! 而不是报错，让界面能给出「这里不是仓库」的明确提示。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 一个文件的改动状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// 新增（未跟踪 `??` 或已暂存 `A`）。
    Added,
    /// 修改。
    Modified,
    /// 删除。
    Deleted,
    /// 重命名 / 移动。
    Renamed,
    /// 冲突（merge/rebase 未解决）。
    Conflicted,
}

impl ChangeKind {
    /// 中文说明（界面提示用）。
    #[allow(dead_code)]
    pub fn label(&self) -> &'static str {
        match self {
            ChangeKind::Added => "新增",
            ChangeKind::Modified => "修改",
            ChangeKind::Deleted => "删除",
            ChangeKind::Renamed => "重命名",
            ChangeKind::Conflicted => "冲突",
        }
    }

    /// 单字符标记（界面左侧显示）。
    pub fn marker(&self) -> &'static str {
        match self {
            ChangeKind::Added => "A",
            ChangeKind::Modified => "M",
            ChangeKind::Deleted => "D",
            ChangeKind::Renamed => "R",
            ChangeKind::Conflicted => "!",
        }
    }
}

/// 一个被改动的文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChangeGit {
    /// 相对仓库根、正斜杠分隔。
    pub path: String,
    pub kind: ChangeKind,
    /// 是否已暂存（进了 index）。
    pub staged: bool,
}

/// 一条提交。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub summary: String,
    pub author: String,
    /// git 给的相对时间，如「2 hours ago」。
    pub when: String,
}

/// 仓库状态快照。
#[derive(Debug, Clone, Default)]
pub struct RepoStatus {
    /// 当前分支名；detached HEAD 时是 `(detached)`。
    pub branch: String,
    /// 上游分支（可能为空）。
    pub upstream: String,
    /// 领先 / 落后上游的提交数。
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<FileChangeGit>,
    pub commits: Vec<Commit>,
}

impl RepoStatus {
    pub fn staged_count(&self) -> usize {
        self.changes.iter().filter(|c| c.staged).count()
    }

    pub fn unstaged_count(&self) -> usize {
        self.changes.iter().filter(|c| !c.staged).count()
    }

    pub fn has_conflicts(&self) -> bool {
        self.changes
            .iter()
            .any(|c| c.kind == ChangeKind::Conflicted)
    }
}

/// 查询结果。
#[derive(Debug, Clone)]
pub enum RepoQuery {
    Ok(RepoStatus),
    /// 目录不在 git 仓库里（或没装 git）。面板自己会给占位文案，不需要消息。
    NotARepo,
}

/// 在当前目录的**直接子目录**里找 git 仓库。
///
/// 用途：工作区常常是一个「容器目录」，其下每个项目各自是仓库
/// （例如 `E:\go\goDemo`、`E:\go\ops-go`）。此时根目录本身不是仓库，
/// 但要给用户一个一键进入某个子仓库的入口，而不是只提示「不是仓库」。
pub fn child_repos(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if e.path().join(".git").exists() {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// 找到 `dir` 所属仓库的根。
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    let out = run(dir, &["rev-parse", "--show-toplevel"])?;
    let root = out.trim();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

/// 跑一条 git 命令，成功返回 stdout。
fn run(dir: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(dir)
        .args(args)
        // 关掉分页与颜色，否则输出里会夹 ANSI 码
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 无窗口应用里子进程默认会弹控制台；CREATE_NO_WINDOW 压住它。
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// 读一次完整状态。
pub fn status(dir: &Path) -> RepoQuery {
    if repo_root(dir).is_none() {
        return RepoQuery::NotARepo;
    }

    let mut st = RepoStatus::default();

    // 分支
    st.branch = run(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(detached)".into());

    // 上游 + 领先/落后
    if let Some(up) = run(
        dir,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"],
    ) {
        let up = up.trim().to_string();
        if !up.is_empty() {
            st.upstream = up;
            if let Some(counts) = run(
                dir,
                &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
            ) {
                let mut it = counts.split_whitespace();
                st.ahead = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
                st.behind = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
            }
        }
    }

    // 文件状态：--porcelain 每行两列，第一列 index，第二列 worktree
    if let Some(porcelain) = run(dir, &["status", "--porcelain=v1", "-z"]) {
        // -z 用 NUL 分隔，避免文件名里有空格时被拆错
        for entry in porcelain.split('\0').filter(|s| !s.is_empty()) {
            if entry.len() < 4 {
                continue;
            }
            let xy: Vec<char> = entry.chars().take(2).collect();
            let path = entry[3..].to_string();
            let (x, y) = (xy[0], xy[1]);

            // 未跟踪
            if x == '?' && y == '?' {
                st.changes.push(FileChangeGit {
                    path,
                    kind: ChangeKind::Added,
                    staged: false,
                });
                continue;
            }
            // 冲突：DD/AU/UD/UA/DU/AA/UU
            let conflict = matches!(
                (x, y),
                ('D', 'D') | ('A', 'U') | ('U', 'D') | ('U', 'A') | ('D', 'U') | ('A', 'A') | ('U', 'U')
            );
            if conflict {
                st.changes.push(FileChangeGit {
                    path,
                    kind: ChangeKind::Conflicted,
                    staged: false,
                });
                continue;
            }

            // 暂存区有变化
            if x != ' ' && x != '?' {
                st.changes.push(FileChangeGit {
                    path: path.clone(),
                    kind: kind_of(x),
                    staged: true,
                });
            }
            // 工作区有变化
            if y != ' ' && y != '?' {
                st.changes.push(FileChangeGit {
                    path,
                    kind: kind_of(y),
                    staged: false,
                });
            }
        }
    }

    // 最近提交
    if let Some(log) = run(
        dir,
        &["log", "-n", "20", "--pretty=format:%h\x1f%s\x1f%an\x1f%ar"],
    ) {
        for line in log.lines() {
            let parts: Vec<&str> = line.split('\x1f').collect();
            if parts.len() >= 4 {
                st.commits.push(Commit {
                    hash: parts[0].to_string(),
                    summary: parts[1].to_string(),
                    author: parts[2].to_string(),
                    when: parts[3].to_string(),
                });
            }
        }
    }

    RepoQuery::Ok(st)
}

fn kind_of(c: char) -> ChangeKind {
    match c {
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' | 'C' => ChangeKind::Renamed,
        'U' => ChangeKind::Conflicted,
        _ => ChangeKind::Modified,
    }
}

/// 取某个文件相对 HEAD 的 diff（工作区 + 暂存区一起看）。
pub fn file_diff(dir: &Path, path: &str) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    // 已跟踪文件的改动
    if let Some(d) = run(dir, &["diff", "HEAD", "--", path]) {
        lines.extend(parse_diff(&d));
    }
    // 未跟踪文件：git diff 看不到，直接读文件全量当新增
    if lines.is_empty() {
        if let Some(untracked) = run(dir, &["ls-files", "--others", "--exclude-standard", "--", path])
        {
            if !untracked.trim().is_empty() {
                let full = dir.join(path);
                if let Ok(text) = std::fs::read_to_string(&full) {
                    for (i, l) in text.lines().take(400).enumerate() {
                        lines.push(DiffLine {
                            marker: "+".into(),
                            text: format!("{:>4} {l}", i + 1),
                        });
                    }
                }
            }
        }
    }
    lines
}

/// diff 展示行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// `+` / `-` / ` `
    pub marker: String,
    pub text: String,
}

/// 解析 unified diff 的正文行（跳过文件头与 hunk 头）。
pub fn parse_diff(raw: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let mut in_hunk = false;
    for line in raw.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            // hunk 头本身作为分隔线显示出来，方便定位
            out.push(DiffLine {
                marker: "@".into(),
                text: line.to_string(),
            });
            continue;
        }
        if !in_hunk {
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            out.push(DiffLine {
                marker: "+".into(),
                text: rest.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix('-') {
            out.push(DiffLine {
                marker: "-".into(),
                text: rest.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix(' ') {
            out.push(DiffLine {
                marker: " ".into(),
                text: rest.to_string(),
            });
        }
        if out.len() >= 800 {
            break;
        }
    }
    out
}

/// 让界面能显示「有多少改动」的紧凑摘要。
pub fn summary_line(st: &RepoStatus) -> String {
    if st.changes.is_empty() {
        return "工作区干净".into();
    }
    format!(
        "{} 个文件（暂存 {} · 未暂存 {}）",
        st.changes.len(),
        st.staged_count(),
        st.unstaged_count()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_parsed_diff_markers() {
        let raw = "@@ -1,3 +1,3 @@\n-old\n+new\n same\n";
        let d = parse_diff(raw);
        assert_eq!(d.len(), 4);
        assert_eq!(d[0].marker, "@");
        assert_eq!(d[1].marker, "-");
        assert_eq!(d[1].text, "old");
        assert_eq!(d[2].marker, "+");
        assert_eq!(d[3].marker, " ");
    }

    #[test]
    fn ignores_header_lines() {
        let raw = "diff --git a/x b/x\nindex 123..456\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        let d = parse_diff(raw);
        // 只保留 hunk 之后的内容（+1 条 hunk 头）
        assert_eq!(d.len(), 3);
        assert!(d.iter().all(|l| !l.text.contains("index 123")));
    }

    #[test]
    fn caps_long_diffs() {
        let mut raw = String::from("@@ -1 +1 @@\n");
        for i in 0..2000 {
            raw.push_str(&format!("+line{i}\n"));
        }
        let d = parse_diff(&raw);
        assert!(d.len() <= 801, "应截断，实际 {}", d.len());
    }

    #[test]
    fn kind_of_maps_git_codes() {
        assert_eq!(kind_of('A'), ChangeKind::Added);
        assert_eq!(kind_of('M'), ChangeKind::Modified);
        assert_eq!(kind_of('D'), ChangeKind::Deleted);
        assert_eq!(kind_of('R'), ChangeKind::Renamed);
        assert_eq!(kind_of('U'), ChangeKind::Conflicted);
    }

    #[test]
    fn status_of_non_repo_is_explicit() {
        let dir = std::env::temp_dir().join(format!("nw-nogit-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // 临时目录通常不在 git 仓库里；万一在，跳过该断言
        if repo_root(&dir).is_none() {
            match status(&dir) {
                RepoQuery::NotARepo => {}
                RepoQuery::Ok(_) => panic!("不应识别为仓库"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_this_repo_status() {
        // 项目根是 git 仓库，应该能读出分支与提交
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .map(|p| p.to_path_buf());
        let Some(root) = root else { return };
        if repo_root(&root).is_none() {
            return; // 非仓库环境下跳过
        }
        match status(&root) {
            RepoQuery::Ok(st) => {
                assert!(!st.branch.is_empty(), "应能读出分支名");
                assert!(!st.commits.is_empty(), "应能读出提交历史");
                assert!(!st.commits[0].hash.is_empty());
            }
            RepoQuery::NotARepo => panic!("应识别为仓库"),
        }
    }

    #[test]
    fn finds_child_repos() {
        let base = std::env::temp_dir().join(format!("nw-childrepos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("proj-a/.git")).unwrap();
        std::fs::create_dir_all(base.join("proj-b")).unwrap(); // 不是仓库
        std::fs::create_dir_all(base.join(".hidden/.git")).unwrap(); // 隐藏目录应跳过

        let repos = child_repos(&base);
        assert_eq!(repos, vec!["proj-a".to_string()], "只应列出直接子仓库");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn child_repos_empty_for_missing_dir() {
        let missing = std::env::temp_dir().join("nw-does-not-exist-xyz");
        assert!(child_repos(&missing).is_empty());
    }

    #[test]
    fn summary_counts() {
        let mut st = RepoStatus::default();
        assert_eq!(summary_line(&st), "工作区干净");
        st.changes.push(FileChangeGit {
            path: "a".into(),
            kind: ChangeKind::Modified,
            staged: true,
        });
        st.changes.push(FileChangeGit {
            path: "b".into(),
            kind: ChangeKind::Added,
            staged: false,
        });
        assert_eq!(st.staged_count(), 1);
        assert_eq!(st.unstaged_count(), 1);
        assert!(summary_line(&st).contains("2 个文件"));
    }

    #[test]
    fn conflicts_are_detected() {
        let mut st = RepoStatus::default();
        st.changes.push(FileChangeGit {
            path: "x".into(),
            kind: ChangeKind::Conflicted,
            staged: false,
        });
        assert!(st.has_conflicts());
    }
}
