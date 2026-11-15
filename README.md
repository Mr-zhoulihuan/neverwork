# NeverWork

纯 Rust + GPUI（Zed 内核）自绘的 AI 原生 IDE。界面、编辑器、多 Agent 协作全部自研，不依赖 WebView。

> 项目代码在 [`NeverWork/`](NeverWork/) 子目录（独立 Cargo workspace）。根目录只放计划文档、设计稿与截图。

## 快速开始

```bash
cd NeverWork

cargo run -p neverwork        # 启动主界面
cargo test --workspace        # 全部测试（40 个）
cargo build -p neverwork      # 只编应用
```

需要 Rust stable + MSVC 工具链（开发机为 rustc 1.100.0）。

启用 AI 对话前设置环境变量：

```bash
set NW_API_KEY=sk-xxx
set NW_MODEL=gpt-4o                          # 可选
set NW_BASE_URL=https://api.openai.com/v1    # 可选
set NW_PROVIDER=anthropic                    # 可选，配合 claude 模型
```

其他运行期变量：`NW_ROOT`（工作区根）、`NW_VIEW=team`（直接进团队编辑器）、`NW_OPEN=<path>`（启动即开文件）、`NW_CONFIG_DIR`（覆盖 `~/.neverwork`）。

## 目录

```
NeverWork/
  crates/
    core/    领域模型（Workspace / Team / Role / Session / Message）+ JSON 原子写配置
    ui/      设计令牌 theme + 基础组件库
    agent/   OpenAI 兼容 / Anthropic 双 Provider（SSE 流式）+ 团队编排
    app/     GPUI 界面
              state.rs          状态与行为（工作区 / 会话 / 键盘 / 发送）
              text_buffer.rs    统一文本缓冲区：光标 / 选区 / 撤销重做
              input_handler.rs  IME 与文本插入的唯一入口
              keymap.rs         按键 → 编辑命令路由
              line_cache.rs     行级高亮缓存
              highlight.rs      轻量词法高亮
              filesystem.rs     目录遍历 / 读写 / 最近改动
              views/            sidebar / chat / team / editor / files / palette
  design/   设计规范、HTML 设计稿与运行截图
```

## 已实现

- **三栏界面**：图标栏 + 会话卡片列表 / 对话与编辑器 / 文件树·改动·预览
- **多 Agent 团队**：角色可自定义（模型、提示词、身份色），流水线 / 并行 / 主管三种编排
- **流式对话**：Provider 抽象 + SSE 逐块渲染，支持中途停止
- **编辑器**：光标、选区、撤销重做、剪贴板、按字符点击定位、语法高亮、原子保存、虚拟滚动
- **文件树**：按需展开、最近改动、只读预览
- **命令面板**：⌘K

## 尚未实现

- Agent 工具调用（读文件 / 写文件 / 跑命令）、产出落盘成 diff
- `Supervisor`（主管指派）目前等同并行，未真正分派
- 查找替换、多光标、代码折叠
- 远程 Workspace Server（M3）
- LSP / tree-sitter / 终端 / Git 集成 / 全局搜索

详细设计与里程碑见 [`NeverWork/README.md`](NeverWork/README.md) 与 [`gpui-ide-plan-v2.md`](gpui-ide-plan-v2.md)。

## 开发环境说明

当前开发机为 GameViewer 远程会话 + 200% DPI：GPUI 用 `GetDpiForWindow/96` 得到 scale=2，
而窗口按 1:1 物理像素创建，内容会被放大并裁掉右侧。布局侧因此用
`visible_w = bounds.width / scale` 做响应式折叠（<1080 收右栏、<820 只留中栏），
标题栏「◧ 专注」可手动切换。常规显示器（scale=1）下三栏正常。

## License

Apache-2.0
