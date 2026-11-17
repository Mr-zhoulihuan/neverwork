# NeverWork

纯 Rust + GPUI 构建的 AI 原生 IDE。第一期：本地 IDE + 多 Agent 自定义团队 + 远程工作区连接（不含跨设备同步）。

## 运行

```bash
# 需要 Rust stable + MSVC 工具链（本机已装 rustc 1.100.0）
cargo run -p neverwork

# 预览 Agent 团队编辑器页面
set NW_PAGE=team && cargo run -p neverwork
```

## 打包（Windows 安装包）

```powershell
powershell -ExecutionPolicy Bypass -File installer\build.ps1            # 编 release + 打安装包
powershell -ExecutionPolicy Bypass -File installer\build.ps1 -SkipBuild # 复用已有 release 产物
```

产出 `dist\NeverWork-0.1.0-setup.exe`（约 5.5MB）：按用户安装到 `%LOCALAPPDATA%\Programs\NeverWork`、
不弹 UAC、可选桌面快捷方式；卸载保留 `%USERPROFILE%\.neverwork`（会话 / 设置 / Key）。
依赖 NSIS（`winget install NSIS.NSIS`）。应用图标已入库（`crates\app\assets\neverwork.ico`），编 exe 时由 `build.rs` 内嵌进资源段（任务栏 / 资源管理器直接显示）；改图标后跑 `python installer\make_icon.py` 重新生成。

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

测试：238 个。app 侧 text_buffer 11 / text_fit 5 / model_picker 10 / settings_ops 3 / skills 1 / input_handler 4 / keymap 3 / line_cache 3 / filesystem 2；agent 侧 tools 17 / diff 12 / team role_provider 5 / sse 2；core 侧 settings 13 + config 5；ui 3。另有 3 条打真实 DeepSeek API 的集成测试（需密钥，默认跳过）。

### M14 — 不止文件操作：问答与动手分开（本期）

**问题**

任何输入都会先去翻文件。问「Rust 的 lifetime 怎么理解」、问「深圳天气」，
回答都是「我只能在本地 IDE 工作区里读写文件，没法……」。根因有两层：

1. **系统提示词通篇讲文件**。开头就是「你正在用户的工作区里工作」，
   接着是六条文件工具说明，还有「改文件之前先用 read_file」。
   模型于是把每条输入都当代码任务处理。提示词从没说过「问答类问题直接答」。
2. **默认强制跑三角色流水线**。`tools_enabled` 默认开，而开工具就等于
   `spawn_agents_chain(team.roles)`——每句输入都过 Planner → Coder → Reviewer，
   三个角色各带一套文件工具。角色职责写死成「拆解任务 / 实现改动 / 审查」，
   纯问答必然被拗成改代码，还慢三倍、贵三倍。

**改法**

- **提示词先分类再动手**。明确两大类：A 提问/解释/讨论 → 直接回答、不动工具；
  B 在本工作区动手 → 才用工具。给出判据「用户想要结果，还是想要信息」，
  并说明工具「仅在 B 类任务时使用」。同时保留完整写工具说明，改代码能力不变。
- **会话级运行模式**（后续演进：全局 `team_mode` 开关已移除）。新建会话时二选一：
  单助手 → 一个助手，问答和改代码都由它按需判断；团队 → 多角色按交接流水线协作，
  且每个角色可绑定插件（技能）。模式存于 `Session.mode`，**创建后不可切换**；
  需要另一种模式就再新建一个会话。单助手不套任何角色 prompt。
- **关工具时不下发工具清单**。`run_agent` 增加 `enable_tools`，为 false 时
  不把 tool specs 发给模型，模型看不到工具就不会试着调，还省 token；
  提示词也追加一句「本次没有开启文件工具，不要声称读过或改过文件」。
- **界面跟着说真话**。单助手会话不挂流水线条（问常识时那条永远停在「步骤 2/3」很误导）；
  团队会话顶部流水线按当前团队 `roles` **动态生成**（不再硬编码「规划 → 编码 → 审查」），
  正在发言的角色高亮。右上角只展示当前模式，不提供切换。

**验证**（都是 Win32 真实注入 + 截图）

- 问 `What is a closure in Rust?` → 纯文本回答（含 `Fn`/`FnMut`/`FnOnce`、
  类型推断等要点），**零工具调用**，没有「浏览了目录」步骤条。
- 问 `List the top-level directories in this workspace` → 正常调
  `list_dir`/`read_file`，逐项说明 goDemo / nas-os / ops-go 各自是什么。

### M13 — 输入即可提问，AI 直接干活（本期）

**1. 为什么会「打不了字」**

三个独立原因叠在一起：

- **输入桥按「谁持焦点」挑宿主**。原来用 `is_focused` 三选一，只要焦点落到没有
  `track_focus` 的元素上（文件树、会话卡片、标题栏…），三个分支全不命中，
  `handle_input` 就不再注册输入处理器，键盘和输入法事件被直接丢弃。
  现在改成按 `input_target` 绑定宿主，并且每帧检查一次，掉焦就补回来——
  打开即能打字，不用先点输入框。
- **IME 组合态被算成空区间**。GPUI 在 Windows 上把 IME 光标位置塞进
  `new_selected_range_utf16`（`pos..pos` 折叠区间），旧代码拿它当选区算标记范围，
  得到空区间；下一次组合更新（`range=None`）找不到要替换的位置，就在光标处再插一遍。
  表现是「打一个字，前面整段重复一遍」，比如 `aa'ba'b'c`。
  现在整段组合串都标记为组合态（`composition_marked_range`）。
- **候选框位置取自 1×1 输入桥**。`bounds_for_range` 用 GPUI 传的 element bounds，
  而那是位于左上角的输入桥，候选框永远飘在窗口角落。现在用输入框自己的
  `input_area_bounds`（渲染期由 1×1 画布记录），中文候选框会贴住输入位置。

**2. 输入区放大**

单行输入框只有 22px 高、`text_sm` 字号，长句很难读。现在：

- 最小高度 52 → 48px（加内边距后整体约 100px，点击面积够大）；
- 字号 `text_sm` → `text_base`，行高 24px，光标同步加高；
- 外框左右内边距 `px_5` → `px_6`，框内 `px_4`；
- **空输入框也画光标**。之前 `empty` 分支只渲染占位文字、不渲染光标，
  默认状态看不出「这里已经可以打字」，用户得先点一下才敢输入。
  现在光标画在占位文字左侧（插入点在文本起始处的直觉位置）。

**3. 移除发送按钮**

发送本来就靠 Enter，按钮只是重复入口，还占着右下角。现在只在生成中出现
「■ 停止」，平时右下角是一行快捷键提示。

顺带修掉停止时的状态残留：`stop_streaming` 原来只置取消标志，`streaming` 要等
Agent 线程跑完当前工具才复位，界面上「生成中」和停止按钮会一直挂着（长工具调用
可达几十秒），用户会以为卡死。现在立刻复位，未完成的工具卡片落到新增的
`ToolRunStatus::Cancelled`（「已中断」），不再永远转圈。

**4. 右侧栏默认收起**

文件 / Git / 预览面板默认折叠成 16px 窄条，把宽度让给对话区；点窄条展开。
`NW_RIGHT_COLLAPSED=0` 可强制展开，便于验证。

验证方式：全用 Win32 `WM_CHAR` 消息真实注入键盘输入 + `design/shot.ps1` 截图，
确认「输入 → Enter → AI 调工具读仓库 → 给出回答」整条链路可用。

### M12 — 窗口尺寸与布局跟随（本期）

**1. 启动不再超出屏幕**

原来在高 DPI 下会「自动最大化」，窗口一启动就贴满整屏、又无法还原到正常大小。
现在：

- 默认按屏幕可用区 92% 开窗并居中，任务栏高度单独留出；
- `NW_WIN_W` / `NW_WIN_H` 覆盖值也会被夹回屏幕范围内（否则窗口能开到自己都看不见边缘）；
- 只有显式 `NW_MAXIMIZE=1` 才最大化；
- 最小窗口 660x440 逻辑像素，避免拖到极小后布局碎掉。

**2. 布局尺寸改为跟随窗口**

根容器原来写死 `.w(px(visible_w)).h(px(visible_h))`——尺寸取自上一帧算出的值，
拖动窗口时会先停在旧尺寸再加一次，观感就是「界面不跟随」。改成 `.size_full()`
之后容器永远等于窗口，`visible_w` 只用来判断断点（1080 收右栏 / 620 收会话列表）。

尺寸真变化时再补一次 `cx.notify()`，兜住 GPUI 在 Windows modal resize 循环里
偶尔漏掉的那一帧。

**3. 一个被误判的「DPI 放大」**

之前怀疑「`bounds()` 返回物理像素、渲染却按 scale 放大」，为此加过
「窗口比屏幕宽就除以 scale」的补丁。实测（Win32 `GetClientRect` + `scale_factor`）
显示 `bounds()` 一直是正确的逻辑值，界面并没有被放大——看到「放大、右侧出界」
是 PowerShell 默认 DPI-unaware 抓屏只拿到屏幕左上 1/4 造成的假象。
该补丁已删除，改用 DPI-aware 的 `design/shot.ps1` 做验收。

### M11 — 会话列表：最近优先 + 分类 + 置顶（本期）

**1. 最新会话在最前**

会话列表按 `updated_at`（真实的秒级时间戳）倒序。

- 之前顺序是固定的插入顺序，聊过的会话不会浮到前面。
- 用时间戳而不是「4 小时前」这类字符串，因为字符串排不了序；
  `updated_label` 仍由 `relative_label()` 从时间戳算出来。
- **只在真正发消息时才刷新时间**（`Session::touch()`）。
  单纯点开看一眼不算活跃——否则列表会随着浏览乱跳。
- 排序返回的是**下标**而不是重排数组：`active_session` 是索引引用，重排会让它指向别的会话。

**2. 分类标记**

7 种分类：功能 / 修复 / 重构 / 文档 / 调试 / 实验 / 未分类，各带专属颜色。

- 卡片标题前显示彩色分类标签，**点一下循环切换**。
- 列表顶部一排 chip 按分类筛选（只显示实际有会话的分类，避免一排空按钮）。
- 命令面板里也能直接指定分类（比在卡片上连点更快）。

**3. 置顶**

- 卡片第二行右侧有图钉：已置顶是实心黄色，未置顶是淡色空心。
- 置顶会话**永远排在最前**（即使它最旧），置顶区与其余之间有一条「最近」分隔线。
- 同一组置顶内部仍按时间倒序。

**顺带修掉的两个体验问题**

- 分类标签原本放在标题前、置顶按钮也挤在第一行，标题被压成省略号；
  把置顶按钮移到第二行后标题恢复了正常宽度。
- 选中态原来只有底色差异，在多条卡片里不明显；改成底色 + 蓝色描边。

测试：238 个（新增 session_order 15：置顶优先、置顶组内按时间、置顶计数、
序列化往返、旧配置默认不置顶、时间戳相同时保持原序）。

### M10 — Git 写操作 + 多仓库工作区（本期）

**1. Git 写操作**（`app/src/git_ops.rs`）

| 操作 | 说明 |
|---|---|
| 暂存 / 取消暂存 | 单文件 `＋`/`−` 按钮，或「暂存全部」 |
| 提交 | 提交框输入信息后点「提交」（暂存区为空时禁用并提示） |
| 丢弃改动 | 单文件 `✕`，**先弹确认条**才执行 |
| 新建分支 | 输入框 + 「＋分支」，`check-ref-format` 挡非法名 |
| 切换分支 | 点分支名展开已有分支列表，选中即切 |

安全设计：

- **只有「丢弃」是破坏性的**，用 `GitOp::is_destructive()` 单独标注，UI 必须二次确认。
  已跟踪文件走 `git restore --`（还原到 HEAD），未跟踪文件直接删这一个文件（不做目录级 clean）。
- **暂存区为空时拒绝提交**，并给出「先暂存」的明确提示，而不是把 git 的长篇提示甩给用户。
- **空提交信息直接拒绝**，避免产生无标题提交。
- 失败一律带 git 的 stderr 原文（如分支不存在、名不合法），不返回笼统的「操作失败」。
- 空仓库（无 HEAD）里取消暂存会走 `git rm --cached`——`git restore --staged` 此时会报 `could not resolve 'HEAD'`。

**2. 多仓库工作区**

工作区是「容器目录」（如 `E:\go`，其下每个项目各自是仓库）时：

- Git 面板顶部列出工作区内所有仓库（`ShareNasWorkTool` / `goDemo` / `nas-os` / `ops-go`），
  点一个即切换该面板操作对象；文件树仍显示整个工作区。
- **多个仓库时默认不选中任何一个**，必须明确选一个才允许写操作。
  这是刻意的：自动选第一个会导致 `git add -A` 作用到用户没点选的仓库——开发中真实踩到过。
- 只有一个仓库时自动选中，无需多一步点击。

测试：223 个（新增 git_ops 15，全部用真实临时仓库跑端到端：
暂存→提交、空暂存区拒绝提交、丢弃已跟踪/未跟踪文件、建分支/切分支/重名拒绝、
中文提交信息 roundtrip、空仓库取消暂存）。

### M9 — Git 面板 + 默认工作区 + 可见尺寸修复（本期）

**1. 右侧「改动」换成 Git 面板**

`文件 / Git / 预览` 三个 Tab，Git 页显示：

- 头部：当前分支、领先/落后上游的提交数（`⎇ main ↑1`）、刷新按钮
- 改动摘要：`N 个文件（暂存 X · 未暂存 Y）`，有冲突时红标提示
- 改动文件列表：`A/M/D/R/!` 标记 + 颜色，已暂存的文件带绿点；
  点一下在下方看该文件相对 HEAD 的 diff（`+` 绿 / `-` 红 / `@@` 蓝）
- 最近提交：hash、摘要、相对时间

全部走 `git` 命令行（`status --porcelain=v1 -z`、`log`、`diff HEAD`），
不引 libgit2 —— 只读展示不需要编 C 依赖。所有命令带 `-C <repo>`，不依赖进程 cwd。

**2. 工作区默认 `E:\go`，且支持子仓库**

- 优先级：`NW_ROOT` → 上次用过的（`settings.last_workspace`）→ 内置默认 `E:\go`
  → 从当前目录向上找 `Cargo.toml` → 当前目录。每一步都校验目录存在，避免打开空目录。
- 切换工作区后会记住，下次启动直接回到那里。
- `E:\go` 这类「容器目录」本身不是仓库、但子目录各自是仓库时，
  Git 页会列出这些子仓库（如 `ShareNasWorkTool` / `goDemo` / `nas-os` / `ops-go`），
  点一下即切换过去。

**3. 修掉多轮困扰的「右栏看不见」（可见尺寸修复）**

现象：远程桌面 / 特殊 DPI 环境下，窗口按 1:1 物理像素创建，但 `bounds()` 返回物理像素、
渲染又按 scale 放大 —— 内容被放大一倍，右侧与底部整块跑到窗口外，右栏永远看不到。

判据：**窗口不可能比屏幕还宽**。当 `bounds().width > display().width` 说明两者单位不一致，
此时 `bounds()` 是物理值，需除以 scale 才等于逻辑尺寸。修复后用该值显式约束根容器宽高
（替代 `size_full()`）。

正常的 200% 缩放机器上 `bounds` 已是逻辑值（小于屏幕），不会触发这条分支，行为不变。

**4. 移除终端与全局搜索**

试做后移除：终端不做 PTY 时无法支持交互式程序，价值有限；全局搜索与编辑器内查找重叠。
对应的 `terminal.rs` / `search.rs` 与视图、快捷键、图标栏入口一并删除，保持代码干净。

测试：208 个（新增 git 10 / brackets 13、移除终端 6 与搜索 12）。

### M8 — 多光标 / 代码折叠 / 跨文件补丁 / 真主管分派 / Gemini 原生（本期）

**1. 多光标**

- `Alt/Option + 点击` 增加一个光标；`Ctrl+D` 把光标处的词连同下一个相同词一起选中（再按继续扩）。
- 主光标用实色、额外光标用半透明细线，同屏能一眼分出有几个。
- 多处同时输入、同时退格：`insert_multi` / `backspace_multi` 从后往前处理，
  **整批只记一次撤销**——撤销时一次全撤，符合直觉。
- 实现上多光标是 `TextBuffer` 里独立的一层（`extra_cursors`），
  单光标路径完全没动，因此既有的光标/选区/撤销行为零回归。

**2. 代码折叠**

- 行号左侧出现 `▾/▸`，点一下折叠/展开；`Ctrl+Shift+[` 全折、`Ctrl+Shift+]` 全展。
- 折叠判定用「花括号配平优先、缩进兜底」：`{}`/`()`/`[]` 收尾的行按括号配对找结尾，
  其余按缩进找。Python / YAML 这类无括号语言同样可用。
- 折叠后**不重排文本**，只是渲染时跳过隐藏行，所以光标、选区、保存全部不受影响。
- 编辑后自动清理失效的折叠点（`reconcile`），不会折到错位置。

**3. 跨文件批量补丁 `multi_patch`**

- 一次调用改多个文件，**整批作为一个事务**：先全量校验（路径/唯一性/体积），
  任何一项不通过就整批不动；写入中途失败会把已写的回滚。
- 于是用户只看到一次合并 diff、点一次允许。跨文件重构（改签名、重命名、批量替换）
  从"十几次审批"变成"一次审批"。
- 单批上限 20 个文件；同一文件重复出现、`content` 与 `old_string` 混用都会直接报错。

**4. Supervisor 真正分派**

- 之前 `主管指派` 等同并行，现在是真分派：Lead 每轮看当前进展，输出下一个该谁做。
- 决策是一次极短调用（只要角色名或 `DONE`），解析容忍多说几句
  （取最早出现的角色名；说了「完成」且没点名才收尾）。
- 解析失败按「还没干过活的角色」兜底，保证能推进；同一角色连续两次以上、
  或达到 6 步上限就收尾，避免空转烧 token。

**5. Gemini 原生协议**

- 新 `GeminiProvider`：`/models/{model}:streamGenerateContent?alt=sse`，
  `contents[{role,parts[{text}]}]` + 独立 `systemInstruction`，认证走 query `key`。
- `role` 映射 `user`/`model`；空文本的 assistant 轮次（纯工具调用）自动跳过。
- 选 Google Gemini 厂商不再走 OpenAI 兼容路径。
- 工具调用暂不接：Gemini 的 function calling 结构与 OpenAI 不同，
  混着做容易让模型行为不可预期，留给单独一轮。

测试：185 个（新增 folding 11 / multi_patch 13 / supervisor 9 / gemini 13 / 多光标 12）。

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
| `run_command` | 在工作区执行非交互式命令（`cwd`/超时可控），返回退出码与输出 | **是** |

**安全模型**

- 所有路径被限制在工作区内：`..` 逃逸、跨盘绝对路径、符号链接逃逸一律拒绝并报错。
- 读操作直接执行；**写操作必须先展示 diff、经用户点「允许并写入」才落盘**。
  拒绝后文件保持原样，且把「用户拒绝了」回填给模型，让它换方案而不是重试。
- **命令执行也是写权限**：`run_command` 必须经审批后才 spawn，批准前不产生任何副作用；
  `cwd` 仍限制在工作区内，默认 120s 超时（最多 600s），输出保留头 3KB + 尾 12KB；
  Windows 走 `cmd /S /C` + `CREATE_NO_WINDOW`（不弹控制台），远程工作区暂不支持。
- **权限档位**（M20）：只读/计划档不向模型提供写工具；完整档免审批但路径沙箱不变。
- 单轮最多 8 次工具循环，避免模型陷入死循环烧 token。

**交互**

- 每次工具调用渲染成一张卡片：工具名、参数摘要、状态（执行中/待确认/完成/已拒绝/失败）、
  `+n −m` 行数；写操作内嵌 diff（`+` 绿、`-` 红、上下文灰，超过 220px 可滚）。
- 待确认卡片橙色描边 + 「拒绝 / 允许并写入」按钮（`run_command` 为「拒绝 / 允许并执行」）；
  输入框上方同时有一条黄色横幅
  写明「等待你确认：write_file xxx」。
- 输入框有「⚒ 工具已开/已关」开关：关掉就退回纯对话。
- 输入框底栏另有权限 chip（只读/计划/标准/完整，见 M20），切换只影响后续消息。

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

- **52px 图标栏**：品牌标 + 对话 / 团队 / 技能三个入口（当前项高亮，内嵌 SVG 图标按蒙版跟随主题与悬停变色），
  底部是设置入口。全局入口不再散落在正文里。
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

- **设置页**（图标栏底部 ⚙）：五个分区，全部即时生效并自动落盘到 `~/.neverwork/settings.json`。
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
- **技能市场**（图标栏技能图标）：数据全部来自真实源，不再有内置的虚构目录。
  - 顶部「MCP / Skill」两个 Tab；MCP 页搜索框按名称、简介、作者本地过滤，
    Skill 页直连 skills.sh 搜索（≥2 字符，400ms 防抖，清空回落热榜）。
  - 三个来源切换：**MCP 官方 Registry**（registry.modelcontextprotocol.io）、
    **腾讯云 MCP 广场**（国内直达，30 条热门条目，安装时再抓详情页取真实启动命令）、
    **skills.sh**（官网首页热榜 + 搜索 API，条目带累计安装量；安装时解析上游真实
    `SKILL.md`——GitHub 源走 jsDelivr 数据接口列文件后取 CDN/raw，站点源走
    `/.well-known/agent-skills/<id>/SKILL.md`，失败回退常见路径与 main/master 双分支）。
  - 结果按来源缓存到 `~/.neverwork/marketplace.json`：断网继续显示上次快照，
    状态行标出「数据来自 … · 上次同步 …」；刷新失败只记错误，不清空旧数据。
    启动时缓存 10 分钟内不自动重拉（刷新按钮强制），避免每次打开都等首页；
    GitHub 源安装先直试 `skills/<id>/SKILL.md`（实测 1.5s → 0.4s）。
  - 分类预览卡（办公效率 / 知识管理 / 开发编程 / 内容创作 / 设计多媒体）与 11 个筛选，
    三种排序（推荐 / 名称 / 热度）；推荐里可安装的排在「仅远程」前面。
  - **详情抽屉显示上游真实介绍**：搜索 API 不带 description，打开详情时才懒加载上游
    `SKILL.md` 正文（已安装的读磁盘），加载中/失败均有提示；安装时直接复用已拉到的正文，
    不再二次请求。正文按 Markdown 渲染（标题/列表/行内代码），`[文字](url)` 链接显示为
    可点击的 accent 文本，点击用系统浏览器打开。抽屉遮罩阻断滚轮穿透：滚正文只滚抽屉，
    背后列表不动。
  - 安装：文本技能把正文落盘到 `skills/<id>/SKILL.md`（skills.sh 条目先从上游解析真实
    正文再写入）；MCP 插件先取启动配置，
    经用户确认后写入 `mcp_servers`，工具以 `mcp__<server>__<tool>` 出现且逐次审批。
  - 已安装文本技能在**单助手对话与团队角色**都会注入系统提示（每个技能截断 1500 字），
    装完就能在对话里生效。
  - **国内镜像**（技能页右上角 / 设置 → 常规）：MCP 子进程注入
    `npm_config_registry=https://registry.npmmirror.com` 与清华 PyPI 索引，
    用户已设置的环境变量不覆盖；关掉开关则用系统默认源。
  - Agent 模板可一键套用为当前团队（流水线三人组 / 并行探索团 / 单人对练）。
- **多协议**：`neverwork-agent` 的 `ProviderConfig` 增加 `temperature`，OpenAI 兼容与
  Anthropic 都带上；对话优先用设置里的当前厂商，未配置时提示去设置页，仍兼容 `NW_API_KEY` 等环境变量。
- **主题运行时切换**：`theme::set_theme_mode()` 写全局原子量，所有 `theme::xxx()`
  立刻返回新色板的颜色，视图代码零改动。

新增/扩展：`core/settings.rs`、`app/settings_ops.rs`、`app/skills_catalog.rs`、
`app/marketplace.rs`（三源客户端 + 缓存）、`views/settings.rs`、`views/skills.rs`；
`Session` 之外新增 `Settings` / `ProviderEntry` / `ProviderPreset` / `Protocol` /
`Skill` / `SkillCategory` / `ThemeMode`。

### M15 — 远程 SSH 工作区（本期）

工作区不再限于本机目录：文件树、编辑器与 Agent 工具都直接作用于远端开发机。

- **连接串**：`ssh://[user@]host[:port]/abs/path`，输入框也收 `user@host:/path`。
  认证优先用密钥 / ssh-agent；只在密码登录的机器（如内置 root 账号）可在连接行里填密码。
  密码**只保存在内存**（不写 settings、不进命令行、不进日志），通过 `SSH_ASKPASS`
  回调本程序交给系统 ssh；关掉应用即失效。不自动信任主机指纹（首次连接请先在终端 `ssh` 一次）。
- **传输层**（`core/src/ssh.rs`）：`RemoteFs` 提供 probe / tree / list / read / write /
  exists / search_fixed；路径统一为相对工作区根的 POSIX 路径；命令统一
  `cd <root> && …`，单引号转义，写入走 `.nw-tmp` + `mv -f`（与本地一样是原子替换）。
- **工具**：`ToolContext` 后端化，`read_file / list_dir / search_files / write_file /
  apply_patch / multi_patch` 在远程下语义不变，写操作仍逐次审批；
  `search_code`（语义检索）远程不可用，提示词会引导模型改用 `search_files`。
- **界面**：工作区管理器新增「⇅ 连接远程 SSH…」内联输入（连接串 + 可选密码，回车即连）；
  密码框失焦时只显示圆点，连接成功后从输入框清掉、转存内存；
  最近列表用 ⇅ 标记远程项；文件树异步拉取（只请求已展开目录，迟到的结果按代次丢弃）；
  点文件异步读取、保存异步写回；左栏工作区细条与右栏文件树标题显示 `host:path`；
  远程下 Git 面板显示「暂不支持」占位。
- **会话**：远程工作区以连接串为键单独存一份会话库；切回本地时旧状态
  （树 / 打开的远程文件 / 连接目标 / 在途请求）整体清理。

**限制**（明确不做）：远程 Git 面板、远端文件监听（不自动刷新，点 ⟳ 手动刷）、
终端 / 端口转发、远程目录选择器。密码不做持久化：每次启动重连需要重新输入
（已有密钥的机器不受影响）。

**测试**：`core/src/ssh.rs` 单测覆盖连接串解析 / 路径规范化 / `sh_quote` / 树解析 /
认证 Debug 不泄密；真实远端往返测试默认 `#[ignore]`，密钥认证设
`NW_SSH_TEST=user@host:/tmp/nw` 即可跑；密码认证再加 `NW_SSH_TEST_PASSWORD` 与
`NW_ASKPASS_EXE=<target/debug/neverwork.exe>`（askpass 由应用本体提供）。

### M16 — 思考等级 + 输入框改版（本期）

输入框按参考稿重排，并给模型一个真正生效的「思考深度」旋钮。

- **输入框**：大圆角卡片，左下圆形「＋」（引用文件 / 工具开关 / 思考档位 / 清空），
  底栏不再常驻思考 chip；发送仍走 Enter（Shift+Enter 换行）。
- **思考等级**（会话级，随会话保存）：`自动 / 低 / 中 / 高`，默认自动。
  - OpenAI 兼容：`reasoning_effort=low|medium|high`；Anthropic：`thinking.budget_tokens`
    （1024/4096/16384，同时 `temperature=1`、`max_tokens>budget`）；Gemini：
    `generationConfig.thinkingConfig.thinkingBudget`（思考 token 计入输出，上限同比抬高）。
  - **自动 = 不发任何思考参数**，与旧行为完全一致；旧会话没有该字段时按自动处理。
  - 网关不认参数（400）时先去掉 `reasoning_effort` 重试，再去掉 `stream_options` 重试。
  - 面板：4 格滑杆**可拖动**（按下即吸到最近一档，窗口级监听保证滑出轨道也跟手），
    也可点刻度/标签；拖动期间只改内存，松开鼠标才落盘。
- **输入联想已移除**：输入框下方不再出候选，聊天框 Tab 不做任何事，设置里的 `ai_suggest` 也已删除。
- **思考入口默认隐藏**：右下不再常驻「思考 · 档位」chip；档位从「＋」菜单进入，
  面板内可「重置为自动 / 收起」（Esc 也可关），详见 M18。
- **验证**：`NW_COMPOSER=thinking|menu` 启动即展开对应面板。

### M17 — 工具步骤合成一行统计（本期）

连续的工具调用不再逐条列出路径与输出，只留一行：

```
⚒ 2 线程 · 6 轮思考 · 24 个工具 · 列目录 10 · 读取 8 · 搜索 6
```

- **线程**：发起调用的角色数（单 Agent 恒 1）；**轮次**：去重后的「角色 × 第几轮模型调用”。
- **动作**：按工具语义归类计数（读取 / 列目录 / 搜索 / 写入 / 打补丁 / 批量修改 /
  联网搜索 / 抓取网页 / 交接）。
- 行尾只给异常计数（`执行中 / 待确认 N / 失败 N / 中断 N`），不点名到具体步骤。
- 明细完全隐藏（`expanded_steps` 与 `NW_EXPAND_STEPS` 已删）；
  **写操作审批是唯一例外**：仍旧弹 diff 与「拒绝 / 允许并写入」。
- `MessageKind::ToolCall` 新增 `thread` / `round` 字段；旧会话没有这两个字段时
  只显示工具数与动作（不编造线程/轮次），`AgentEvent::ToolStarted` 带上 `round` 供界面记录。

### M18 — 对话联网：搜索 + 抓网页；思考入口默认隐藏

**联网**（`agent/src/web.rs`，两个只读内置工具，默认常开、不弹审批）：

- `web_search(query, count?)`：走必应 RSS（`https://www.bing.com/search?q=…&format=rss`），
  返回标题 / 链接 / 摘要（默认 8 条，最多 10）。用 RSS 而不是爬搜索结果页：结构稳定、
  不需要 Key、中文结果由必应本地化。
- `web_fetch(url, max_chars?)`：只收 http/https，12s 超时、300KB 体积上限，
  HTML 去掉 script/style/head 后转纯文本（默认 8000 字，上限 20000）；
  拒绝 localhost 与内网地址（防止被网页内容诱导去探测本机/内网）。
- 复用 `models.rs` 的代理感知 ureq（`Proxy::try_from_env`）；开启 ureq `charset`
  特性按 Content-Type 解码，中文站点不乱码。
- 返回正文照常被 `wrap_untrusted` 包成不可信数据；提示词新增「需要最新/外部信息
  （版本号、发布说明、新闻、官方文档）就用 web_search，不要凭记忆猜」。
- **验证**：自然问句「Rust 最近发布了什么新版本？」→ 模型自己连发 2 次 `web_search` →
  4 次 `web_fetch`（含 `blog.rust-lang.org` 正文 13.5KB，全部 `ok=true`）→ 给出带链接的回答；
  `agent/tests/web_live.rs`（`#[ignore]`，真网冒烟）覆盖搜索 + 抓取 + 中文乱码断言。

**思考入口默认隐藏**：输入框右下不再常驻「思考 · 档位」chip；档位入口收进左下「＋」菜单
（`⚙ 思考 · 当前档位`）。点开后输入框上方出现滑杆面板，面板头部有「重置为自动 / 收起」，
Esc 也可关。档位本身与各厂商映射不变（见 M16）。

### M19 — 命令执行工具

`run_command(command, cwd?, timeout_secs?)`：在 Agent 工具集里补上命令执行。

- **审批门控**：`requires_approval = true`，卡片文案「会执行命令，确认后运行：」+
  「允许并执行」；拒绝时只回填「用户拒绝了这个命令」，不 spawn 任何进程。
- **执行**：Windows `cmd /S /C`（`raw_arg`，命令自带引号原样保留）+ `CREATE_NO_WINDOW`
  （不弹控制台）+ `chcp 65001`（cmd 内建输出 UTF-8，仍有 GBK 兜底解码）；Unix `/bin/sh -lc`。
- **边界**：`cwd` 走 `ToolContext::resolve`（`..`/跨盘/符号链接照旧被挡）；默认 120s、
  最多 600s 超时，到点 kill；输出按头 3KB + 尾 12KB 截断（stdout/stderr 分开标注）；
  非零退出返回 `COMMAND_FAILED`（带退出码与输出），超时 `TIMEOUT`，远程工作区 `UNSUPPORTED`。
- **提示词**：系统提示新增 run_command 条目（构建/测试/git/装依赖用，读文件别用命令），
  强调写操作与命令都先弹审批。
- **验证**：真机走完整链路（模型发起 → 审批 → 执行 → 输出回填），会话卡预览显示命令摘要；
  单测覆盖 echo、非零退出、cwd、路径越界、超时、引号保留、输出头尾截断。

### M20 — 会话权限模式（本期）

对话框底栏新增权限 chip，按会话保存（旧会话缺字段默认「标准」），只影响审批与写工具的可见性：

| 档位 | 模型可见工具 | 写操作 / 命令 |
|---|---|---|
| 只读 | 读、列目录、搜索、联网 | 写工具（含全部 MCP）不发给模型 |
| 计划 | 同上 | 同上；系统提示要求先产出实施计划，切档后执行 |
| 标准（默认） | 全部 | 逐次弹审批（原有行为） |
| 完整 | 全部 | 直接执行、不弹审批；切入需二次确认 |

- **执行层**：`tools::specs_for(permission)` 先过滤工具清单；agent 循环里若仍收到写调用，
  直接回 `PERMISSION_DENIED`（不弹审批）。`full` 只免审批——`ToolContext::resolve`、
  `.git` 禁写、命令超时/截断全部保留。
- **系统提示**：只读/计划/完整各追加一段；计划档明确「不要声称已修改」。
- **UI**：底栏常驻 `◉ 只读 / ◇ 计划 / ◈ 标准 / ⚡ 完整` chip，点击在输入框上方展开
  四档面板（名称 + 说明 + 选中标记）；切「完整」出现黄色确认条。与「＋」菜单、
  思考面板互斥，Esc 可关；工具关闭时 chip 置灰。
- **验证**：`NW_PERMISSION=read_only|plan|standard|full`、`NW_COMPOSER=permission`；
  单测覆盖工具过滤、系统提示与旧会话默认值。


## 运行

```bash
# 主界面
cargo run -p neverwork
# 指定工作区根
set NW_ROOT=E:\path\to\project && cargo run -p neverwork
# 直接连远程工作区（密钥/agent 优先；密码登录的机器在界面里补密码，首次请先在终端 ssh 确认指纹）
set NW_REMOTE=me@10.0.0.2:/srv/app && cargo run -p neverwork
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
