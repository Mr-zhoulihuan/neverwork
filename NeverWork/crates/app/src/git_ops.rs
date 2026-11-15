//! Git 写操作：暂存、取消暂存、提交、丢弃、分支切换与新建。
//!
//! 安全原则（重要）：
//! - **只有「丢弃改动」是破坏性的**，它不可撤销。因此它单独标注 `is_destructive()`，
//!   UI 必须先弹确认，不能和其他按钮同等对待。
//! - 提交前检查「暂存区是否为空」，空了直接报错而不是让 git 输出一长串提示。
//! - 所有失败都把 git 的 stderr 原文带回去，而不是笼统的「操作失败」——
//!   分支切换失败常有具体原因（未提交改动、分支不存在），用户需要看到。
//! - 提交信息为空直接拒绝，避免产生空标题提交。

use std::path::Path;
use std::process::Command;

/// 写操作结果。
#[derive(Debug, Clone)]
pub struct OpResult {
    pub ok: bool,
    /// 成功时的简短说明，或失败时的原因（含 git 原文）。
    pub message: String,
}

impl OpResult {
    fn ok(msg: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: msg.into(),
        }
    }

    fn err(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: msg.into(),
        }
    }
}

/// 可能的写操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitOp {
    /// 暂存一个文件（`git add --`）。
    Stage(String),
    /// 取消暂存（`git restore --staged --`）。
    Unstage(String),
    /// 丢弃工作区改动（`git restore --` 或未跟踪文件删除）。
    /// **不可撤销**。
    Discard(String),
    /// 提交暂存区内容。
    Commit(String),
    /// 切换到已有分支。
    SwitchBranch(String),
    /// 新建并切换到新分支。
    CreateBranch(String),
    /// 暂存全部改动。
    StageAll,
}

impl GitOp {
    /// 是否是破坏性操作（需要二次确认）。
    pub fn is_destructive(&self) -> bool {
        matches!(self, GitOp::Discard(_))
    }

    /// 一句话描述，用于确认弹窗与日志。
    pub fn describe(&self) -> String {
        match self {
            GitOp::Stage(p) => format!("暂存 {p}"),
            GitOp::Unstage(p) => format!("取消暂存 {p}"),
            GitOp::Discard(p) => format!("丢弃 {p} 的改动（不可撤销）"),
            GitOp::Commit(m) => {
                let one: String = m.lines().next().unwrap_or("").chars().take(50).collect();
                format!("提交「{one}」")
            }
            GitOp::SwitchBranch(b) => format!("切换到分支 {b}"),
            GitOp::CreateBranch(b) => format!("新建并切换到分支 {b}"),
            GitOp::StageAll => "暂存全部改动".into(),
        }
    }
}

/// 跑一条 git，返回 (是否成功, stdout+stderr)。
fn run_raw(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        // 不弹编辑器（否则提交/分支操作会挂住后台线程）
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .output();

    match out {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr).to_string();
            if !err.trim().is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&err);
            }
            (o.status.success(), text.trim().to_string())
        }
        Err(e) => (false, format!("无法执行 git：{e}")),
    }
}

/// 取 git 输出里最有信息量的一行（通常最后一行是结论）。
fn best_line(text: &str) -> String {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .next_back()
        .unwrap_or("")
        .to_string()
}

/// 执行一次写操作。`dir` 是仓库内任意目录。
pub fn apply(dir: &Path, op: &GitOp) -> OpResult {
    match op {
        GitOp::Stage(path) => simple(dir, &["add", "--", path], format!("已暂存 {path}")),
        GitOp::StageAll => simple(dir, &["add", "-A"], "已暂存全部改动".to_string()),
        GitOp::Unstage(path) => unstage(dir, path),
        GitOp::Discard(path) => discard(dir, path),
        GitOp::Commit(msg) => commit(dir, msg),
        GitOp::SwitchBranch(name) => switch_branch(dir, name),
        GitOp::CreateBranch(name) => create_branch(dir, name),
    }
}

/// 取消暂存。
///
/// 三种情况要分别处理：
/// 1. 正常有提交：`git restore --staged`
/// 2. 老版本 git 没有 restore：回退到 `git reset HEAD`
/// 3. **仓库还没有任何提交**：没有 HEAD，前两者都会报
///    `could not resolve 'HEAD'`；此时用 `git rm --cached` 从索引里摘掉。
fn unstage(dir: &Path, path: &str) -> OpResult {
    let (ok, text) = run_raw(dir, &["restore", "--staged", "--", path]);
    if ok {
        return OpResult::ok(format!("已取消暂存 {path}"));
    }

    // 没有 HEAD（空仓库）：只能从索引里删
    let no_head = text.contains("could not resolve")
        || text.contains("ambiguous argument 'HEAD'")
        || text.contains("unknown revision");
    if no_head {
        let (ok2, text2) = run_raw(dir, &["rm", "--cached", "--quiet", "--", path]);
        return if ok2 {
            OpResult::ok(format!("已取消暂存 {path}（仓库尚无提交）"))
        } else {
            OpResult::err(format!("取消暂存失败：{}", best_line(&text2)))
        };
    }

    // 老版本 git 没有 restore 子命令
    if text.contains("unknown option") || text.contains("not a git command") {
        let (ok3, text3) = run_raw(dir, &["reset", "HEAD", "--", path]);
        return if ok3 {
            OpResult::ok(format!("已取消暂存 {path}"))
        } else {
            OpResult::err(format!("取消暂存失败：{}", best_line(&text3)))
        };
    }

    OpResult::err(format!("取消暂存失败：{}", best_line(&text)))
}

fn simple(dir: &Path, args: &[&str], ok_msg: String) -> OpResult {
    let (ok, text) = run_raw(dir, args);
    if ok {
        OpResult::ok(ok_msg)
    } else {
        OpResult::err(format!("操作失败：{}", best_line(&text)))
    }
}

/// 丢弃改动。
///
/// 已跟踪文件 `git restore --` 还原到 HEAD；未跟踪文件 git 不管，
/// 这时用 `git clean -f --` 删除（只删这一个文件，不做目录级清理）。
fn discard(dir: &Path, path: &str) -> OpResult {
    // 先判断是否被 git 跟踪
    let (tracked, _) = run_raw(dir, &["ls-files", "--error-unmatch", "--", path]);
    if tracked {
        let (ok, _text) = run_raw(dir, &["restore", "--", path]);
        if ok {
            return OpResult::ok(format!("已丢弃 {path} 的改动"));
        }
        // 老 git 回退
        let (ok2, text2) = run_raw(dir, &["checkout", "--", path]);
        return if ok2 {
            OpResult::ok(format!("已丢弃 {path} 的改动"))
        } else {
            OpResult::err(format!("丢弃失败：{}", best_line(&text2)))
        };
    }

    // 未跟踪：删除这个文件
    let full = dir.join(path);
    if !full.exists() {
        return OpResult::err(format!("{path} 不存在"));
    }
    match std::fs::remove_file(&full) {
        Ok(()) => OpResult::ok(format!("已删除未跟踪文件 {path}")),
        Err(e) => OpResult::err(format!("删除失败：{e}")),
    }
}

/// 提交暂存区。
fn commit(dir: &Path, message: &str) -> OpResult {
    let msg = message.trim();
    if msg.is_empty() {
        return OpResult::err("提交信息不能为空");
    }

    // 暂存区为空就别提交了，否则 git 会给一长串提示
    let (ok, staged) = run_raw(dir, &["diff", "--cached", "--name-only"]);
    if !ok {
        return OpResult::err(format!("无法读取暂存区：{}", best_line(&staged)));
    }
    if staged.trim().is_empty() {
        return OpResult::err("暂存区为空，先暂存要提交的文件");
    }

    let (ok, text) = run_raw(dir, &["commit", "-m", msg]);
    if ok {
        // 取第一行（形如 `[main abc1234] 标题`）
        let first = text.lines().next().unwrap_or("已提交").trim().to_string();
        OpResult::ok(first)
    } else {
        OpResult::err(format!("提交失败：{}", best_line(&text)))
    }
}

/// 切换到已有分支。
fn switch_branch(dir: &Path, name: &str) -> OpResult {
    let name = name.trim();
    if name.is_empty() {
        return OpResult::err("分支名不能为空");
    }
    if branch_exists(dir, name) {
        // 已存在就直接切
        let (ok, text) = run_raw(dir, &["checkout", name]);
        return if ok {
            OpResult::ok(format!("已切换到 {name}"))
        } else {
            OpResult::err(format!("切换失败：{}", best_line(&text)))
        };
    }
    OpResult::err(format!("分支 {name} 不存在"))
}

/// 新建并切换。
fn create_branch(dir: &Path, name: &str) -> OpResult {
    let name = name.trim();
    if name.is_empty() {
        return OpResult::err("分支名不能为空");
    }
    if branch_exists(dir, name) {
        return OpResult::err(format!("分支 {name} 已存在"));
    }
    // check-ref-format 能挡住含空格、~、^ 等非法字符的名字
    let (valid, _) = run_raw(dir, &["check-ref-format", "--branch", name]);
    if !valid {
        return OpResult::err(format!("分支名 {name} 不合法"));
    }

    let (ok, text) = run_raw(dir, &["checkout", "-b", name]);
    if ok {
        OpResult::ok(format!("已新建并切换到 {name}"))
    } else {
        OpResult::err(format!("新建失败：{}", best_line(&text)))
    }
}

fn branch_exists(dir: &Path, name: &str) -> bool {
    let (ok, _) = run_raw(
        dir,
        &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")],
    );
    ok
}

/// 列出本地分支（当前分支排第一）。
pub fn branches(dir: &Path) -> Vec<String> {
    let (ok, text) = run_raw(dir, &["branch", "--format=%(refname:short)"]);
    if !ok {
        return Vec::new();
    }
    let mut out: Vec<String> = text
        .lines()
        .map(|l| l.trim().trim_start_matches("* ").to_string())
        .filter(|l| !l.is_empty())
        .collect();
    // 把当前分支挪到最前，方便用户一眼看到
    if let Some(cur) = current_branch(dir) {
        if let Some(i) = out.iter().position(|b| b == &cur) {
            let b = out.remove(i);
            out.insert(0, b);
        }
    }
    out
}

/// 当前分支名。
pub fn current_branch(dir: &Path) -> Option<String> {
    let (ok, text) = run_raw(dir, &["rev-parse", "--abbrev-ref", "HEAD"]);
    if ok && !text.trim().is_empty() {
        Some(text.trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 建一个临时仓库，配好 user，避免 commit 因缺身份失败。
    fn temp_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nw-gitops-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        run_raw(&dir, &["init", "-q", "-b", "main"]);
        run_raw(&dir, &["config", "user.email", "t@example.com"]);
        run_raw(&dir, &["config", "user.name", "Tester"]);
        run_raw(&dir, &["config", "commit.gpgsign", "false"]);
        // 关掉行尾转换：Windows 上 autocrlf 会把 LF 变 CRLF，
        // 导致「还原后的内容」和测试写入的原文不等，属于环境噪声。
        run_raw(&dir, &["config", "core.autocrlf", "false"]);
        dir
    }

    fn write(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    fn read(dir: &Path, name: &str) -> String {
        fs::read_to_string(dir.join(name)).unwrap()
    }

    #[test]
    fn stage_then_commit() {
        let dir = temp_repo("commit");
        write(&dir, "a.txt", "hello\n");

        let r = apply(&dir, &GitOp::Stage("a.txt".into()));
        assert!(r.ok, "{}", r.message);

        let r = apply(&dir, &GitOp::Commit("first commit".into()));
        assert!(r.ok, "{}", r.message);

        // 提交后工作区应干净
        let (_, st) = run_raw(&dir, &["status", "--porcelain"]);
        assert!(st.trim().is_empty(), "应无未提交改动: {st}");
    }

    #[test]
    fn commit_without_staged_files_is_rejected() {
        let dir = temp_repo("emptycommit");
        let r = apply(&dir, &GitOp::Commit("nothing".into()));
        assert!(!r.ok);
        assert!(r.message.contains("暂存区为空"), "{}", r.message);
    }

    #[test]
    fn empty_commit_message_is_rejected() {
        let dir = temp_repo("emptymsg");
        write(&dir, "a.txt", "x\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        let r = apply(&dir, &GitOp::Commit("   ".into()));
        assert!(!r.ok);
        assert!(r.message.contains("不能为空"), "{}", r.message);
    }

    #[test]
    fn unstage_keeps_file_content() {
        let dir = temp_repo("unstage");
        write(&dir, "a.txt", "v1\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        let r = apply(&dir, &GitOp::Unstage("a.txt".into()));
        assert!(r.ok, "{}", r.message);
        assert_eq!(read(&dir, "a.txt"), "v1\n", "取消暂存不该动文件内容");
    }

    #[test]
    fn unstage_works_before_first_commit() {
        // 空仓库（无 HEAD）时取消暂存：git restore 会失败，需要走 rm --cached
        let dir = temp_repo("firstunstage");
        write(&dir, "a.txt", "v1
");
        let r = apply(&dir, &GitOp::Stage("a.txt".into()));
        assert!(r.ok, "{}", r.message);

        let r = apply(&dir, &GitOp::Unstage("a.txt".into()));
        assert!(r.ok, "{}", r.message);
        let (_, staged) = run_raw(&dir, &["diff", "--cached", "--name-only"]);
        assert!(staged.trim().is_empty(), "暂存区应清空: {staged}");
        assert_eq!(read(&dir, "a.txt"), "v1
", "文件内容不该变");
    }

    #[test]
    fn discard_restores_tracked_file() {
        let dir = temp_repo("discard");
        write(&dir, "a.txt", "original\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        apply(&dir, &GitOp::Commit("c1".into()));

        write(&dir, "a.txt", "changed\n");
        let r = apply(&dir, &GitOp::Discard("a.txt".into()));
        assert!(r.ok, "{}", r.message);
        assert_eq!(read(&dir, "a.txt"), "original\n", "应还原到 HEAD 版本");
    }

    #[test]
    fn discard_removes_untracked_file() {
        let dir = temp_repo("discardnew");
        write(&dir, "new.txt", "tmp\n");
        let r = apply(&dir, &GitOp::Discard("new.txt".into()));
        assert!(r.ok, "{}", r.message);
        assert!(!dir.join("new.txt").exists(), "未跟踪文件应被删除");
    }

    #[test]
    fn discard_is_marked_destructive() {
        assert!(GitOp::Discard("x".into()).is_destructive());
        assert!(!GitOp::Stage("x".into()).is_destructive());
        assert!(!GitOp::Commit("m".into()).is_destructive());
        assert!(!GitOp::SwitchBranch("b".into()).is_destructive());
        assert!(GitOp::Discard("x".into()).describe().contains("不可撤销"));
    }

    #[test]
    fn create_and_switch_branch() {
        let dir = temp_repo("branch");
        write(&dir, "a.txt", "x\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        apply(&dir, &GitOp::Commit("c1".into()));

        let r = apply(&dir, &GitOp::CreateBranch("feature".into()));
        assert!(r.ok, "{}", r.message);
        assert_eq!(current_branch(&dir).as_deref(), Some("feature"));

        let r = apply(&dir, &GitOp::SwitchBranch("main".into()));
        assert!(r.ok, "{}", r.message);
        assert_eq!(current_branch(&dir).as_deref(), Some("main"));
    }

    #[test]
    fn switching_to_missing_branch_reports_clearly() {
        let dir = temp_repo("nobranch");
        write(&dir, "a.txt", "x\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        apply(&dir, &GitOp::Commit("c1".into()));

        let r = apply(&dir, &GitOp::SwitchBranch("nope".into()));
        assert!(!r.ok);
        assert!(r.message.contains("不存在"), "{}", r.message);
    }

    #[test]
    fn creating_existing_branch_is_rejected() {
        let dir = temp_repo("dup");
        write(&dir, "a.txt", "x\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        apply(&dir, &GitOp::Commit("c1".into()));

        let r = apply(&dir, &GitOp::CreateBranch("main".into()));
        assert!(!r.ok);
        assert!(r.message.contains("已存在"), "{}", r.message);
    }

    #[test]
    fn invalid_branch_name_is_rejected() {
        let dir = temp_repo("badname");
        write(&dir, "a.txt", "x\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        apply(&dir, &GitOp::Commit("c1".into()));

        let r = apply(&dir, &GitOp::CreateBranch("bad name".into()));
        assert!(!r.ok);
        assert!(
            r.message.contains("不合法") || r.message.contains("失败"),
            "{}",
            r.message
        );
    }

    #[test]
    fn branches_lists_current_first() {
        let dir = temp_repo("list");
        write(&dir, "a.txt", "x\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        apply(&dir, &GitOp::Commit("c1".into()));
        apply(&dir, &GitOp::CreateBranch("zzz".into()));

        let list = branches(&dir);
        assert!(list.contains(&"main".to_string()), "{list:?}");
        assert!(list.contains(&"zzz".to_string()), "{list:?}");
        assert_eq!(list[0], "zzz", "当前分支应排第一: {list:?}");
    }

    #[test]
    fn stage_all_picks_up_everything() {
        let dir = temp_repo("all");
        write(&dir, "a.txt", "1\n");
        write(&dir, "b.txt", "2\n");
        let r = apply(&dir, &GitOp::StageAll);
        assert!(r.ok, "{}", r.message);
        let (_, staged) = run_raw(&dir, &["diff", "--cached", "--name-only"]);
        assert!(staged.contains("a.txt") && staged.contains("b.txt"), "{staged}");
    }

    #[test]
    fn chinese_commit_message_roundtrip() {
        let dir = temp_repo("cjk");
        write(&dir, "a.txt", "x\n");
        apply(&dir, &GitOp::Stage("a.txt".into()));
        let r = apply(&dir, &GitOp::Commit("修复：中文提交信息".into()));
        assert!(r.ok, "{}", r.message);
        let (_, log) = run_raw(&dir, &["log", "-1", "--pretty=%s"]);
        assert_eq!(log.trim(), "修复：中文提交信息");
    }
}
