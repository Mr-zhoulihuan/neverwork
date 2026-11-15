# NeverWork

纯 Rust + GPUI 构建的 AI 原生 IDE。第一期：本地 IDE + 多 Agent 自定义团队 + 远程工作区连接（不含跨设备同步）。

## 运行

```bash
# 需要 Rust stable + MSVC 工具链（本机已装 rustc 1.100.0）
cargo run -p neverwork

# 预览 Agent 团队编辑器页面
set NW_PAGE=team && cargo run -p neverwork
```

## 目录

```
crates/
  core/   领域模型（工作区 / 团队 / 角色 / 会话 / 消息）+ 配置持久化 —— 纯数据，可序列化
  ui/     设计令牌 theme.rs + 组件库 components.rs
  agent/  LLM Provider（OpenAI 兼容 / Anthropic）+ 团队编排 team.rs
  app/    main.rs 入口
          state.rs          应用状态与行为（工作区/会话/键盘/发送）
          text_buffer.rs    统一文本缓冲区：光标/选区/撤销重做（含单测）
          input_handler.rs  IME 与文本插入的唯一入口（含单测）
          keymap.rs         按键 → 编辑命令路由（含单测）
          line_cache.rs     按行缓存高亮，避免每次按键全量重算（含单测）
          highlight.rs      轻量词法高亮
          filesystem.rs     目录遍历（按需展开）/ 读写 / 最近改动
          data.rs           演示数据
          views/            sidebar / chat / team / editor / files / palette
design/   设计稿（HTML/PNG）与设计规范 DESIGN-SPEC.md
```

## 待办（后续里程碑）

- M1+：文件监听（`notify`）自动刷新、真实语法高亮、编辑器内编辑保存
- M2：多 Provider AI 对话（OpenAI 兼容/Anthropic/Gemini，SSE 流式）
- M2.5：团队编辑器交互（增删角色、拖拽排序、持久化）
- M3：远程 Workspace Server 连接协议
- ~~跨设备同步~~：第一期不做

## 已实现（对齐设计稿）

### M0 — 界面骨架
- **主界面**（`state.rs`，对齐 `design/nw-main-v3`）：三段式布局
  - 左栏 236px：品牌 + 状态、工作区连接卡（真实路径/文件数）、Agent/Chat 切换、会话列表、设备底栏
  - 中栏：会话标题 + 角色点、协作流横幅、多 Agent 对话流、底部主输入框
  - 右栏 300px：文件/改动/预览 Tab + **真实文件树**
- **Agent 团队编辑器**（`team_view.rs`，对齐 `design/nw-agent-team`）

### M1 — 真实文件系统 + 编辑器
- **真实读取工作区**：`filesystem.rs` 递归遍历目录（忽略 `.git`/`target`/`node_modules` 等）。
- **交互式打开文件**：点击右侧文件树 → 读取文件内容 → 中栏切换到编辑器视图（带行号）。
- **返回对话**：编辑器顶部「← 对话」按钮切回多 Agent 对话。
- **状态化管理**：应用根为有状态 `Entity<AppState>`（GPUI 的 Entity/Context/listener 模型）。

### M2 — 多 Provider AI 对话（SSE 流式）—— 本期
- **`neverwork-agent` crate**：
  - 统一消息模型 `ChatMessage` + `LlmProvider` trait + `StreamEvent`（Delta/Done/Error）。
  - **OpenAI 兼容** Provider（覆盖 OpenAI / DeepSeek / Qwen / GLM / Moonshot / Ollama）：`/chat/completions` 流式，SSE 解析 `delta.content`。
  - **Anthropic** Provider：`/v1/messages` 流式，SSE 解析 `content_block_delta`。
  - 基于 `ureq`（阻塞 + 流式读取），后台线程运行，与 GPUI 执行器不冲突。
- **流式 UI**：输入框可编辑（字符/退格/Enter 发送），发送后追加用户消息 + AI 占位消息，`flume` 通道 + `cx.spawn` 逐块追加到对话流；流式期间显示「生成中…」。
- **配置**（环境变量）：`NW_API_KEY`（必填）、`NW_PROVIDER`=`openai|anthropic`、`NW_BASE_URL`、`NW_MODEL`。未配置时在输入框上方给出友好提示。
- **测试**：SSE 解析单测 + 本地 mock HTTP 服务端到端流式测试（`crates/agent/tests/streaming.rs`）全部通过。

### M2.5 — 多 Agent 团队协作 + 团队编辑器（本期）
- **多角色编排 `run_team`**（`crates/agent/src/team.rs`）：
  - 流水线：按角色顺序执行，后一角色可参考前一角色产出（上下文累积）。
  - 并行：同时向所有角色发起请求，各自独立流式。
  - 每角色独立 system prompt + 模型；模型名以 `claude` 开头自动切到 Anthropic。
  - 支持中途取消（`AtomicBool`）。
- **对话里按角色身份流式**：发送消息 → 团队按编排运行 → 每个角色开始发言时创建带其身份色/头像/名字的消息，增量逐字追加。
- **团队编辑器（可交互）**：中栏切到「团队编辑器」——切换编排（流水线/并行/主管）、**换模型**（循环常用模型）、**增删角色**、保存。
- **持久化**：`~/.neverwork/config.json`（原子写：临时文件 + rename），支持 `NW_CONFIG_DIR` 覆盖。
- **测试**：`crates/agent/tests/team_flow.rs`（流水线顺序执行 + 取消）与 `crates/core/tests/config_test.rs`（配置 roundtrip）全部通过。

### M1+ — 可编辑编辑器 + 语法高亮 + 文件监听（本期）
- **编辑器可编辑**：字符输入、退格、回车、左右方向键移动光标（UTF-8 边界安全）；当前行高亮；`Ctrl+S` 或「保存 ⌘S」按钮保存（**原子写**：临时文件 + rename）；未保存时标签显示 `●`。
- **轻量语法高亮**（`highlight.rs`）：按扩展名识别 Rust / TS / Python / JSON / TOML；着色 关键字(紫)/字符串(橙棕)/注释(绿)/数字(浅绿)/类型(青)/函数调用(黄)；不引入重型 parser，构建快。
- **文件监听**（`notify`）：后台线程监听工作区，防抖 300ms，外部改动自动刷新文件树。
- **测试**：`highlight.rs` 内 6 个单测（keyword/string/comment/number/type/func/python-comment）全部通过。

### M3 — 输入与交互修正（本期重点）

上一版存在「输入框写不好、点了没反应」的实际缺陷，本期按根因重写：

- **输入收敛为单一通道**：可打印字符与中文输入法一律走 `EntityInputHandler`，
  键盘只处理非文本按键。删掉了 `on_key_down` 里手写的 `key_char → push_str`，
  它和 IME 同时写入导致字符重复/丢失。
- **统一 `TextBuffer`**：光标（UTF-8 字节偏移、始终落在 char boundary）、
  选区、撤销/重做（整串快照，栈深 400）、按行/按词移动、行首行尾，
  单行与多行共用一套语义。对话输入框终于能改中间的字了。
- **编辑器可见光标**：每个字符位置都能精确绘制 caret；之前完全没有光标。
- **按字符点击定位**：点哪个字符就落在哪个字符前（之前只能跳到行首）。
- **选区与剪贴板**：Shift+方向键、Ctrl+A、Ctrl+C/X/V、Ctrl+Z/Shift+Z、
  Tab/Shift+Tab 缩进、Ctrl+S 保存。
- **文件树真正可展开**：`FileNode.expanded` 之前写了没人读，目录点不开；
  现在点击展开/折叠，并修复同名文件点错（改用相对路径而非按文件名搜索）。
- **补齐所有死点击**：右侧「文件/改动/预览」Tab、会话列表、新建会话、
  工作区「切换 ▾」、标题栏「⌘K」命令面板——之前只有样式没有行为。
- **性能**：`uniform_list` 虚拟滚动 + `LineCache` 行级高亮缓存，
  大文件不再每帧全量重排。
- **响应式 + 专注模式**：窗口较窄或高 DPI 时自动收起侧栏，
  标题栏「◧ 专注」可手动切换。

测试：129 个。app 侧 text_buffer 11 / text_fit 5 / model_picker 10 / settings_ops 3 / skills 1 / input_handler 4 / keymap 3 / line_cache 3 / filesystem 2；agent 侧 tools 17 / diff 12 / team role_provider 5 / sse 2；core 侧 settings 13 + config 5；ui 3。另有 3 条打真实 DeepSeek API 的集成测试（需密钥，默认跳过）。

### M7 — 步骤折叠 / 右栏收起 / 查找替换 / 团队带工具（本期）

**1. 工具步骤折叠成一行（对齐参考稿）**

连续的工具调用不再一人一张大卡片，而是聚合成一条紧凑的步骤条：

```
▸ ● 读取了文件 readme.txt 等 3 步        +2 −1
▾ ● 浏览了目录 · 1 步
    ▤ list_dir
      目录（1 项）
      readme.txt
```

- 折叠态一行：箭头 + 状态点（绿完成 / 黄待确认 / 红失败 / 灰拒绝）+ 人话标题 + 增删行数。
- 点一下展开，逐步显示工具名、参数、状态、diff 或输出。
- **待审批的组永远自动展开**，否则用户看不到「允许 / 拒绝」按钮。

**2. 右栏可收起**

右栏头部右侧 `▶` 收起成 16px 窄条，窄条上的 `◀` 再展开。与「专注模式」的区别：
专注模式连左栏一起收起，这里只让出右栏，适合「要宽一点但还要看会话」。

**3. 编辑器查找 / 替换**

`Ctrl+F` 查找、`Ctrl+H` 查找并聚焦替换框：

- `Aa` 开关区分大小写；输入即搜，右侧显示 `当前/总数`。
- Enter 下一个、Shift+Enter 上一个、Esc 关闭；命中会真的选中并滚动到位。
- 「替换」只替换当前命中，「全部替换」一次改完并提示次数。
- 匹配逻辑抽在 `app/src/find.rs`，纯函数 + 12 个单测（含中文与大小写）。

**4. 多角色团队也能用工具**

开启工具后团队成员**串行**执行：前一个角色的产出作为后一个的上下文，
这样「规划 → 编码 → 审查」在带工具时依然成立。串行是刻意的——
并发写同一个文件只会互相覆盖，无法逐个审批。

**5. 补丁清单**

右栏「改动」页顶部新增「Agent 改动 · N 个文件」，按文件合并增删行数、
标出被拒绝的文件，点一下直接打开该文件。聚合逻辑抽成
`core::aggregate_changes`（纯函数，4 个单测）。

### M6 — Agent 工具调用（真读、真写、真 diff）

Agent 现在能直接操作工作区文件，而不是只会「说话」。

**工具集**（`agent/src/tools.rs`）

| 工具 | 作用 | 是否需批准 |
|---|---|---|
| `read_file` | 读文件，返回带行号内容（可 offset/limit） | 否 |
| `list_dir` | 列目录 | 否 |
| `search_files` | 按文本搜索，返回 `文件:行号: 内容` | 否 |
| `write_file` | 整体写入（新建/覆盖），原子写 | **是** |
| `apply_patch` | 把**唯一存在**的一段原文替换掉 | **是** |

**安全模型**

- 所有路径被限制在工作区内：`..` 逃逸、跨盘绝对路径、符号链接逃逸一律拒绝并报错。
- 读操作直接执行；**写操作必须先展示 diff、经用户点「允许并写入」才落盘**。
  拒绝后文件保持原样，且把「用户拒绝了」回填给模型，让它换方案而不是重试。
- 不提供 shell 执行。跑命令的风险和收益不成正比，请用独立终端。
- 单轮最多 8 次工具循环，避免模型陷入死循环烧 token。

**交互**

- 每次工具调用渲染成一张卡片：工具名、参数摘要、状态（执行中/待确认/完成/已拒绝/失败）、
  `+n −m` 行数；写操作内嵌 diff（`+` 绿、`-` 红、上下文灰，超过 220px 可滚）。
- 待确认卡片橙色描边 + 「拒绝 / 允许并写入」按钮；输入框上方同时有一条黄色横幅
  写明「等待你确认：write_file xxx」。
- 输入框有「⚒ 工具已开/已关」开关：关掉就退回纯对话。

**协议层**（`agent/src/openai.rs`）

- 请求带 `tools` + `tool_choice: auto`；流式解析 `delta.tool_calls[]`，
  按 `index` 累积被切碎的 `function.arguments`，靠 `finish_reason` 或流结束收尾。
- 兼容推理模型：`deepseek-flash` 会先返回 `reasoning_content`，直接忽略、不影响正文与工具调用。

**顺带修掉的两个真问题**

- 团队角色的 `model` 字段（`Claude Opus` / `GPT-5`）被当成真实模型 id 用于切厂商，
  会把 DeepSeek 的 Key 发到 Anthropic 去。现在只有**全小写的真实 id**（`claude-sonnet-4-5`）
  才覆盖模型名，展示标签一律忽略。
- 新消息到达时对话没有自动滚到底，用户以为「没反应」。现在内容更新后在本帧布局完成后
  请求滚动，而不是在旧布局上滚动。

**验证**

`agent/tests/deepseek_tools.rs` 打真实 DeepSeek API，跑三条端到端用例（需 `NW_DEEPSEEK_KEY`）：

1. 读文件后回答问题 —— 断言确实调了 `read_file`，回答包含文件内容
2. 批准后创建文件 —— 断言磁盘上真的出现了文件与内容
3. 拒绝写操作 —— 断言文件字节完全未变

### M4 — 左侧栏重做（对齐 pic/ 参考稿）

参考 `pic/Snipaste_2026-11-15_13-19-46.png` 的成品形态，把左侧栏从「一张大工作区卡 + 几行纯文字」
改成「窄图标栏 + 会话卡片列表」：

- **48px 图标栏**：品牌标 + 对话 / 团队 / 编辑器 / 命令四个入口（当前项高亮），
  底部是 AI 就绪状态与用户头像（点开命令面板）。全局入口不再散落在正文里。
- **会话卡片**：每张卡片两行——第一行是身份色头像、可选分支徽标（`⑂`）、
  标题，以及**右上角状态圆圈**（黄 `▶` 可继续 / 黄 `◉` 可重放 / 蓝 `✓` 已完成 /
  灰 `✕` 失败，本身可点击）；第二行是摘要、更新时间和状态胶囊
  （`4m 42s` 绿 / `已完成` 蓝 / `已废弃` 灰）。
  卡片无边框，靠底色区分；选中项底色更亮。
- **工作区压成一条细条**：`▤ 名称 · N 项 · 切换`，点击即可切换工作区，
  把纵向空间让给会话。列表底部是会话计数与 `⋯` 更多菜单。
- **文本按像素预算截断**（`views/text_fit.rs`）：gpui 的 `.truncate()` 在嵌套 flex 里
  拿不到确定宽度，会直接硬裁而不出省略号；改为按字符宽度预算截断并补 `…`，
  长标题、长摘要、长工作区名都不会再溢出到中栏。

`Session` 相应扩展了 `preview / status / meta / accent / icon / badge / action` 字段（均带 serde 默认值），新增 `SessionStatus`（Idle/Working/Done/Dropped）与 `SessionAction`（None/Resume/Replay/Done/Failed）两个枚举。

### M5 — 设置 / 技能市场 / 多厂商 / 同步升级（本期）

- **设置页**（图标栏 ⚙ 或 ⌘K）：五个分区，全部即时生效并自动落盘到 `~/.neverwork/settings.json`。
  - **常规**：主题（深色 / 浅色 / 跟随系统，运行时切换整套色板）、界面缩放、编辑器字号、
    缩进宽度、行号显示、自动保存（停止输入 800ms 落盘）、配置目录、恢复默认。
  - **模型与厂商**：内置 10 家主流厂商（OpenAI / Anthropic / Gemini / DeepSeek / 通义千问 /
    智谱 GLM / Moonshot / 豆包 / 百川 / Ollama 本地），每家预填基址与模型列表；
    填 API Key 即可用，可设当前厂商、启用停用、调温度；支持添加自定义厂商
    （任意 OpenAI 兼容端点，如公司中转）。
    **模型选择是可搜索的内联面板**（不是逐个循环点击）：点模型行原地展开，
    输入即按分段模糊匹配过滤（`4o mini` 命中 `gpt-4o-mini`），↑↓ 选择、Enter 确认；
    搜索词不在列表里时回车直接当自定义模型名。模型再多也一两次按键到位。
  - **技能**：已安装技能列表，可卸载，一键跳到市场。
  - **同步与升级**：对比远端仓库新提交（`git rev-list HEAD..origin/main`）、一键
    `git pull --ff-only`；配置导出成带时间戳的 JSON 快照、按路径导入恢复。
- **密钥脱敏**（安全）：
  - 导出/同步默认**不含** API Key（`ExportOptions::default()`），需要完整迁移时
    必须在界面上显式打开「导出包含 API Key」，并给出明文风险提示。
  - 导入脱敏文件时按厂商 id 保留本地已填的 Key，不会因为点一次导入就把密钥清空。
  - 界面只显示 `••••••••` + 末尾 4 位，前缀也不显示（避免截图/录屏泄露）。
  - `ProviderEntry` 手写 `Debug`，任何 `{:?}` 日志都只输出打码后的 Key。
  - **关于**：版本、依赖、仓库、统计。
- **技能市场**（图标栏 ✦ 或 ⌘K）：
  - 顶部「技能 / Agent 模板」两个 Tab；搜索框按名称、id、简介、作者过滤。
  - 分类预览卡（办公效率 / 知识管理 / 开发编程 / 内容创作 / 设计多媒体），各显示安装量最高的 4 个。
  - 11 个分类筛选 + 三种排序（综合评分 / 安装量 / 星标），45 个内置技能，卡片网格。
  - 安装/卸载即时写入设置；Agent 模板可一键套用为当前团队（流水线三人组 / 并行探索团 / 单人对练）。
- **多协议**：`neverwork-agent` 的 `ProviderConfig` 增加 `temperature`，OpenAI 兼容与
  Anthropic 都带上；对话优先用设置里的当前厂商，未配置时提示去设置页，仍兼容 `NW_API_KEY` 等环境变量。
- **主题运行时切换**：`theme::set_theme_mode()` 写全局原子量，所有 `theme::xxx()`
  立刻返回新色板的颜色，视图代码零改动。

新增/扩展：`core/settings.rs`、`app/settings_ops.rs`、`app/skills_catalog.rs`、
`views/settings.rs`、`views/skills.rs`；`Session` 之外新增 `Settings` / `ProviderEntry` /
`ProviderPreset` / `Protocol` / `Skill` / `SkillCategory` / `ThemeMode`。

## 运行

```bash
# 主界面
cargo run -p neverwork
# 指定工作区根
set NW_ROOT=E:\path\to\project && cargo run -p neverwork
# 启用 AI 对话（OpenAI 兼容）
set NW_API_KEY=sk-xxx
set NW_MODEL=gpt-4o        # 可选
set NW_BASE_URL=https://api.openai.com/v1   # 可选
cargo run -p neverwork
# 或用 Anthropic
set NW_PROVIDER=anthropic && set NW_MODEL=claude-sonnet-4-5 && cargo run -p neverwork
# 直接进入团队编辑器（也可在界面点「编辑 ▾」）
set NW_VIEW=team && cargo run -p neverwork
# 启动即打开某个文件
set NW_OPEN=path\to\file.rs && cargo run -p neverwork
# 覆盖窗口尺寸（异常 DPI 环境下有用）；NW_MAXIMIZE=0 可关闭自动最大化
set NW_WIN_W=1200 && set NW_WIN_H=740 && cargo run -p neverwork
# 启动直达某页：chat|team|skills|settings（配合 NW_SETTINGS_TAB=general|models|skills|sync|about）
set NW_VIEW=settings && set NW_SETTINGS_TAB=models && cargo run -p neverwork
# 调试：启动即展开某个厂商的模型选择面板（下标从 0 开始）
set NW_PICK_MODEL=0 && cargo run -p neverwork
```

## 技术说明

- GUI：`gpui 0.2.2`（Zed 内核，GPU 渲染，Apache-2.0）
- 本机验证：rustc 1.100.0，应用成功编译运行并渲染出设计界面
- 注：当前开发机为 GameViewer 远程会话、200% DPI。GPUI 用 `GetDpiForWindow/96` 得到
  scale=2，而窗口按 1:1 物理像素创建，导致内容被放大一倍、右侧被裁。窗口尺寸受系统限制
  无法直接撑大，因此：①scale>1 时启动自动最大化，尽量露出完整宽度；
  ②布局用 `visible_w = bounds.width / scale` 做响应式折叠（<1100 收右栏、
  <820 只留中栏）；③标题栏「◧ 专注」可手动切换。常规显示器（scale=1）下三栏正常。
