# 远程 SSH 工作区设计（阶段二）

日期：2026-11-17
状态：已确认（用户批准范围与认证方式）
范围：远程文件树 / 编辑器 / Agent 工具；远程 Git 面板与 watcher 不在本阶段。

## 1. 目标 / 非目标

目标：

1. 通过系统 OpenSSH 连接远程开发机，打开远程目录作为工作区。
2. 远程工作区下：文件树浏览、文件打开/保存、Agent 工具（读写/检索/批量补丁）全部作用于远端。
3. 会话仍存本机；远程会话库按连接串分库，重启可恢复上次远程工作区。
4. 连接失败（无 ssh、主机指纹未确认、认证失败、路径不存在）给出明确文案，不阻塞 UI。

非目标（阶段二不做）：

- 远程 Git 面板（`git_dir()` 远程返回 `None`，面板显示「远程暂不支持」）。
- 目录 watcher（改为手动刷新 / 展开时拉取）。
- 终端、端口转发、应用内密码、远程目录选择器、`search_code` 语义检索。
- SSH 连接复用（每条命令一次 ssh，先保证正确性）。

## 2. 数据模型

`core/src/ssh.rs`：

```rust
pub struct SshTarget { pub host: String, pub port: u16, pub root: String }
```

- `host` 是 `~/.ssh/config` 别名或 `user@host`（不含端口）。
- `root` 是远端绝对 POSIX 路径，无尾斜杠；`~` 在连接探测时展开。
- 连接串：`ssh://[user@]host[:port]/abs/path`，`url()` / `parse()` 互逆。
- 输入格式（工作区管理器）：`[user@]host[:port]:/abs/path`，也接受 `ssh://` 前缀；路径必须是绝对路径或以 `~` 开头。

`RecentWorkspace` 不加字段：`path` 存连接串，`is_remote()` 按 `ssh://` 前缀判断；远程行显示「远程」，不做 `is_dir` 存在性检查。

`AppState`：

- `root: Option<PathBuf>` 只表示本地根；新增 `remote: Option<SshTarget>`。
- `has_workspace()` 改为 `workspace.is_some()`。
- `session_key()`：远程返回连接串，本地返回根的路径字符串，无工作区返回 `scratch`。
- 新增 `remote_expanded: HashSet<String>`（相对 POSIX 路径）、`remote_open_rel: Option<String>`、`remote_gen: u64`（异步代次，防迟到结果串台）。

## 3. SSH 传输层（`core/src/ssh.rs`）

命令：

```
ssh -o BatchMode=yes -o ConnectTimeout=8 -o ServerAliveInterval=5 -o ServerAliveCountMax=2 [-p PORT] HOST "<command>"
```

- 不存密码、不弹交互；认证完全交给系统 ssh（密钥 / agent / config）。
- 主机指纹不自动信任：首次连接失败时提示先执行 `ssh <host>` 接受指纹。
- 远程路径单引号转义（`'` → `'\''`）。
- 写文件：`cat > '<path>.nw-tmp' && mv -f '<path>.nw-tmp' '<path>'`，内容走 stdin。
- 读文件：`head -c 524288 '<path>'`（与本地 512KB 上限一致）。
- 列目录：`ls -1pA '<path>'`（`/` 结尾即目录）。
- 文件树：按「最深已展开目录 + 1，上限 8」算 `find -maxdepth`，一条命令同时输出目录与文件（`node_modules` 等 prune），本地重建扁平 `FileNode` 列表；未展开目录的子节点被过滤。
- 检索：`grep -rnF -I --exclude-dir=... -e '<pattern>' '<base>'`，命中上限 200。
- 错误映射：主机指纹 / 认证失败 / 连接超时 / 路径不存在 / 远端命令失败（附 stderr 前几行）。

## 4. Agent 工具后端化

- `ToolContext` 内部改为后端枚举（本地 / 远程），对外提供按**相对路径字符串**的读写、列举、检索、展示接口；本地语义不变（`..`、跨盘、符号链接逃逸、`.git` 写入拦截、512KB 上限）。
- `read_file / list_dir / search_files / write_file / apply_patch / multi_patch` 全部走后端；写操作仍 `requires_approval`，审批 diff 预览用后端读原文。
- 远程 `search_code` 返回明确「远程暂不支持语义检索，请用 search_files」。
- 系统提示词按后端说明「当前是远程工作区 host:path，工具操作远端文件」。

## 5. App 集成

- 工作区管理器底部新增「⇅ 连接远程 SSH…」：单输入框（`[user@]host:/abs/path`），Enter 后异步探测 `cd '<root>' && pwd`（展开 `~`），成功才切换；失败在管理器内显示原因，不关闭输入。
- 切换远程：停旧 watcher → 存旧会话 → 设置 workspace/remote → 换会话库 → 异步拉文件树（默认只展开根层）→ 复位浮层与输入目标。
- 文件树展开/收起/刷新异步（std::thread + flume + `cx.spawn`），通过 `remote_gen` 丢弃过期结果。
- 编辑器：远程打开/保存异步；保存失败保留 dirty 并提示。
- Git 面板：远程显示「远程工作区暂不支持 Git 面板」，不执行任何本地 git。
- 标题栏沿用「· 远程」；调试开关 `NW_REMOTE=user@host:/abs/path` 启动即连。
- 启动恢复：`settings.last_workspace` 以 `ssh://` 开头时按远程恢复，异步探测；失败保留工作区但标 offline 并提示，可手动刷新重连。

## 6. 错误处理

- `ssh` 不存在：启动/连接时提示安装 OpenSSH。
- 连接/探测失败：notice 明确分类文案；不把远程误判为「无工作区」。
- 树/读取/保存失败：notice；编辑器内容与 dirty 不丢。
- 切换工作区仍在流式生成：沿用阶段一「先停止再切换」。

## 7. 测试与验收

单元测试：

- 连接串 `parse/url` 往返；端口、`user@`、`ssh://` 前缀、`~` 路径。
- shell 引号转义（含单引号、空格、中文）。
- 远程相对路径归一化与 `..` 越界拒绝。
- `ls -1pA` / `find` 树输出 / `grep` 输出解析。
- agent 本地工具全部既有测试保持通过。

集成（`#[ignore]`，`NW_SSH_TEST=user@host:/tmp/nw`）：探测、列目录、读、写、检索。

手动验收：

1. 连接远程 → 文件树出现 → 展开多级目录。
2. 打开远程文件 → 编辑 → 保存 → 远端内容更新；断网保存失败保留 dirty。
3. 对话让 agent 读远程文件、改远程文件（弹审批、拒绝不落盘）。
4. 切回本地工作区：树/编辑/工具全部回到本地。
5. 重启恢复上次远程工作区；ssh 未配置时提示清晰。
6. Git 面板显示「远程暂不支持」且不执行本地 git。

## 8. 补记：应用内密码认证（2026-11-17 追加）

原设计只支持密钥 / ssh-agent。验收阶段用户需要连接仅开放密码登录的机器（root 账号），
因此追加密码认证，约束不变：

- 密码只存内存（`AppState.remote_pw`），不写 settings / 日志 / 命令行；关闭应用即失效。
- 传输：密码经 `SSH_ASKPASS` 回调应用本体（`NW_ASKPASS_MODE` 分支，`main.rs` 最先处理），
  系统 ssh 以 `BatchMode=no` + `NumberOfPasswordPrompts=1` 运行；密钥仍优先。
- 认证抽象：`RemoteAuth`（KeyOnly / Password）+ `RemoteFs::with_auth`；`Debug` 手写打码。
- 界面：连接行新增密码框（失焦圆点、连接成功后清空转存内存）；重连最近远程项先试密钥，
  失败后展开输入行补密码。
- 测试：单测覆盖 Debug 不泄密；真实密码往返测试 `#[ignore]`，需 `NW_SSH_TEST_PASSWORD`
  与 `NW_ASKPASS_EXE`。
