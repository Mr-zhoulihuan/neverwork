# 浏览器自动化测试报告（HTML）设计

> 日期：2026-11-18
> 状态：已与用户确认方向（方案 A），待评审后进入实施计划

## 背景与目标

NeverWork 已有内置浏览器自动化（`browser_open/snapshot/click/type/press/wait/screenshot/console/close`，免审批）。用户需要每次自动化测试后产出一份**可直接阅读/归档的 HTML 测试报告**：

- 报告是**单文件 HTML**：用例汇总 + 每条用例的步骤/预期/实际 + 截图证据 + console 错误，截图以 base64 内嵌，拷走即可打开。
- 测试过程中的截图**不再作为独立产物**出现在「产物」条/右栏产物面板，全部汇入报告；HTML 报告本身是**唯一产物**。
- 产物区点开 `.html` 用系统默认浏览器打开（当前会进文本编辑器显示源码）。
- 自动化：浏览器测试收尾时由 Agent 自动调用报告工具；漏调用时机制提醒一次。

## 方案对比（结论：A）

| 方案 | 做法 | 结论 |
|---|---|---|
| **A. 新工具 `browser_report` + Rust 侧模板** | 模型只传结构化用例数据，Rust 读会话截图清单并内嵌 base64 生成 HTML | **采用**：样式统一、token 省、截图可自包含 |
| B. Agent 用 `write_file` 直接写 HTML | 零新工具 | 否决：模型读不了二进制无法内嵌截图，HTML 质量不可控 |
| C. Markdown 报告再转换 | 复用现有渲染 | 否决：用户要 HTML 单文件，多一层转换无收益 |

## 设计

### 1. 截图改为报告素材（不进产物）

`crates/agent/src/browser/cdp.rs`：

- 新增
  ```rust
  /// 本次浏览器会话内的一张截图（报告素材）。
  pub struct Shot {
      pub path: PathBuf,
      pub title: String,
      pub at: u64,
  }
  ```
- `BrowserSession` 增加字段：
  - `pub shots: Vec<Shot>`（编号 = 下标 + 1，会话销毁即清空）
  - `pub started: Instant`（`launch` 时赋值，用于报告里的会话耗时）
  - `pub browser_name: String`（`find_browser()` 取到的 exe 文件名，报告元信息用）

`crates/agent/src/browser/mod.rs::screenshot`：

- 仍落盘 `artifacts_dir(ctx)/<name>.png`（工作区 `artifacts/`，远程/无工作区为配置目录）。
- 成功后 `b.shots.push(Shot { .. })`，返回文案改为：
  ```
  截图已归档：#3（登录 - FcStor）
  （截图只进 browser_report 生成的测试报告；不要在正文里引用截图）
  ```
  不再返回 `截图已保存：<abs>` 首行，也不返回 Markdown 图片行——从源头断掉「正文图片扫描」与「首行解析登记」两条产物通道。

`crates/app/src/state.rs`：

- 删除 `ToolFinished` 里 `result.name == "browser_screenshot"` 的登记分支（原「截图已保存：」解析，约 4700-4718 行）。
- 新增 `result.name == "browser_report"` 的登记分支：取结果首行前缀 `测试报告已生成：` 后的绝对路径，走现有 `register_artifact(session_idx, path, thread)`。前缀解析抽成纯函数 `report_path_from(content: &str) -> Option<&str>` 并加单测。

### 2. 新工具 `browser_report`

- 注册：`tools.rs::tool_specs()` 增加 spec；`BROWSER_TOOLS` 数组加入 `"browser_report"`（自动获得免审批、只读/计划档剔除、`browser_tools_present_and_free_of_approval` 测试覆盖）；`ToolCall::summary()` 增加一行展示。
- 执行入口：`browser::execute` 增加 `"browser_report" => report(call, ctx)`。
- 前置条件：要求浏览器会话存在（复用 `with_existing`，拿 `shots`/`console_lines()`/`title_url()`/`started`）；`browser_close` 后再生成会失败——提示词要求先生成报告再关闭。

参数 schema：

```json
{
  "type": "object",
  "properties": {
    "title":    { "type": "string", "description": "报告标题，如「FcStor 登录流程验收」" },
    "summary":  { "type": "string", "description": "可选，一句话总体结论" },
    "include_console": { "type": "boolean", "description": "是否附 console 错误段，默认 true" },
    "cases": {
      "type": "array",
      "description": "测试用例列表（至少 1 条）",
      "items": {
        "type": "object",
        "properties": {
          "name":    { "type": "string", "description": "用例名" },
          "status":  { "type": "string", "enum": ["pass", "fail", "skip"] },
          "steps":   { "type": "string", "description": "操作步骤，多行文本，可选" },
          "expected":{ "type": "string", "description": "预期结果" },
          "actual":  { "type": "string", "description": "实际结果" },
          "chapter": { "type": "string", "description": "所属章节名（如「登录流程」）；同一章节的用例会聚合，缺省归入「其他」" },
          "screenshots": {
            "type": "array",
            "items": { "type": "integer" },
            "description": "browser_screenshot 返回的截图编号"
          }
        },
        "required": ["name", "status", "expected", "actual"]
      }
    }
  },
  "required": ["title", "cases"]
}
```

校验规则（全部返回 `BAD_ARGS`，消息带可用编号方便模型一次纠正）：

- `title` 非空；`cases` 非空数组；`status` 只接受 `pass|fail|skip`（大小写不敏感，落库统一小写）。
- `screenshots` 元素接受数字或 `"[3]"`/`"3"` 字符串；编号必须在 `1..=shots.len()` 内，越界时报错并列出可用编号。
- 截图文件缺失：该图位置渲染「图片缺失」占位，不整体失败。
- 单张截图 > 8MB 不内嵌，占位「图片过大未内嵌」。
- 写文件失败返回 `BROWSER_ERROR`。

输出文件：

- 目录同截图：`artifacts_dir(ctx)`。
- 文件名 `测试报告-<本地时间戳>.html`（如 `测试报告-20261118-153045.html`）；同秒重名追加 `-2`、`-3`。
- 返回：
  ```
  测试报告已生成：<abs 路径>
  用例 5 个：通过 4 / 失败 1 / 跳过 0
  （已内嵌 3 张截图与 console 错误；产物区点击用默认浏览器打开）
  ```

### 3. HTML 报告（`crates/agent/src/browser/report.rs`，新增）

纯函数模块，便于单测；`mod.rs` 只做参数解析与文件 IO。

```rust
pub enum CaseStatus { Pass, Fail, Skip }
pub struct ReportCase { name, status, steps, expected, actual, screenshots: Vec<usize> }
pub struct ShotInfo { path: PathBuf, title: String }   // 由 cdp::Shot 转换
pub struct ConsoleItem { level: String, text: String }
pub struct ReportInput {
    title, summary, url, browser, generated_at, elapsed_secs,
    cases: Vec<ReportCase>, shots: Vec<ShotInfo>, console: Vec<ConsoleItem>,
}
pub fn render_html(input: &ReportInput) -> String;
pub fn escape_html(s: &str) -> String;   // & < > " ' 全转义
```

页面结构（内联 CSS，浅色、打印友好、左侧固定目录 + 右侧内容区）：

0. **章节式组织**：用例可带 `chapter` 字段，同一 chapter 的用例聚成一章（按首次出现顺序编号 `1/2/3…`，用例编号 `1.1/1.2/2.1…`）；空 chapter 归「其他」章。全部用例都无 chapter 时退化为平铺列表（不渲染章头，编号 `1/2/3…`），保持旧观感。
1. **左侧目录（TOC）**：固定宽 264px、独立滚动；列出「汇总」、各章（可折叠）与章内用例链接（状态色点）；滚动时用内联 JS（IntersectionObserver）高亮当前项；窄屏（<980px）与打印时隐藏。允许少量内联 JS（目录折叠/高亮），单文件、离线可用。
2. 头部：`<h1>` 标题、summary、元信息行（生成时间、目标 URL、浏览器名、工作区路径）。
3. 汇总卡片（锚点 `#summary`）：用例总数 / 通过 / 失败 / 跳过 / 耗时（`<div class="stats">` 五格，数字大号）。
4. 章节区：每章 `<section class="chapter" id="ch-N"><h2>N 章名</h2>`；每条用例 `<article class="case {status}" id="case-N-M">`：
   - 标题行 `N.M 用例名` + 状态徽章（通过绿 / 失败红 / 跳过灰）。
   - 「步骤」`<pre>`（保留换行，`white-space: pre-wrap`，有才显示）。
   - 「预期 / 实际」两列对照；失败用例的实际结果加浅红底。
   - 截图 `<figure><img src="data:image/png;base64,…"><figcaption>截图 #3 · 页面标题</figcaption></figure>`，图片宽度 100%。
5. console 段（锚点 `#console`）：`include_console` 且存在 error/warn/assert/http/net 条目时列出（最多 100 条，级别 + 文本表格）；为空显示「无 console 错误」。
6. 底部：`由 NeverWork 自动生成 · <时间>`。

截图内嵌数据由 `mod.rs` 在写文件前读入并 base64 编码（`base64::engine::general_purpose::STANDARD`）。

### 4. 本地时间戳（core 公用）

`crates/core/src/config.rs` 新增：

```rust
/// `20261118-153045`，报告文件名用。
pub fn local_timestamp() -> String;
/// `2026-11-18 15:30:45`，报告正文用。
pub fn local_datetime_now() -> String;
```

- Windows：手写 FFI `GetLocalTime`（`#[repr(C)] SYSTEMTIME`，`extern "system"`，`#[link(name = "kernel32")]`），不引依赖。
- 其它平台：退回 UTC 拆分（`civil_from_days` 小函数内联），仅测试环境使用。
- 单测：格式长度、全数字/分隔符、月份/日范围合理。

### 5. 产物打开方式

`crates/app/src/state.rs::open_artifact`：

- 在图片分支之后增加：扩展名 `html|htm` → `cx.open_url(&format!("file:///{}", path 斜杠化))`（`Context` 可 deref 到 `App`，`App::open_url` 走平台默认浏览器），直接 return。
- 其余类型行为不变（文本进编辑器）；远程工作区维持现有「暂不能直接打开」。

### 6. 提示词与自动化触发

`crates/agent/src/agent_loop.rs`：

- 系统提示工具清单加一行：
  `- browser_report(title, cases[], summary?, include_console?)：生成 HTML 测试报告（用例汇总+内嵌截图+console 错误），存 artifacts/ 并登记产物；浏览器测试收尾必须调用`
- 「浏览器验收与登录测试」小节追加两条：
  - 完成浏览器测试（登录、关键流程、负向用例）后**必须**调用 `browser_report` 出报告，每条用例填名称/步骤/预期/实际/状态，截图编号引用 `browser_screenshot` 返回值。
  - 截图只进报告，**不要在正文里贴图片**；生成报告后再 `browser_close`。
- 轻量提醒（只拦一次，不设 MAX 计数）：
  - 常量 `BROWSER_REPORT_REMINDER`：说明「本轮浏览器操作已多步但没生成报告，请整理用例调用 browser_report 后收尾」。
  - 统计：成功执行的浏览器工具（`is_browser_tool` 且非 `browser_close`）计数 ≥ 3 且 `browser_report` 从未成功过时，在 `calls.is_empty()` 的收尾分支（browser_gate 之后）注入一次提醒并 `continue`；`report_reminded: bool` 防重复。
  - 判定抽纯函数 `report_should_remind(browser_steps: usize, report_written: bool, reminded: bool) -> bool` 并加单测；用户明确拒绝/环境不支持时模型可在回答中说明，机制最多烦一次。

## 错误处理

- 报告工具任何参数问题返回 `BAD_ARGS`（不可重试标志同现有约定），文件系统问题返回 `BROWSER_ERROR`（可重试）。
- 浏览器会话已关闭/崩溃：`NO_BROWSER` 提示先 `browser_open` 重新测试。
- 截图清单在 `browser_open` 重启会话（headed 切换/崩溃重启）时清空，报告只能引用当前会话编号——提示词已要求先生成报告再关闭。

## 测试计划

单测：

- `report.rs`：`escape_html`（五类字符）、汇总统计、截图编号映射与 figcaption、缺失截图占位、console 段有/无、标题与用例渲染。
- `tools.rs`：`browser_report` 在 `BROWSER_TOOLS` 且免审批（现有遍历测试自动覆盖）。
- `agent_loop.rs`：`report_should_remind` 边界（步数 2/3、已写报告、已提醒）。
- `core/config.rs`：`local_timestamp` 格式。
- `state.rs`：`report_path_from` 前缀解析。

回归：`cargo test --workspace`（含 `cargo build -p neverwork`）。

实机验收：

1. `NW_SEND` 下发「用浏览器测试本地页面并生成测试报告」类指令（可用内置 `python -m http.server` 或 192.168.1.218 登录，等用户凭据）。
2. 检查：对话产物条只出现 **HTML 报告**（无 png）；右栏产物面板同样。
3. 点报告产物 → 系统默认浏览器打开，截图以 base64 显示、console 段存在。
4. 漏调报告场景：构造只点两次浏览器的任务，确认收尾被提醒一次后正常结束。

## 不做的事（YAGNI）

- 不做应用内报告渲染页（HTML 交给系统浏览器）。
- 不做报告历史管理/对比（每次一个新文件，产物列表天然按时间倒序）。
- 不删除截图原文件（留作排障，只是不登记为产物）。
- 不给报告工具加审批（与截图同属自动测试链路，免审批）。
- 不引 chrono 等时间依赖。
