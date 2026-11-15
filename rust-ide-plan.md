# 纯 Rust 重写自己的 IDE —— 技术选型与分阶段落地计划

> 定位：一款「AI 优先」的桌面 IDE。参考 Proma（Electron+React）的产品形态，但代码从零用 Rust 重写。
> 本文是需求拆分与技术路线，用于交给实现 Agent 执行。**不含依赖安装与代码生成**。

---

## 0. 先讲清楚三件事

1. **这不是「改 Proma」，是「另起一个项目」。** Proma 的 Electron/TS 代码不可复用；唯一可继承的是**产品设计**（三段布局、Chat/Agent 双模式、内嵌终端与浏览器、worktree、技能系统）。
2. **纯 Rust = 自绘 GUI 或 Tauri？** 见第 2 节，建议 **GPUI（Zed 内核）** 或 **Tauri 2 + Web 前端** 二选一。两条路差别巨大，需要先定。
3. **AI 能力 = 调 LLM API。** Rust 生态有成熟的异步 HTTP + SSE + 流式解析，这条没问题；难点在「编辑器本身」，不在 Agent。

---

## 1. 目标与非目标

### MVP 目标（第一期必须做到）
- 打开一个文件夹（工作区），左侧文件树，中间代码编辑器，右侧 AI 对话面板。
- 代码编辑：语法高亮、基本编辑、保存、文件树增删改。
- AI 对话：流式输出、多轮、可选模型、把「当前文件/选区」作为上下文发给模型。
- 内置终端：在工作区目录下开 shell。
- 设置：API Key、模型、主题。

### 非目标（先不做，避免泥潭）
- LSP 全语言智能补全/跳转（第一期只做语法树级高亮，智能能力后置）。
- 多人协作、Git 图形化全功能（先只做 status/diff，不碰 merge）。
- 插件体系、扩展市场。
- 多平台打包（先跑通 Windows 本机，mac/linux 后置）。

---

## 2. GUI 技术栈选型（**最关键决策**）

| 维度 | 方案 A：GPUI（Zed 内核） | 方案 B：Tauri 2 + React 前端 | 方案 C：egui / iced 纯 Rust 立即模式 |
|---|---|---|---|
| 语言纯度 | 几乎全 Rust | Rust 后端 + TS/React 前端 | 全 Rust |
| 效果上限 | 极高（Zed 同款，真·原生 IDE 体验） | 高（Web 渲染，编辑体验靠 Monaco/CodeMirror） | 中（自绘控件，写作体验一般） |
| 开发速度 | 慢（GPUI 生态小、文档少、API 迭代快） | **快**（前端生态成熟，Monaco 直接拿来） | 中 |
| 代码编辑器 | 需自建（工程量大） | **Monaco/CodeMirror 开箱即用** | 需自建或接 egui-code-editor（不成熟） |
| 包体/内存 | 小/低 | 中（WebView） | 小/低 |
| 风险 | GPUI 未发独立 crate，耦合 Zed 仓库，跟进成本高 | WebView2/WebKit 依赖；前后端两边写 | 富控件缺，后期扩展吃力 |

### 建议
- **如果目标是「做一个真正好用、要长期维护的 IDE」→ 选 B：Tauri 2 + React + Monaco。** 编辑器内核直接用 Monaco（VSCode 同款），Rust 专注做 Agent runtime、终端、文件系统、Git、搜索这些「重后端」。这是**投入产出比最高**的路线，且可以复用 Proma 的交互设计思路。
- **如果目标是「纯 Rust、追求极致原生、可以接受慢」→ 选 A：GPUI。** 参考 Zed 仓库，但要接受自建编辑器和跟版本的成本。
- **不建议 C**，除非只是个人练手。

> 结论：**推荐方案 B（Tauri 2 + React + Monaco + Rust 后端）**。下文按 B 展开，A 的差异在第 8 节标出。

---

## 3. 架构总览（方案 B）

```
┌─────────────────────────────────────────────────────────┐
│  前端 (React + TS + Monaco)  ← WebView                  │
│  左:文件树  中:编辑器/终端  右:AI 对话                  │
│  状态:Zustand/Jotai    编辑器:Monaco                    │
└───────────────▲─────────────────────────▲───────────────┘
                │ Tauri IPC (invoke/commands)│ events(流式)
┌───────────────┴─────────────────────────┴───────────────┐
│  Rust 后端 (Tauri core + 自建 crates)                    │
│  ┌──────────┐ ┌──────────┐ ┌─────────┐ ┌──────────────┐ │
│  │ workspace│ │  agent   │ │ terminal│ │ git / search │ │
│  │ fs/tree  │ │ llm/sse  │ │ pty     │ │              │ │
│  └──────────┘ └──────────┘ └─────────┘ └──────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### 模块切分（对应到 crates）
```
my-ide/
├── src-tauri/            # Tauri 应用壳 + command 注册
├── crates/
│   ├── core/             # 通用类型、错误、配置（serde）
│   ├── workspace/        # 文件树、watcher、读写（原子写）
│   ├── editor/           # 文件打开/保存、编码、diff（可选）
│   ├── agent/            # LLM 客户端、会话、工具调用、流式
│   ├── terminal/         # PTY 管理
│   ├── gitkit/           # git status/diff（libgit2 或调 git）
│   └── search/           # 全文搜索（ignore + grep 引擎）
└── src/                  # 前端 React
    ├── components/       # 三栏布局、对话、编辑器容器
    ├── stores/           # 状态
    └── lib/ipc.ts        # 封装 invoke/event
```

---

## 4. Rust crate 选型（逐模块，均已核对存在与可用性）

> 版本以 `cargo add` 当日解析为准，这里给「用什么、为什么」。

### 应用壳 / GUI
- **Tauri 2**（`tauri`, `tauri-build`）：Rust 后端 + Web 前端，Windows 用 WebView2。**注意本机 WebView2 未检测到，需先安装或确认系统自带。**
- 前端：`react` + `typescript` + `vite`；编辑器 **Monaco**（`monaco-editor`），对话/UI 参考 shadcn/Tailwind。

### 文件系统 / 工作区
- `ignore`：**必用**，gitignore 感知的递归遍历（ripgrep 同款），用于文件树和搜索。
- `notify`：跨平台文件监听（FileWatcher）。配合 `notify-debouncer-full` 去抖。
- `walkdir`：简单遍历兜底。
- 原子写：仿 Proma 的 safe-file 思路——**写临时文件 + rename**，避免半截文件破坏用户数据。

### 代码/语法
- **`tree-sitter`（0.27.x，MIT）**：语法树核心。
- `tree-sitter-<lang>`（如 `tree-sitter-rust`、`tree-sitter-typescript`）：各语言 grammar。
- `syntect`（可选）：主题化高亮，作为 Monaco 之外的备选。**编辑器高亮优先交 Monaco Topic，tree-sitter 主要给「结构化上下文」用（把函数/类抽取给 AI）。**

### AI / LLM
- `reqwest`：HTTP 客户端（`json`、`stream` feature）。
- `serde` / `serde_json`：请求响应序列化。
- `tokio`：异步运行时；`tokio-util`（`codec`）做大块流读取。
- **SSE 解析**：`eventsource-stream`（把字节流转成 event 流）或 `reqwest-eventsource`。主流 LLM 都是 SSE 流式，这条必须稳。
- 多模型：自己写 provider trait 适配 OpenAI 兼容 / Anthropic / 本地 Ollama。参考 Proma 的 `packages/core` 设计（provider 适配 + SSE），但代码重写。
- 密钥安全：`keyring`（系统钥匙串）存 API Key，**别明文写配置文件**。

### 终端
- **`portable-pty`**（wezterm 出品）：跨平台 PTY，Windows 走 ConPTY。**这是 Rust 里最成熟的 PTY 方案，对标 Proma 的 node-pty。**
- 前端终端：`xterm.js` + 通过 Tauri event 双向传字节。
- 注意：ConPTY 需要 Windows 10 1809+。

### Git
- **`git2`（libgit2 绑定）**：status/diff/log 读操作最方便，无需依赖系统 git。
- 或 `gix`（gitoxide，纯 Rust，正在成熟）——**若追求纯 Rust，可用 gix 读 status/diff**，但写操作（commit/push）gix 还需评估；稳妥起见读用 gix/git2、写可直接调用系统 `git` 命令（`std::process`）。
- 系统 git 已在本机可用（v2.54），**M1 直接调 `git` CLI 最省事**，后置再换 git2/gix。

### 搜索
- `grep-searcher` + `grep-regex` + `ignore`（ripgrep 核心库三件套）：做工作区全文搜索。**这是 ripgrep 官方可复用内核，质量极高。**

### 配置/持久化
- `serde` + `toml`/`json`：配置与工作区状态。
- `directories`（`dirs`）：跨平台配置目录。
- 本地优先：**不用数据库**，用 JSON/JSONL 文件（对齐 Proma 理念，便于用户迁移和 diff）。

### 其他
- `anyhow` / `thiserror`：错误处理。
- `tracing` + `tracing-subscriber`：日志。
- `uuid`、`chrono`、`regex`、`once_cell`/`std OnceLock`：常规。
- `tokio::sync::mpsc`：后端→前端流式事件的通道。

---

## 5. 分阶段计划（里程碑 + 验收标准）

> 每个里程碑都要「可运行 + 可验收」，不要一次性铺开。

### M0　地基（约 1 周）
- 建 monorepo：`src-tauri` + `crates/*` + `src/`（React）。
- Rust workspace 组织，CI 跑 `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test`、前端 `tsc --noEmit`。
- Tauri 壳跑起来，空窗口 + 一条 `ping` command 打通 invoke。
- 统一错误类型 `AppError`（thiserror）+ 日志就绪。
- **验收**：`cargo tauri dev` 打开窗口，前端调 `ping` 返回 "pong"。

### M1　工作区与编辑器（约 2 周）
- 打开文件夹（系统对话框 `tauri-plugin-dialog`）。
- 文件树：`ignore` 遍历，懒加载目录，watcher 实时刷新。
- Monaco 编辑器：打开/编辑/保存；原子写；多标签页；未保存提示。
- 简单主题（深浅）。
- **验收**：能打开任意项目目录，浏览文件、编辑并保存、外部改动自动刷新。

### M2　AI 对话（约 2 周，核心）
- LLM provider trait + OpenAI 兼容 provider 先行（覆盖绝大多数网关）。
- SSE 流式：`reqwest` + `eventsource-stream`，`tokio::mpsc` → Tauri `emit` 逐块推前端。
- 对话面板：多轮、Markdown 渲染、代码块、复制、中止（cancel）。
- 上下文：把「当前文件全文 / 选中代码 / 附加文件」组装进 prompt。
- API Key 存 `keyring`，设置页配置模型与 base_url。
- **验收**：配置一个 OpenAI 兼容网关，右侧面板流式对话；能带上当前文件上下文并正确回答代码问题；能中途停止。

### M3　终端与文件操作（约 2 周）
- `portable-pty` 起 shell（Windows PowerShell/ConPTY），xterm.js 渲染，尺寸同步。
- Agent **工具调用**：read_file / write_file / list_dir / run_terminal（受权限策略约束）。
- 权限模型：每个危险操作要显式确认（对齐 Proma 的 permission 思路）。
- **验收**：IDE 内终端可用；AI 能读文件并在用户确认后写文件；能执行命令并回显。

### M4　编码体验补强（约 2–3 周）
- 全文搜索面板（`grep-*` + `ignore`）。
- 代码大纲/符号面板（`tree-sitter` 查询抽取函数/结构）。
- Git status 面板 + diff 查看（先调 git CLI）。
- 命令面板（`cmdk` 风格）、快捷键体系。
- **验收**：跨文件搜索、跳转命中、看 git 改动、快捷键面板可用。

### M5　打磨与打包（约 1–2 周）
- 设置持久化、首次启动引导。
- 崩溃恢复、日志、自动更新（`tauri-plugin-updater`）。
- Windows 打包（MSI/NSIS），后续 mac/linux。
- **验收**：`cargo tauri build` 产出安装包，安装后能正常使用全部 MVP 功能。

> 总估：**10–13 周**（单人全职理想值；含踩坑和 LSP 后置不计）。GPUI 方案在此基础上 +30~50%，主要花在自建编辑器。

---

## 6. 关键设计约束（一定要早定）

1. **IPC 契约先行。** 像 Proma 的「四层契约」一样，Rust 端 command 的请求/响应类型在前端有一份对应的 TS 类型（用 `ts-rs` 自动生成，避免手写漂移）。
2. **流式事件要有 id 和 seq。** 对话、终端输出都是流，前端要能按 session 路由、支持取消、断线可恢复。
3. **原子写文件。** 任何保存/Agent 写文件都走「临时文件 + rename」，禁止直接覆盖。
4. **权限边界。** Agent 的文件/终端操作默认「需确认」；工作区根之外禁止访问（对齐 Proma 的 workspace 隔离）。
5. **本地优先持久化。** 配置与会话用 JSON/JSONL，不上数据库。
6. **不引入重依赖凭习惯。** 每加一个 crate 先看维护状态与许可证。

---

## 7. Agent（AI）设计要点

- **Provider 抽象**：trait `LlmProvider { async fn stream(&self, req) -> impl Stream<Item=Delta> }`，实现 OpenAI 兼容 / Anthropic / Ollama。
- **Agent loop**：对话 → 模型返回 tool_call → 执行工具 → 结果回填 → 再请求，直到无工具调用。**工具执行要可取消、要限步数（防死循环）。**
- **上下文管理**：把当前文件、选区、搜索命中作为「结构化上下文」注入；超长时做裁剪（先符号后正文）。
- **参考**：Proma 用 Pi Agent runtime + 一套 `agent-*.ts`（orchestrator / permission / prompt-builder / session-manager）。**照它的职责划分重写为 Rust 模块**，不要照搬代码。

---

## 8. 若改选 GPUI（方案 A）的差异

- GUI 用 `gpui` + `gpui-component`（社区组件库），编辑器需自建（参考 Zed 的 `editor`/`text`/`rope` crate，`ropey` 做文本模型）。
- 高亮用 `tree-sitter` + 自绘，或移植 Zed 的 highlight。
- 优点：无需 WebView、性能极致、包体小；缺点：生态小、文档少、跟进 GPUI 版本成本高、开发周期显著拉长。
- **除非「纯 Rust + 原生性能」是硬需求，否则不建议首发就选 A。**

---

## 9. 环境前置（本机现状，动手前要补）

- Rust：**未安装** → 需 `rustup` + MSVC 工具链（VS2022 Build Tools 已在）。
- WebView2：**未检测到**（方案 B 必须，Windows 需装 WebView2 Runtime）。
- 终端：ConPTY 需 Win10 1809+（当前系统需确认版本）。
- cmake 在 `D:\soft\cmake\bin`（部分 crate 的 build script 可能用到）。
- Git v2.54 已有（M1/M4 可先调 CLI）。

---

## 10. 待你拍板的 3 个问题（回答后即可开工拆分任务）

1. **GUI 选 A（GPUI 纯 Rust）还是 B（Tauri 2 + React + Monaco）？** — 我推荐 B。
2. **首发平台**：只 Windows，还是 Win + macOS 都要？
3. **AI 渠道**：只用 OpenAI 兼容网关（最通用），还是要同时支持 Anthropic / 本地 Ollama / 其他？

> 定了这三点，我就把 M0–M5 拆成可派发的任务卡（每张卡：目标、改动文件/新文件、验收命令），交给实现 Agent 执行。
