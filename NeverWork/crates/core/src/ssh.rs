//! 远程工作区：系统 OpenSSH 之上的极简文件操作。
//!
//! 设计取舍：
//! - 认证交给系统 ssh：密钥 / ssh-agent / `~/.ssh/config` 优先；密码只留在内存，
//!   经 `SSH_ASKPASS` 回调本进程传递（不落盘、不进命令行）。
//!   不自动信任主机指纹（首次连接失败会提示先在终端接受指纹）。
//! - 每条命令一次 ssh，先保证正确性；连接复用留给以后。
//! - 所有命令都在 `cd <root> && ...` 里跑，路径一律相对工作区根，
//!   避免 `~` 展开与绝对路径拼接带来的歧义。

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

/// 单文件读取上限（与本地编辑器一致）。
pub const MAX_REMOTE_FILE_BYTES: usize = 512 * 1024;
/// 文件树最深展开层数，防止误展开超深目录。
const MAX_TREE_DEPTH: usize = 8;
/// 文件树节点上限。
const MAX_TREE_NODES: usize = 8000;
/// 目录与文件两段输出之间的分隔标记。
const TREE_MARKER: &str = "__NW_FILES__";
/// 文件树里跳过的目录（与本地 `filesystem.rs` 保持一致）。
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

/// 一个远程工作区目标。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTarget {
    /// `~/.ssh/config` 别名或 `user@host`（不含端口）。
    pub host: String,
    pub port: u16,
    /// 远端绝对 POSIX 路径或以 `~` 开头；无尾斜杠。
    pub root: String,
}

impl SshTarget {
    /// 解析用户输入。支持：
    /// - `user@host:/abs/path`
    /// - `host:2222:/abs/path`
    /// - `host:~/payments`
    /// - `ssh://user@host:22/abs/path`
    pub fn parse(input: &str) -> Result<Self, String> {
        let raw = input.trim();
        if raw.is_empty() {
            return Err("请输入远程地址，如 user@host:/abs/path".into());
        }

        let (authority, root) = if let Some(rest) = raw.strip_prefix("ssh://") {
            let (authority, path) = rest
                .split_once('/')
                .ok_or_else(|| "ssh:// 后面缺少目录，如 ssh://host/home/me/app".to_string())?;
            let root = if path.starts_with('~') {
                path.to_string()
            } else {
                format!("/{path}")
            };
            (authority.to_string(), root)
        } else {
            let (authority, rest) = raw.split_once(':').ok_or_else(|| {
                "格式应为 [user@]host[:端口]:/绝对路径，例如 me@10.0.0.2:/srv/app".to_string()
            })?;
            if rest.starts_with('/') || rest.starts_with('~') {
                (authority.to_string(), rest.to_string())
            } else if let Some((port, path)) = rest.split_once(':') {
                if !path.starts_with('/') && !path.starts_with('~') {
                    return Err("远端路径必须是绝对路径（/ 开头）或以 ~ 开头".into());
                }
                (format!("{authority}:{port}"), path.to_string())
            } else {
                return Err("远端路径必须是绝对路径（/ 开头）或以 ~ 开头".into());
            }
        };

        let (host, port) = split_authority(&authority)?;
        let root = normalize_root(&root)?;
        Ok(Self { host, port, root })
    }

    /// 连接串，写入 `settings.last_workspace` 与 `Workspace.remote_url`。
    pub fn url(&self) -> String {
        // `~` 路径要保留成 `ssh://host/~/x`，解析时才能还原成 `~/x`。
        let path = if self.root.starts_with('~') {
            format!("/{}", self.root)
        } else {
            self.root.clone()
        };
        if self.port == 22 {
            format!("ssh://{}{}", self.host, path)
        } else {
            format!("ssh://{}:{}{}", self.host, self.port, path)
        }
    }

    /// 展示名：远端目录最后一段。
    pub fn name(&self) -> String {
        self.root
            .trim_end_matches('/')
            .rsplit('/')
            .find(|s| !s.is_empty() && *s != "~")
            .unwrap_or(&self.host)
            .to_string()
    }

    /// `host:path` 形式，给提示与标题用。
    pub fn display(&self) -> String {
        format!("{}:{}", self.host, self.root)
    }
}

/// 最近工作区 / 设置里保存的字符串是不是远程连接串。
pub fn is_remote_url(path: &str) -> bool {
    path.trim_start().starts_with("ssh://")
}

fn split_authority(authority: &str) -> Result<(String, u16), String> {
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            let port: u16 = p.parse().map_err(|_| format!("端口不合法：{p}"))?;
            if port == 0 {
                return Err("端口必须大于 0".into());
            }
            (h, port)
        }
        _ => (authority, 22),
    };
    if host.trim().is_empty() {
        return Err("缺少主机名".into());
    }
    if host.chars().any(char::is_whitespace) {
        return Err("主机名里不能有空格".into());
    }
    Ok((host.to_string(), port))
}

/// 工作区根：绝对路径或 `~` 开头；词法归一化并去掉尾斜杠。
fn normalize_root(p: &str) -> Result<String, String> {
    let p = p.trim();
    if p == "~" {
        return Ok("~".into());
    }
    if let Some(rest) = p.strip_prefix("~/") {
        let rest = normalize_rel(rest)?;
        return Ok(if rest.is_empty() {
            "~".into()
        } else {
            format!("~/{rest}")
        });
    }
    if !p.starts_with('/') {
        return Err("远端路径必须是绝对路径（/ 开头）或以 ~ 开头".into());
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

/// 词法归一化相对路径；`..` 越出根时拒绝。
pub fn normalize_rel(rel: &str) -> Result<String, String> {
    let replaced = rel.replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for seg in replaced.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("路径越出工作区：{rel}"));
                }
            }
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

/// 单引号转义，供远端 sh 使用。
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `cd` 的目标：`~` 用 `$HOME` 展开，其余整体单引号。
fn cd_target(root: &str) -> String {
    if root == "~" {
        "\"$HOME\"".to_string()
    } else if let Some(rest) = root.strip_prefix("~/") {
        format!("\"$HOME\"{}", sh_quote(&format!("/{rest}")))
    } else {
        sh_quote(root)
    }
}

/// 远端文件树里的一个节点（扁平列表，level 由调用方按路径算）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteNode {
    /// 相对工作区根的 POSIX 路径。
    pub path: String,
    pub is_dir: bool,
}

/// 认证方式。
///
/// 密码只保存在内存里：不写 settings、不进命令行（进程列表看不到），
/// 由 `SSH_ASKPASS` 交回本进程（`main.rs` 开头的 askpass 分支）。
#[derive(Clone, Default)]
pub struct RemoteAuth {
    password: Option<String>,
}

impl RemoteAuth {
    /// 只用密钥 / ssh-agent。
    pub fn key_only() -> Self {
        Self::default()
    }

    pub fn password(pw: impl Into<String>) -> Self {
        let pw = pw.into();
        Self {
            password: (!pw.is_empty()).then_some(pw),
        }
    }

    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    fn secret(&self) -> Option<&str> {
        self.password.as_deref()
    }
}

impl std::fmt::Debug for RemoteAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 密码永远不进日志：调试输出、错误信息里出现一次就算泄漏
        if self.password.is_some() {
            write!(f, "RemoteAuth::Password(<hidden>)")
        } else {
            write!(f, "RemoteAuth::KeyOnly")
        }
    }
}

/// 远程文件系统句柄。
#[derive(Debug, Clone)]
pub struct RemoteFs {
    target: SshTarget,
    auth: RemoteAuth,
}

impl RemoteFs {
    /// 只用密钥 / ssh-agent。
    pub fn new(target: SshTarget) -> Self {
        Self {
            target,
            auth: RemoteAuth::key_only(),
        }
    }

    pub fn with_auth(target: SshTarget, auth: RemoteAuth) -> Self {
        Self { target, auth }
    }

    pub fn auth(&self) -> &RemoteAuth {
        &self.auth
    }

    pub fn target(&self) -> &SshTarget {
        &self.target
    }

    /// 探测目录是否存在，返回展开 `~` 后的绝对路径。
    pub fn probe(&self) -> Result<String, String> {
        let cmd = format!("cd {} && pwd", cd_target(&self.target.root));
        let out = run_ssh(&self.target, &cmd, None, &self.auth)?;
        let pwd = out
            .lines()
            .last()
            .map(|s| s.trim())
            .unwrap_or("")
            .to_string();
        if pwd.is_empty() || !pwd.starts_with('/') {
            return Err(format!("远端目录不可用：{}", self.target.root));
        }
        Ok(pwd)
    }

    /// 拉取文件树。只展开 `expanded` 里的目录（根层永远可见）。
    pub fn tree(&self, expanded: &std::collections::HashSet<String>) -> Result<Vec<RemoteNode>, String> {
        let depth = expanded
            .iter()
            .map(|p| p.split('/').filter(|s| !s.is_empty()).count())
            .max()
            .unwrap_or(0)
            + 1;
        let depth = depth.clamp(1, MAX_TREE_DEPTH);
        let prune = IGNORE_DIRS
            .iter()
            .map(|n| format!("-name {}", sh_quote(n)))
            .collect::<Vec<_>>()
            .join(" -o ");
        let cmd = format!(
            "cd {root} && find . -maxdepth {depth} \\( {prune} \\) -prune -o -type d -print | head -n {cap} ; \
             echo {marker} ; \
             find . -maxdepth {depth} \\( {prune} \\) -prune -o -type f -print | head -n {cap}",
            root = cd_target(&self.target.root),
            marker = sh_quote(TREE_MARKER),
            cap = MAX_TREE_NODES,
            depth = depth,
            prune = prune,
        );
        let out = run_ssh(&self.target, &cmd, None, &self.auth)?;
        let (dirs, files) = out.split_once(TREE_MARKER).unwrap_or((out.as_str(), ""));
        Ok(build_visible_tree(dirs, files, expanded))
    }

    /// 列一层目录。返回 `(is_dir, name)`。
    pub fn list_entries(&self, rel: &str) -> Result<Vec<(bool, String)>, String> {
        let rel = self.rel_path(rel)?;
        let cmd = self.in_root(&format!("ls -1pA {}", self.arg(&rel)));
        let out = run_ssh(&self.target, &cmd, None, &self.auth)?;
        Ok(parse_ls(&out))
    }

    /// 读文本（超过 512KB 截断），与本地编辑器行为一致。
    pub fn read_text(&self, rel: &str) -> Result<String, String> {
        let rel = self.rel_path(rel)?;
        let cmd = self.in_root(&format!(
            "head -c {} {}",
            MAX_REMOTE_FILE_BYTES + 1,
            self.arg(&rel)
        ));
        let out = run_ssh(&self.target, &cmd, None, &self.auth)?;
        let bytes = out.as_bytes();
        if bytes.len() > MAX_REMOTE_FILE_BYTES {
            let mut text = String::from_utf8_lossy(&bytes[..MAX_REMOTE_FILE_BYTES]).to_string();
            text.push_str("\n\n... (file too large, truncated)");
            Ok(text)
        } else {
            Ok(out)
        }
    }

    /// 读文本；文件不存在时返回空串（写前看旧内容 / diff 预览用）。
    pub fn read_text_opt(&self, rel: &str) -> Result<String, String> {
        match self.read_text(rel) {
            Ok(t) => Ok(t),
            Err(e) if e.contains("不存在") => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    /// 读整个文本；超过上限直接报错。
    ///
    /// 编辑器用这个而不是 `read_text`：截断后的内容一旦被保存，
    /// 就会把远端文件写坏（只留前半段）。
    pub fn read_text_exact(&self, rel: &str) -> Result<String, String> {
        let rel = self.rel_path(rel)?;
        let cmd = self.in_root(&format!(
            "head -c {} {}",
            MAX_REMOTE_FILE_BYTES + 1,
            self.arg(&rel)
        ));
        let out = run_ssh(&self.target, &cmd, None, &self.auth)?;
        if out.len() > MAX_REMOTE_FILE_BYTES {
            return Err(format!(
                "文件过大（> {}KB），请到远端终端或本地 IDE 处理",
                MAX_REMOTE_FILE_BYTES / 1024
            ));
        }
        Ok(out)
    }

    /// 写文本：先写临时文件再 `mv -f`，避免写一半损坏文件。
    pub fn write_text(&self, rel: &str, content: &str) -> Result<(), String> {
        let rel = self.rel_path(rel)?;
        let parent = rel.rsplit_once('/').map(|(p, _)| p.to_string());
        let tmp = if rel.is_empty() {
            return Err("不能写入工作区根目录".into());
        } else {
            format!("{rel}.nw-tmp")
        };
        let mkdir = parent
            .filter(|p| !p.is_empty())
            .map(|p| format!("mkdir -p {} && ", self.arg(&p)))
            .unwrap_or_default();
        let cmd = self.in_root(&format!(
            "{mkdir}cat > {tmp} && mv -f {tmp} {dst}",
            tmp = self.arg(&tmp),
            dst = self.arg(&rel)
        ));
        run_ssh(&self.target, &cmd, Some(content.as_bytes()), &self.auth)?;
        Ok(())
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.test(rel, "-e")
    }

    pub fn is_dir(&self, rel: &str) -> bool {
        self.test(rel, "-d")
    }

    fn test(&self, rel: &str, flag: &str) -> bool {
        let Ok(rel) = self.rel_path(rel) else {
            return false;
        };
        let cmd = self.in_root(&format!(
            "[ {flag} {p} ] && echo 1 || true",
            flag = flag,
            p = self.arg(&rel)
        ));
        run_ssh(&self.target, &cmd, None, &self.auth)
            .map(|o| o.trim() == "1")
            .unwrap_or(false)
    }

    /// 子串检索（grep -F），返回 `path:line: 内容`，与本地 `search_files` 对齐。
    pub fn search_fixed(&self, pattern: &str, base: &str, limit: usize) -> Result<Vec<String>, String> {
        if pattern.is_empty() {
            return Ok(Vec::new());
        }
        let base = if base.trim().is_empty() {
            String::new()
        } else {
            self.rel_path(base)?
        };
        let excludes = IGNORE_DIRS
            .iter()
            .map(|d| format!("--exclude-dir={}", sh_quote(d)))
            .collect::<Vec<_>>()
            .join(" ");
        let cmd = self.in_root(&format!(
            "grep -rnF -I {excludes} -e {pat} {base} | head -n {limit}",
            pat = sh_quote(pattern),
            base = self.arg(&base),
            limit = limit
        ));
        let out = run_ssh(&self.target, &cmd, None, &self.auth)?;
        Ok(parse_grep(&out))
    }

    /// 远程相对路径：接受根内绝对路径，其余转相对并挡 `..` 越界。
    fn rel_path(&self, rel: &str) -> Result<String, String> {
        let raw = rel.trim().replace('\\', "/");
        if raw.starts_with('~') {
            return Err(format!("不允许使用 ~ 路径：{rel}"));
        }
        if raw.starts_with('/') {
            if self.target.root.starts_with('~') {
                return Err("远程工作区尚未连接（目录还没展开），请稍后重试".into());
            }
            let root = self.target.root.trim_end_matches('/');
            let norm = normalize_root(&raw)?;
            if norm == root {
                return Ok(String::new());
            }
            if let Some(rest) = norm.strip_prefix(&format!("{root}/")) {
                return Ok(rest.to_string());
            }
            return Err(format!("路径越出工作区：{rel}"));
        }
        normalize_rel(&raw)
    }

    fn in_root(&self, tail: &str) -> String {
        format!("cd {} && {}", cd_target(&self.target.root), tail)
    }

    fn arg(&self, rel: &str) -> String {
        if rel.is_empty() {
            sh_quote(".")
        } else {
            sh_quote(rel)
        }
    }
}

/// 执行一条远端命令。`stdin` 有值时写入子进程标准输入（写文件用）。
///
/// 有密码时关掉 `BatchMode`（它会禁用密码提示），改用 `SSH_ASKPASS`：
/// 密码经环境变量交给回调进程，命令行里只有 host 与命令，进程列表看不到密码。
pub fn run_ssh(
    target: &SshTarget,
    command: &str,
    stdin: Option<&[u8]>,
    auth: &RemoteAuth,
) -> Result<String, String> {
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-o",
        "ConnectTimeout=8",
        "-o",
        "ServerAliveInterval=5",
        "-o",
        "ServerAliveCountMax=2",
    ]);
    if auth.has_password() {
        cmd.args([
            "-o",
            "BatchMode=no",
            "-o",
            "NumberOfPasswordPrompts=1",
            "-o",
            "PreferredAuthentications=publickey,password,keyboard-interactive",
        ]);
        // askpass 由本程序提供（`NW_ASKPASS_MODE=1` 分支），
        // 密码只经环境变量进子进程，不进命令行。
        let askpass = std::env::var_os("NW_ASKPASS_EXE")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .ok_or_else(|| "无法定位 askpass 程序".to_string())?;
        cmd.env("SSH_ASKPASS", askpass)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("NW_ASKPASS_MODE", "1")
            .env("NW_ASKPASS_SECRET", auth.secret().unwrap_or_default());
    } else {
        cmd.args(["-o", "BatchMode=yes"]);
    }
    if target.port != 22 {
        cmd.arg("-p").arg(target.port.to_string());
    }
    cmd.arg(&target.host).arg(command);
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 无窗口应用里子进程默认会弹控制台；CREATE_NO_WINDOW 压住它。
        cmd.creation_flags(0x0800_0000);
    }

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "未找到 ssh 命令：请先安装 OpenSSH 客户端，并确认它在 PATH 里".to_string()
        } else {
            format!("启动 ssh 失败：{e}")
        }
    })?;

    if let Some(input) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            // 写不进去（连接断了）不在这里报错：先等子进程退出，拿它的 stderr。
            let _ = pipe.write_all(input);
        }
    }

    let out = child
        .wait_with_output()
        .map_err(|e| format!("等待 ssh 结束失败：{e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(friendly_error(&target.host, &stderr, auth.has_password()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn friendly_error(host: &str, stderr: &str, password_auth: bool) -> String {
    let text = stderr.trim();
    let lower = text.to_lowercase();
    if lower.contains("host key verification failed") {
        format!("主机指纹未确认：先在终端执行 ssh {host}，接受指纹后重试")
    } else if lower.contains("permission denied") {
        if password_auth {
            format!("认证失败：密码不正确，或主机未允许密码登录（也可改用 ssh 密钥）")
        } else {
            format!(
                "认证失败：请确认 ssh 密钥 / ssh-agent 可用（ssh {host} 能免密登录），或填写密码"
            )
        }
    } else if lower.contains("connection timed out")
        || lower.contains("operation timed out")
        || lower.contains("no route to host")
    {
        format!("连接 {host} 超时：检查网络或主机地址")
    } else if lower.contains("could not resolve hostname") {
        format!("主机名解析失败：{host}")
    } else if lower.contains("no such file or directory") {
        "远端路径不存在或不可读".to_string()
    } else if text.is_empty() {
        format!("连接 {host} 失败（ssh 未返回原因）")
    } else {
        let head: Vec<&str> = text.lines().take(3).collect();
        format!("远程命令失败：{}", head.join(" "))
    }
}

fn parse_ls(out: &str) -> Vec<(bool, String)> {
    let mut items = Vec::new();
    for line in out.lines() {
        let name = line.trim_end_matches('\r');
        if name.is_empty() || name == "." || name == ".." || name == ".DS_Store" {
            continue;
        }
        if let Some(dir) = name.strip_suffix('/') {
            if !dir.is_empty() && !IGNORE_DIRS.contains(&dir) {
                items.push((true, dir.to_string()));
            }
        } else if !IGNORE_DIRS.contains(&name) {
            items.push((false, name.to_string()));
        }
    }
    items
}

/// 把 `find` 两段输出整理成「只含可见节点」的扁平树（顺序与本地一致：目录先、忽略大小写）。
fn build_visible_tree(
    dirs_out: &str,
    files_out: &str,
    expanded: &std::collections::HashSet<String>,
) -> Vec<RemoteNode> {
    let mut children: HashMap<String, Vec<(bool, String)>> = HashMap::new();
    let mut total = 0usize;
    for (is_dir, out) in [(true, dirs_out), (false, files_out)] {
        for line in out.lines() {
            let path = line.trim().trim_start_matches("./").trim_end_matches('/');
            if path.is_empty() || path == "." {
                continue;
            }
            let (parent, name) = match path.rsplit_once('/') {
                Some((p, n)) => (p.to_string(), n.to_string()),
                None => (String::new(), path.to_string()),
            };
            if name.is_empty() || name == ".DS_Store" {
                continue;
            }
            children.entry(parent).or_default().push((is_dir, name));
            total += 1;
            if total >= MAX_TREE_NODES {
                break;
            }
        }
    }

    let mut out = Vec::new();
    walk_children("", &children, expanded, &mut out);
    out
}

fn walk_children(
    parent: &str,
    children: &HashMap<String, Vec<(bool, String)>>,
    expanded: &std::collections::HashSet<String>,
    out: &mut Vec<RemoteNode>,
) {
    let Some(items) = children.get(parent) else {
        return;
    };
    let mut items = items.clone();
    items.sort_by(|a, b| match (a.0, b.0) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.1.to_lowercase().cmp(&b.1.to_lowercase()),
    });
    for (is_dir, name) in items {
        if out.len() >= MAX_TREE_NODES {
            return;
        }
        let path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        out.push(RemoteNode {
            path: path.clone(),
            is_dir,
        });
        if is_dir && expanded.contains(&path) {
            walk_children(&path, children, expanded, out);
        }
    }
}

fn parse_grep(out: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        let mut parts = line.splitn(3, ':');
        let (Some(path), Some(no), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if no.parse::<usize>().is_err() {
            continue;
        }
        let path = path.trim_start_matches("./");
        let mut shown: String = text.trim().chars().take(160).collect();
        if text.trim().chars().count() > 160 {
            shown.push('…');
        }
        hits.push(format!("{path}:{no}: {shown}"));
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_target() {
        let t = SshTarget::parse(" me@10.0.0.2:/srv/app/ ").unwrap();
        assert_eq!(t.host, "me@10.0.0.2");
        assert_eq!(t.port, 22);
        assert_eq!(t.root, "/srv/app");
        assert_eq!(t.name(), "app");
        assert_eq!(t.url(), "ssh://me@10.0.0.2/srv/app");
    }

    #[test]
    fn parses_port_and_ssh_url() {
        let t = SshTarget::parse("host:2222:/srv/app").unwrap();
        assert_eq!(t.port, 2222);
        assert_eq!(t.url(), "ssh://host:2222/srv/app");

        let round = SshTarget::parse(&t.url()).unwrap();
        assert_eq!(round, t);
    }

    #[test]
    fn parses_tilde_paths() {
        let t = SshTarget::parse("dev:~/payments").unwrap();
        assert_eq!(t.root, "~/payments");
        assert_eq!(t.name(), "payments");
        let round = SshTarget::parse(&t.url()).unwrap();
        assert_eq!(round, t, "url={}", t.url());
        assert_eq!(t.url(), "ssh://dev/~/payments");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(SshTarget::parse("").is_err());
        assert!(SshTarget::parse("host").is_err());
        assert!(SshTarget::parse("host:relative/path").is_err());
        assert!(SshTarget::parse(":22:/x").is_err());
        assert!(SshTarget::parse("host:0:/x").is_err());
        assert!(SshTarget::parse("ssh://host").is_err());
    }

    #[test]
    fn detects_remote_url() {
        assert!(is_remote_url("ssh://a/b"));
        assert!(!is_remote_url("D:/code"));
    }

    #[test]
    fn quotes_shell() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("路径"), "'路径'");
    }

    #[test]
    fn normalizes_relative_paths() {
        assert_eq!(normalize_rel("a//b/./c").unwrap(), "a/b/c");
        assert_eq!(normalize_rel("a/../b").unwrap(), "b");
        assert!(normalize_rel("../etc").is_err());
        assert!(normalize_rel("a/../../etc").is_err());
    }

    #[test]
    fn maps_relative_and_absolute_paths() {
        let fs = RemoteFs::new(SshTarget {
            host: "h".into(),
            port: 22,
            root: "/srv/app".into(),
        });
        assert_eq!(fs.rel_path("src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(fs.rel_path("/srv/app/src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(fs.rel_path("/srv/app").unwrap(), "");
        assert!(fs.rel_path("/etc/passwd").is_err());
        assert!(fs.rel_path("../x").is_err());
    }

    #[test]
    fn parses_ls_output() {
        let items = parse_ls("src/\nREADME.md\ntarget/\n.git/\n.DS_Store\n");
        assert_eq!(
            items,
            vec![(true, "src".to_string()), (false, "README.md".to_string())]
        );
    }

    #[test]
    fn filters_collapsed_tree_nodes() {
        let dirs = "./src\n./src/app\n./docs\n";
        let files = "./README.md\n./src/main.rs\n./src/app/x.rs\n./docs/a.md\n";
        let collapsed = build_visible_tree(dirs, files, &Default::default());
        let paths: Vec<&str> = collapsed.iter().map(|n| n.path.as_str()).collect();
        assert!(paths.contains(&"src"));
        assert!(paths.contains(&"README.md"));
        assert!(!paths.contains(&"src/main.rs"), "未展开目录不能出现子节点");

        let mut expanded = std::collections::HashSet::new();
        expanded.insert("src".to_string());
        let open = build_visible_tree(dirs, files, &expanded);
        let paths: Vec<&str> = open.iter().map(|n| n.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(!paths.contains(&"src/app/x.rs"), "只展开一层");
        // 目录排在文件前
        assert_eq!(open[0].path, "docs");
    }

    #[test]
    fn parses_grep_output() {
        let hits = parse_grep("./src/main.rs:12:let x = 1;\nnot a hit\n./a b.txt:3: hello\n");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0], "src/main.rs:12: let x = 1;");
        assert_eq!(hits[1], "a b.txt:3: hello");
    }

    #[test]
    fn builds_cd_targets() {
        assert_eq!(cd_target("/srv/app"), "'/srv/app'");
        assert_eq!(cd_target("~"), "\"$HOME\"");
        assert_eq!(cd_target("~/p q"), "\"$HOME\"'/p q'");
    }

    #[test]
    fn debug_never_leaks_password() {
        let auth = RemoteAuth::password("s3cret-pw");
        let shown = format!("{auth:?}");
        assert!(!shown.contains("s3cret-pw"), "调试输出不能带密码：{shown}");
        assert!(shown.contains("hidden"));
        assert!(auth.has_password());
        assert!(!RemoteAuth::key_only().has_password());
        assert!(!RemoteAuth::password("").has_password(), "空密码等于没填");
    }

    #[test]
    #[ignore = "需要真实远端：NW_SSH_TEST=user@host:/tmp/nw"]
    fn remote_roundtrip() {
        let spec = std::env::var("NW_SSH_TEST").expect("设置 NW_SSH_TEST");
        let target = SshTarget::parse(&spec).unwrap();
        let fs = RemoteFs::new(target);
        let pwd = fs.probe().unwrap();
        assert!(pwd.starts_with('/'));
        fs.write_text("nw-test/hello.txt", "第一行\nneedle\n").unwrap();
        let text = fs.read_text("nw-test/hello.txt").unwrap();
        assert!(text.contains("needle"));
        let hits = fs.search_fixed("needle", "nw-test", 10).unwrap();
        assert!(!hits.is_empty(), "grep 应命中");
        let entries = fs.list_entries("nw-test").unwrap();
        assert!(entries.iter().any(|(d, n)| !d && n == "hello.txt"));
        let tree = fs.tree(&Default::default()).unwrap();
        assert!(tree.iter().any(|n| n.path == "nw-test" && n.is_dir));
    }

    #[test]
    #[ignore = "需要真实远端：NW_SSH_TEST、NW_SSH_TEST_PASSWORD、NW_ASKPASS_EXE"]
    fn remote_password_roundtrip() {
        // 测试二进制本身不提供 askpass，所以要把 NW_ASKPASS_EXE 指向 neverwork.exe
        let spec = std::env::var("NW_SSH_TEST").expect("设置 NW_SSH_TEST");
        let pw = std::env::var("NW_SSH_TEST_PASSWORD").expect("设置 NW_SSH_TEST_PASSWORD");
        let store = std::env::var("NW_ASKPASS_EXE").expect("设置 NW_ASKPASS_EXE=neverwork.exe");
        assert!(!store.is_empty());
        let target = SshTarget::parse(&spec).unwrap();
        let fs = RemoteFs::with_auth(target, RemoteAuth::password(pw));
        let pwd = fs.probe().unwrap();
        assert!(pwd.starts_with('/'));
        fs.write_text("nw-test/pw.txt", "password-auth\n").unwrap();
        let text = fs.read_text("nw-test/pw.txt").unwrap();
        assert!(text.contains("password-auth"));
    }
}
