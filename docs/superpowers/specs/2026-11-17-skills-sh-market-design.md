# skills.sh 技能来源 + 单助手技能注入

日期：2026-11-17
状态：已实现（2026-11-17 验收：单测 + 真网冒烟 + 截图）

## 背景

- 现有 Skill 页只有两个固定 GitHub 仓库（`anthropics/skills`、`vercel-labs/agent-skills`），收录面窄；magpie 的技能市场用的是 skills.sh（可搜索、带安装量，覆盖数千个仓库和站点）。
- 已安装文本技能只在团队角色路径注入提示词（`enrich_team_skills`），单助手对话直接用一个空 prompt 的 `Role`，不读已安装技能——装完对话没有任何变化，看起来像"假安装"。

## 目标

1. Skill 页来源改为 skills.sh（**替换** GitHub 技能源，不并存）。
2. 搜索框接 skills.sh 搜索 API（≥2 字符，400ms 防抖）；空搜索显示官网首页热榜。
3. 安装时按需解析真实 `SKILL.md` 并落盘，复用 `skills_ops::install_text`。
4. 单助手对话也注入已安装文本技能（每个截断 1500 字，沿用 `role_skills_section` 的既有截断）。

## 非目标

- 不接 skills.sh 下载 API（每小时 60 次的限制）。
- 不解析 `SKILL.md` 同目录的 scripts 等附属文件（本项目技能模型是单文件）。
- 不改 MCP 来源（官方 Registry / 腾讯云广场）。

## 上游接口（2026-11-17 已实测）

- 搜索：`GET https://www.skills.sh/api/search?q=<urlencode>&limit=200`
  - 返回 `{"skills":[{"id":"owner/repo/skillId","source":"owner/repo","skillId":"...","name":"...","installs":48742,"isOfficial":true}]}`
  - `q` 少于 2 字符 → `{"error":"Query must be at least 2 characters"}`
  - 也收录非 GitHub 站点源（如 `"source":"open.feishu.cn"`）。
- 首页：`GET https://www.skills.sh/` 为服务端渲染 HTML，热榜行结构：
  `href="/<source>/<skillId>"` + `<h3 ...>name</h3>` + `<p ...>source</p>` + 安装量 `<span class="font-mono text-sm text-foreground">4.4M</span>`。
  magpie 旧解析依赖的 `initialSkills":` JSON 已经不存在。
- 正文解析：
  - GitHub 源：`https://data.jsdelivr.com/v1/packages/gh/<owner>/<repo>@<branch>?structure=flat` 列仓库文件，找 `…/<skillId>/SKILL.md`（大小写不敏感），下载走 `https://cdn.jsdelivr.net/gh/<owner>/<repo>@<branch><path>`，失败回退 `https://raw.githubusercontent.com/<owner>/<repo>/<branch><path>`。
  - 站点源：`https://<source>/.well-known/agent-skills/<skillId>/SKILL.md`（`open.feishu.cn` 实测 200）。
  - 分支依次尝试 `main`、`master`；列表接口失败时回退直接试常见路径：`skills/<id>/SKILL.md`、`<id>/SKILL.md`、`.claude/skills/<id>/SKILL.md`、`.agents/skills/<id>/SKILL.md`。

## 改动

### marketplace.rs

- `MarketSource` 用 `SkillsSh` 替换 `GitHub`：key `skillssh`、label `skills.sh`、origin `www.skills.sh`、hue `0x10b981`；`all()` 仍为 3 个来源。
- `MarketEntry` 增可选字段 `remote: Option<RemoteSkillRef>`；`RemoteSkillRef { source: String, id: String }`，serde 默认省略，兼容旧缓存。
- `MarketEntry` 增 `detail_loaded: bool`（serde 默认 `false`）：搜索 API **不返回 description**，条目的 `detail` 初始只是占位文案（「正在从上游读取技能说明…」），打开详情抽屉时才懒加载上游 `SKILL.md` 正文回填，回填后置 `true`。缓存 roundtrip 兼容旧文件（缺字段按 `false`）。
- 删除 GitHub 来源实现：`GITHUB_REPOS`、`fetch_github`、`fetch_github_repo`、`list_github_skill_dirs`、`download_github_skill`、`parse_github_skill` 及相关常量。
- 新增：
  - `fetch_skills_sh_home()` / `parse_skills_sh_home(html)`：解析首页热榜行；安装量支持 `K/M/B` 后缀（`4.4M` → 4400000），解析不到记 0。
  - `search_skills_sh(q)` / `parse_skills_sh_search(&Value)`：按 `(source, skillId)` 去重，非法条目跳过。
  - `fetch_remote_skill_body(source, id)`：按上面"正文解析"顺序返回 Markdown 文本。
  - 辅助：`is_github_source`（source 恰好一段 `/`）、`pick_skill_path`、`skill_page_url`、`urlencode`。
- 条目生成：id `ss.<slug(source)>.<slug(skillId)>`；作者 = source；标签只在 `isOfficial` 时加「官方」——不把 source 当标签，否则筛选条会被上百个仓库名挤爆；`source_url = https://www.skills.sh/<source>/<skillId>`；图标 = GitHub owner 头像（仅 GitHub 源）；`kind = Text`；`summary` 说明来自 skills.sh 与累计安装量。
- 缓存：首页热榜写入 `marketplace.json`（key `skillssh`）；搜索结果只进内存，不写缓存。

### state.rs / settings_ops.rs

- 新字段：`skill_source: MarketSource`（默认 `SkillsSh`，与 `mcp_source` 同构）、`skill_search_gen: Rc<Cell<u64>>`（防抖代际，仿照 `auto_save_gen`）。
- `set_market_source`：`SkillsSh` → `skills_tab = 1` 并记 `skill_source`；其余 → `skills_tab = 0` 并记 `mcp_source`。
- `set_skills_tab(1)` 回到 `skill_source`，不再固定 GitHub。
- `refresh_market`：`SkillsSh` 且搜索词 ≥2 字符 → `search_skills_sh`（不写缓存）；否则首页热榜（写缓存）。应用结果前校验代际与当前来源，丢弃过期响应。启动时若当前来源缓存距今 <10 分钟则不自动重拉（避免每次开技能页都等首页 1~5s），刷新按钮仍强制。
- 新方法 `on_skill_search_changed`：当前来源为 `SkillsSh` 时，输入变化经 400ms（`background_executor().timer`）防抖触发刷新；清空搜索回落热榜。触发点：搜索框按键、退格清空、清空按钮、`set_field`、IME 上屏（`input_handler::after_edit`，组合态 `marked_range` 期间不触发，避免拿拼音搜索）。
- 启动环境变量：`NW_MARKET_SOURCE=skillssh`、`NW_SKILL_QUERY=<q>` 时启动刷新即走搜索；`NW_SKILLS_TAB=1` 归一为 `SkillsSh`。
- 安装：`install_market_entry` 遇到 `entry.remote = Some(..)` 时统一走 `install_remote_entry(entry, bind_role, cx)`——后台 `fetch_remote_skill_body`（`market_installing` 置忙、失败写 `action_msg` 并附来源页链接），成功后 `skills_ops::parse_md` 取正文，保留条目名称/简介，再走 `install_skill` 落盘。GitHub 源先直试最常见布局 `skills/<id>/SKILL.md`（main 分支，6s 短超时，CDN→raw），命中即返回（实测 1.5s → 0.4s）；未命中再列目录定位与常见路径回退。
- 详情正文懒加载：新字段 `market_detail_loading: bool` / `market_detail_error: Option<String>`。`load_market_detail_body(cx)`：已安装技能直接 `skills_ops::read_body` 读磁盘；未安装后台 `fetch_remote_skill_body`，成功后 `parse_md` 取 `detail` 经 `set_market_detail_body` 同步回 `market_detail` 与 `market_entries`（`detail_loaded = true`），失败记 error；响应回来时若抽屉已切到别的条目则给当前条目补拉（同 id 不重试，避免失败循环）。`open_market_detail*` 清 error 并触发懒加载，`close_market_detail` 复位 loading/error。
- 安装复用详情正文：`install_remote_entry` 见 `entry.detail_loaded` 时直接用已回填的 `skill` 走 `install_skill` + `bind_skill_to_role`，不再二次请求上游。
- 团队绑定：角色面板「＋添加」与详情抽屉「绑定到角色」不得对远程条目直接 `install_skill`（会落盘占位正文），统一经 `attach_skill_to_role` → 未安装且 `market_entry_for(id).remote.is_some()` 时走 `install_remote_entry(entry, Some(role_idx))`，装完再 `bind_skill_to_role`。
- `NW_INSTALL_SKILL` 启动钩子优先 `market_entry_for(id)` → `install_market_entry`，找不到再回退 `install_skill`。
- 单助手：solo 路径把 `settings.skills` 中 `installed && !is_mcp()` 的 id 交给 `role_skills_section`，拼进 `solo.prompt`。

### views/skills.rs

- Skill 页来源按钮只剩 `skills.sh`；搜索占位符「搜索 skills.sh 技能（至少 2 个字符）」。
- 状态行：空搜索显示「www.skills.sh · 刚刚」等同步时间；搜索中显示「www.skills.sh · 搜索「git」」。
- 顶部 Tab 文案为「MCP / Skill」，导入 `.md` 只出现在 Skill 列。
- `filtered_entries`：`SkillsSh` 且搜索词非空时不再本地按词过滤（服务端已按相关度返回），避免误删结果。

### views/market_detail.rs / markdown.rs

- 详情正文区：`market_detail_loading && !entry.detail_loaded` 时显示「⟳ 正在拉取上游说明…」；失败且未回填时显示红色「说明拉取失败：{err}」；回填后按 Markdown 渲染上游正文。徽标「x 次阅读」改为「累计安装 x」（skills.sh 只有安装量，没有阅读数）；安装按钮忙态文案「安装中…」。
- 抽屉遮罩 `market-detail-mask` 必须 `.occlude()`：gpui 的滚轮命中取鼠标下所有 hitbox（`should_handle_scroll`），遮罩不阻断时滚正文会同时滚背后 `skills-scroll`，表现为「整页跟着往下拉」。
- `markdown.rs` 支持链接：`InlineSpan` 增 `url: Option<String>`，`InlineKind` 增 `Link`；`inline_spans` 解析 `[文字](url)`（未闭合/空地址/地址带空白按普通文本，不吞字符），`wrap_spans` 按字符携带 url 折行。`views/chat.rs::inline_row` 用 accent 色 + `cursor_pointer` 渲染，点击 `App::open_url` 打开浏览器（`on_click` 需 Stateful 元素，id 取 url+文字哈希）。详情抽屉与聊天回答共用这套渲染。

### 文档

- README M5 段与 AGENTS.md「技能市场」描述更新为 skills.sh。

## 测试

- 单测（marketplace.rs）：
  - 首页 HTML 解析：行、来源、名称、`4.4M` 安装量；跳转链接路径过滤（`/trending` 等）。
  - 搜索 JSON 解析：去重、非法条目跳过、id/作者/链接生成。
  - `pick_skill_path`：`skills/`、根目录、`.claude/skills/`、多候选取最短非隐藏路径。
  - `is_github_source`、`urlencode`（中文、空格、`&`）。
  - 缓存 roundtrip 改用 `SkillsSh`；旧缓存缺 `detail_loaded` 字段仍可加载（默认 `false`）。
- 单测（markdown.rs）：`link_spans_parse` — 合法链接拆出 `Link` span 与 url；`[坏]( )` 等非法写法按普通文本，不吞字符。
- `#[ignore]` 真网冒烟：首页 ≥20 条；搜索 `git` 非空；`fetch_remote_skill_body("github/awesome-copilot", "git-commit")` 返回含正文的 Markdown；站点源 `open.feishu.cn/lark-doc` 返回 200。
- 手动验收：
  - `NW_MARKET_SOURCE=skillssh` + `NW_SKILLS_TAB=1` 启动，截图 `pic/skills-sh-home.png` 确认热榜、来源行与已安装卡片。
  - `NW_SKILL_QUERY=git` 启动，截图 `pic/skills-sh-search.png` 确认「www.skills.sh · 搜索「git」」与 200 条服务端结果。
  - `NW_MARKET_DETAIL=1` 打开详情，截图 `pic/skills-sh-detail.png` 确认抽屉显示上游真实正文（git-commit 为 Conventional Commits 全文）、列表/行内代码样式正常、`[文字](url)` 链接显示为可点击 accent 文本而非原始 Markdown 语法。
  - 抽屉正文内滚轮下滑：正文滚动而背后卡片网格位置不变（无滚动穿透）。
  - 安装一条技能，检查 `~/.neverwork/skills/<id>/SKILL.md` 真实存在且有正文（git-commit 实测为上游 Conventional Commits 全文）。
  - 单助手发一条消息，trace 确认系统提示包含该技能正文。

## 风险与兜底

- 首页 HTML 或搜索 API 改版：解析失败时保留上次缓存，状态行显示错误，不虚构条目。
- jsDelivr 数据接口不可用：回退 raw.githubusercontent.com（逐分支）与常见路径直试。
- 站点源没有 `.well-known/agent-skills`：安装报错并提示来源页链接，不做猜测。
- 旧缓存里的 `github` 源条目不再展示，但已安装的 `gh.*` 技能仍保留在 settings 与磁盘，可继续卸载/绑定。
