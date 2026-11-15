# 方案 A（纯 Rust + GPUI）—— 已通过环境验证的落地计划 v2

> 用户在 2026-11 确认：①在本机 Windows 直接开发运行 ②纯 Rust ③主流 AI 全部兼容 ④终端待定。
> 本文已在本机实测：Rust 工具链可用、**GPUI 在本机（GameViewer 远程会话）成功创建并渲染 GPU 窗口**。

---

## 0. 已完成的实地验证（这是最关键的部分）

在本机做了 M0 冒烟测试，结果如下：

| 验证项 | 结果 |
|---|---|
| Rust 工具链 | ✅ `rustc 1.100.0` / `cargo 1.100.0`（stable-msvc），VS2022 Build Tools 已用 |
| GPUI 依赖编译 | ✅ 709 个 crate，首次全量 5m55s（增量会快很多） |
| **GPUI 窗口创建** | ✅ 可见窗口 613x436，位于桌面中央，无崩溃 |
| **GPU 渲染初始化** | ✅ 无报错、无黑屏迹象，blade/D3D 后端正常工作 |
| 远程会话兼容性 | ✅ 在 **GameViewer 远程桌面 / console 已断开** 的环境下仍能正常出窗 |

**结论：方案 A 在本机可行，GPU-only 渲染成立。** GPUI 本身没有 CPU 软件渲染回退，所以"前期只用 GPU"≈ GPUI 的默认形态，无需额外取舍。

> 注：冒烟测试产物在 `C:\Users\Administrator\AppData\Local\Temp\opencode\gpui-smoke`，可删除。

---

## 1. 你的四项决策（已锁定）

1. **开发/运行平台**：本机 Windows 直接跑（已实测可行）。
2. **语言**：纯 Rust，零 JS/WebView。
3. **AI 渠道**：**主流 AI 全部兼容**——
   - OpenAI 兼容（含各类中转网关、DeepSeek、Qwen、GLM、Moonshot 等）
   - Anthropic（Claude）
   - Google Gemini
   - 本地：Ollama / OpenAI 兼容本地服务
   - → 采用 **provider trait + 每厂商适配器**，统一成内部消息/流式模型。
4. **终端**：待定 → **不阻塞 MVP**，M3 可选，先预留接口。

---

## 2. 技术栈（全 Rust，已核对 2026-11 现状）

- GUI：**`gpui` 0.2.2**（Apache-2.0，Zed 官方 crate）
- 组件/编辑器：**`gpui-kit` 0.7**（内含 `gpui-component` 0.9.1，75+ 组件；含 **Code Editor**（tree-sitter 高亮 + LSP + 20 万行性能）与 **Dock 布局**）
- 语法：`tree-sitter` 0.27 + `tree-sitter-<lang>`
- 文件：`ignore` + `notify` + `notify-debouncer-full`
- AI：`reqwest`(json,stream) + `tokio` + `eventsource-stream`（SSE）+ `serde`；密钥 `keyring`
- 终端（待定）：`portable-pty` + `alacritty_terminal`
- Git/搜索：`git2` 或 `gix`；`grep-searcher`+`grep-regex`+`ignore`
- 通用：`anyhow`/`thiserror`、`tracing`、`directories`、`serde`+`toml/json`（本地优先，不上数据库）

**许可证红线**：只依赖 Apache-2.0 的 `gpui`/`gpui-kit`；**不要复制 Zed 编辑器源码**（GPL-3.0 传染）。

---

## 3. 多模型 Provider 设计（对应"主流 AI 全部兼容"）

```rust
// 内部统一模型，与厂商解耦
struct ChatRequest { model: String, messages: Vec<Msg>, tools: Vec<Tool>, stream: bool }
enum Delta { Text(String), ToolCall(ToolCall), Done(Usage), Error(String) }

#[async_trait]
trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;                       // "openai" / "anthropic" / "gemini" / "ollama"
    async fn stream(&self, req: ChatRequest) -> Result<impl Stream<Item = Delta>>;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
}
```

实现清单（MVP 顺序）：
1. **OpenAICompat**（一套代码覆盖最多厂商：OpenAI、DeepSeek、Qwen、GLM、Moonshot、各类网关、Ollama 的 openai 兼容口）
2. **Anthropic**（messages API，SSE）
3. **Gemini**（generateContent 流式）
4. **Ollama 原生**（可选，openai 兼容口已能覆盖则不单做）

统一要点：把各家不同的事件协议与角色/工具调用格式，映射到内部 `Msg` / `Delta`；错误与限流统一。

---

## 4. 架构 / 模块

```
my-ide/
├── crates/
│   ├── app/        # GPUI 入口、窗口、路由
│   ├── ui/         # 主题、三栏 Dock 布局、通用控件
│   ├── workspace/  # 文件树、watcher、原子写
│   ├── editor/     # 基于 gpui-kit Code Editor 的封装
│   ├── agent/      # provider trait + 适配器 + 会话 + 工具调用 + 流
│   ├── terminal/   # (待定) portable-pty + VT
│   ├── gitkit/     # git status/diff
│   ├── search/     # grep-* 全工作区搜索
│   └── core/       # 类型、错误、配置
└── assets/
```

---

## 5. 里程碑（已按"终端待定"调整）

### M0　地基（约 1 周）— **已完成大半**
- ✅ Rust 工具链、GPUI 编译与出窗验证。
- 待做：建 workspace、引入 `gpui-kit`、三栏骨架能跑、错误类型 + 日志。
- **验收**：`cargo run` 打开三栏窗口；`cargo clippy -D warnings` 通过。

### M1　工作区 + 编辑器（约 2.5–3 周）
- 打开文件夹、文件树（`ignore`+watcher）、GPUI Kit Code Editor（高亮、多 Tab、原子写保存、未保存提示）。
- **验收**：真实项目可浏览/编辑/保存/外部改动刷新；1 万行滚动不卡。

### M2　AI 对话 + 多 Provider（约 3 周，核心）
- provider trait + OpenAICompat + Anthropic + Gemini；SSE 流式；中止。
- 对话面板（Markdown、代码块）；上下文注入（当前文件/选区/附件）。
- API Key → `keyring`；设置页（选 provider/模型/base_url）。
- **验收**：三家 provider 均可流式对话；能带当前文件上下文答代码问题；可中止。

### M3　（可选）终端 + Agent 工具（约 2.5–3 周）
- 终端按你决定取舍。
- Agent 工具：read/write/list/run_cmd（默认需确认）。
- **验收**：AI 能读文件、确认后写、按需执行命令。

### M4　编码体验（约 2–3 周）
- 全文搜索、符号大纲（tree-sitter）、Git status/diff、命令面板 + 快捷键。

### M5　打磨打包（约 1.5–2 周）
- 持久化、首启引导、崩溃恢复、更新；Windows 安装包（`cargo-packager`/`cargo-wix`）。

> 总估：**11–15 周**（不含终端则少 2–3 周）。

---

## 6. 风险与对策

| 风险 | 对策 |
|---|---|
| GPUI pre-1.0 API 变更 | 用 gpui-kit pin 版本，跟随其节奏升级 |
| Windows/远程会话偶发平台 bug | 已实测可出窗；每个里程碑在本机复验 |
| 自建终端（若做） | 唯一大自建，单列 M3，可延后 |
| 首次编译慢（~6min 起） | 正常；增量快；CI 可缓存 target |
| 许可证 | 只用 Apache 的 gpui/gpui-kit，不粘 Zed GPL 源码 |

---

## 7. 下一步（无需再等决策，可直接开工）

你的四项已锁定，环境已验证。**我建议直接启动 M0 建仓**：搭 workspace + 引入 gpui-kit + 三栏骨架 + 日志/错误类型。

需要你确认一句话即可：
- **是否现在就让我把 M0–M5 拆成可派发的任务卡，并开始执行 M0（建仓与骨架）？**
- 项目名/包名想叫什么？（默认可用 `proma-rs` 或你指定的名字）
