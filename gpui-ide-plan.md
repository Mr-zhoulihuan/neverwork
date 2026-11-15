# 方案 A：纯 Rust + GPUI 重写自己的 IDE —— 技术选型与落地计划

> 方向已定：**方案 A，纯 Rust，GPUI（Zed 内核）**。参考 Proma 的产品形态，代码从零写。
> 本文已核对 2026-11 的 crate 现状。**不包含依赖安装与代码生成。**

---

## 0. 关键调研结论（先说，影响可行性）

1. **GPUI 已是官方 crate**：`gpui 0.2.2`（Apache-2.0，Zed Industries 发布，2026-10 更新）。
2. **Zed 已支持 Windows**（README 明确列 macOS/Linux/Windows，并有 `docs/src/development/windows.md`）。GPUI docs 里"仅 macOS/Linux"是旧文案，**Windows 可用**——但仍需留意：Windows 下 GPUI 用 D3D/Blade 后端，偶有平台 bug，要预留调试时间。
3. **不用从零写所有控件**：**GPUI Kit**（Longbridge 出品，生产级，Apache-2.0，驱动其商业产品 Longbridge Pro）提供：
   - `gpui-component 0.9.1`：**75+ 组件**，含 **Dock 布局**（可拖拽面板、可拖 Tab、嵌套分割）、虚拟列表、数据表格、富文本/Markdown、图表。
   - **Code Editor 组件**：tree-sitter 高亮 + **LSP**（诊断/补全/悬浮），**20 万行**性能。
   - 入口 crate `gpui-kit`（0.7）一次引入 gpui + base + component + assets。
   → **这是方案 A 能否快速落地的决定性因素。选它，M1/M4 的自建量大幅下降。**

> 建议：**主框架用 `gpui-kit`（或其分层 `gpui-base` + `gpui-component`），编辑器直接用 GPUI Kit 的 Code Editor，不要用 Zed 仓库里的 editor crate**（Zed 的 editor 与整个 Zed 工程深度耦合，抽出来成本极高）。

---

## 1. 目标与非目标

### MVP 目标（第一期）
- 打开工作区 → 左侧文件树 + 中间代码编辑器（多 Tab）+ 右侧 AI 对话。
- 编辑器：tree-sitter 高亮、编辑/保存（原子写）、多标签、未保存提示。
- AI 对话：流式输出、多轮、选模型、把当前文件/选区作为上下文。
- 内嵌终端：工作区目录开 shell。
- 设置：API Key（系统钥匙串）、模型、主题。

### 非目标（后置）
- 全语言 LSP 智能（先只接 1–2 门语言）；插件体系；协作；全平台打包（先 Windows）。

---

## 2. 技术栈（方案 A，全部 Rust）

### GUI / 编辑器（核心）
| 用途 | 选择 | 说明 |
|---|---|---|
| GUI 框架 | **`gpui` 0.2.x** | Zed 内核，GPU 加速，retained+immediate 混合 |
| 组件库 | **`gpui-component` 0.9.x**（或总成 `gpui-kit` 0.7） | 75+ 组件、Dock 布局、主题 |
| 代码编辑器 | **GPUI Kit 的 Code Editor**（feature `tree-sitter` / `lsp`） | 自带高亮/诊断/补全/虚拟滚动 |
| 基础库 | **`gpui-base` 0.9.x** | 行为、状态、基础设施（若要自建视觉系统） |

> 建议 Cargo 依赖只写一行：`gpui-kit = { version = "0.7", features = ["tree-sitter", "lsp"] }`（具体 feature 名以 `cargo add` 解析结果为准），避免 gpui 版本错配——gpui-kit 会 pin 匹配的 gpui。

### 语法 / 文本
- `tree-sitter` 0.27.x + `tree-sitter-<lang>`：结构化抽取（给 AI 提供符号上下文）。
- `ropey`：大型文本缓冲区（若自建编辑器；用 GPUI Kit 编辑器则多半内置）。

### 文件 / 工作区
- `ignore`：gitignore 感知遍历（文件树 + 搜索）。**必用。**
- `notify` + `notify-debouncer-full`：文件监听。
- 原子写：临时文件 + rename（对齐 Proma safe-file 思路）。

### AI / LLM
- `reqwest`（`json` + `stream`）：HTTP。
- `tokio` + `tokio-util`（codec）：异步运行时。
- **`eventsource-stream`**：SSE 流式解析（主流 LLM 都是 SSE）。
- `serde`/`serde_json`：协议。
- `keyring`：API Key 存系统钥匙串。
- Provider trait 自写（OpenAI 兼容 / Anthropic / Ollama）。

### 终端
- **`portable-pty`**（wezterm）：跨平台 PTY，Windows 走 **ConPTY**。
- 终端视图：GPUI Kit 无内置终端 → **自建 xterm 风格控件**（这是方案 A 的一个自建点，用 GPUI 的 Element 绘制等宽网格 + 字节流渲染）。或用 `alacritty_terminal` 做 VT 解析，GPUI 负责渲染。

### Git / 搜索
- `git2`（libgit2）或 `gix`（纯 Rust）；M1 可先调系统 `git` CLI。
- `grep-searcher` + `grep-regex` + `ignore`：全文搜索（ripgrep 内核）。

### 通用
- `anyhow`/`thiserror`、`tracing`、`directories`/`dirs`、`uuid`、`chrono`、`serde`+`toml`/`json`（本地优先持久化，不上数据库）。

---

## 3. 架构与模块

```
my-ide/
├── Cargo.toml            # workspace
├── crates/
│   ├── app/              # GPUI 应用入口、窗口、路由
│   ├── ui/               # 主题、布局(三栏)、通用控件（终端控件等）
│   ├── workspace/        # 文件树、watcher、原子写
│   ├── editor/           # 编辑器状态（基于 GPUI Kit Code Editor 的封装）
│   ├── agent/            # LLM 客户端、会话、工具调用、SSE 流
│   ├── terminal/         # portable-pty + VT 解析(alacritty_terminal)
│   ├── gitkit/           # git status/diff
│   ├── search/           # grep-* 全文搜索
│   └── core/             # 类型、错误、配置
└── assets/               # 图标、字体、主题
```

> GPUI 是"实体(Entity) + View(Render) + Element"模型，**没有 WebView、没有 JS**，状态用 Entity + `Context`，异步用 `cx.spawn`。所有 UI 写 Rust。

---

## 4. 分阶段计划（含验收）

> 方案 A 比 Tauri 慢，但 GPUI Kit 把组件和编辑器补上了，主要风险在 Windows 下 GPUI 平台层和自建终端。

### M0　地基（约 1–1.5 周）
- `cargo new` workspace；引入 `gpui-kit`，跑通**官方 hello_world / story**。
- 三栏骨架（左/中/右）用 Dock 布局跑起来。
- 错误类型 + `tracing` 日志。
- **验收**：`cargo run` 在 **Windows** 打开三栏空窗口；`cargo clippy -D warnings` 通过。

### M1　工作区 + 编辑器（约 2.5–3 周）
- 打开文件夹（GPUI 的 `prompt_for_paths`）。
- 文件树（`ignore` 遍历 + 懒加载 + watcher）。
- 接入 GPUI Kit Code Editor：tree-sitter 高亮、多 Tab、保存（原子写）、未保存提示。
- **验收**：打开真实项目，浏览/编辑/保存/外部改动刷新；1 万行文件滚动不卡。

### M2　AI 对话（约 2.5 周，核心）
- Provider trait + OpenAI 兼容先行；`reqwest` + `eventsource-stream` 流式。
- 后台 task 逐块推送到 Entity → UI 刷新（用 GPUI 的 `cx.spawn` + 通道）。
- 对话面板：多轮、Markdown 渲染（GPUI Kit 自带富文本）、中止。
- 上下文注入：当前文件/选区/附加文件。
- API Key → `keyring`；设置页。
- **验收**：配网关后流式对话；能带当前文件上下文答代码问题；可中止。

### M3　终端 + Agent 工具（约 2.5–3 周，方案 A 的自建重点）
- `portable-pty` 起 shell（Windows PowerShell/ConPTY）；`alacritty_terminal` 解析 VT；GPUI Element 渲染网格。
- 键盘输入、尺寸同步、滚动回看。
- Agent 工具：read/write/list/run_terminal（默认需确认）。
- **验收**：内嵌终端可用（vim/top 类 TUI 基本可用）；AI 能读文件、确认后写、执行命令回显。

### M4　编码体验（约 2–3 周）
- 全文搜索面板（grep-*）。
- 符号大纲（tree-sitter query）。
- Git status + diff（先 git CLI）。
- 命令面板 + 快捷键（GPUI 的 Actions/Keymap）。
- **验收**：搜索、大纲、看 git 改动、命令面板均可键盘驱动。

### M5　打磨与打包（约 1.5–2 周）
- 设置持久化、首启引导、崩溃恢复、自动更新（自建或接 `self_update`）。
- Windows 打包（MSI/NSIS，可用 `cargo-wix` 或 `cargo-packager`；GPUI 本身无 Tauri 式打包器）。
- **验收**：产出可安装的 Windows 包，安装后 MVP 功能可用。

> 总估：**12–15 周**（单人全职理想值，Windows 平台问题与自建终端是主要不确定项）。

---

## 5. 方案 A 的真实风险（必须心里有数）

| 风险 | 说明 | 缓解 |
|---|---|---|
| GPUI 版本快变 | 0.2.x 仍 pre-1.0，API 常破坏性变更 | 用 `gpui-kit` pin 版本，跟随其升级节奏 |
| Windows 平台层 | GPUI 主力在 mac；Windows 有 D3D/Blade 后端但边缘 bug 多 | 每个里程碑都在 Windows 跑；遇坑查 Zed windows 文档与 issue |
| 无 WebView 生态 | 富文本/图表/终端都要 Rust 控件 | 尽量用 GPUI Kit 现成组件；终端是唯一必须自建的大件 |
| 文档少 | GPUI 教程少，主流是读 Zed 源码 | 用 GPUI Kit 的 docs 站与 skills；先跑通 story 示例 |
| 学习曲线 | Entity/Context/Element 心智模型 | M0 就打通最小闭环，逐步扩 |
| 许可证 | GPUI = Apache-2.0；GPUI Kit = Apache-2.0（文档 CC BY 4.0）；Zed 编辑器本体 = GPL-3.0。**不要复制 Zed 的 editor crate 源码**（GPL 传染），只依赖 Apache 的 gpui/gpui-kit | 只 `cargo add` Apache-2.0 的 crate，不粘贴 Zed 源码 |

---

## 6. 环境前置（本机）

- Rust：**未安装** → `rustup` + MSVC（VS2022 Build Tools 已在）。**必须用 stable 最新版**（GPUI 要求）。
- GPU：GPUI 走 GPU 渲染，需可用显卡驱动（本机需确认；虚拟机/远程桌面下可能有问题）。
- ConPTY：Win10 1809+（终端模块依赖）。
- 无 WebView2 需求（方案 A 不依赖，这点比 Tauri 好）。

---

## 7. 我对方案 A 的建议（务实）

1. **别照抄 Zed**。Zed 编辑器 crate 是 GPL-3.0 且与整个 Zed 深度耦合，抽离成本极高且有许可证问题。**依赖 `gpui-kit`（Apache-2.0）**，用它的 Code Editor + Dock 布局。
2. **把"终端"当成独立攻坚项**，它没有现成 GPUI 控件，是方案 A 唯一真正的大自建。
3. **M0 必须先确认 Windows 上 GPUI 能稳定跑**（这是全盘前提）。跑不通就要么换 WSL/Linux 开发，要么回退方案 B。
4. **编辑器能力分级**：先用 GPUI Kit 的 Code Editor（高亮 + 基本编辑），LSP 补全作为 M4 增强，不要一开始就追平 VSCode。

---

## 8. 待确认（回答后即可拆任务开工）

1. **开发/运行平台**：就是这台 Windows 直接跑 GPUI，还是接受用 WSL/Linux 开发（Windows 只是最终运行目标）？
2. **首发语言**：编辑器首期只做哪门语言的 LSP？（建议 Rust / TypeScript 二选一）
3. **AI 渠道**：只 OpenAI 兼容网关，还是也要 Anthropic / 本地 Ollama？
4. **终端优先级**：M3 就做，还是放到 MVP 之后（可先不做终端，用外部终端）？

> 前 3 个是硬阻塞。确认后我把 M0–M5 拆成可派发的任务卡（目标 / 新文件 / 验收命令），交给实现 Agent。
