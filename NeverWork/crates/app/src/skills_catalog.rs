//! 内置技能目录：技能市场里可浏览/安装的技能清单。
//!
//! 这是**离线内置**的目录，不依赖网络。条目字段与 `core::Skill` 一致，
//! 安装状态保存在 `settings.json` 的 `skills` 里。
//! 将来接远端 registry 时，只需把 `catalog()` 换成一次请求的结果。

use neverwork_core::{Skill, SkillCategory};

/// 目录条目模板（安装状态由调用方合并）。
struct Entry {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    category: SkillCategory,
    summary: &'static str,
    author: &'static str,
    installs: u64,
    stars: u64,
}

macro_rules! entry {
    ($id:expr, $name:expr, $glyph:expr, $cat:ident, $summary:expr, $author:expr, $installs:expr, $stars:expr) => {
        Entry {
            id: $id,
            name: $name,
            glyph: $glyph,
            category: SkillCategory::$cat,
            summary: $summary,
            author: $author,
            installs: $installs,
            stars: $stars,
        }
    };
}

fn entries() -> Vec<Entry> {
    vec![
        // ===== 办公效率 =====
        entry!(
            "tencent.tencent-docs",
            "腾讯文档 TENCENT DOCS",
            "文",
            Office,
            "腾讯文档在线协作平台，支持文档、表格、幻灯片的多人在线编辑与权限管理。",
            "tencent",
            2_465_000,
            333
        ),
        entry!(
            "wps.office-suite",
            "WPS Office 全家桶",
            "W",
            Office,
            "覆盖文字、表格、演示与 PDF 的完整办公套件技能，支持批量转换与模板套用。",
            "wps",
            1_870_000,
            214
        ),
        entry!(
            "community.ppt-generator",
            "ppt-generator-skill",
            "P",
            Office,
            "输入主题与大纲，自动生成结构完整、版式统一的 PPT 初稿。",
            "community",
            962_000,
            158
        ),
        entry!(
            "tencent.doc-automation",
            "腾讯文档在线协作自动化",
            "⟳",
            Office,
            "自动创建、分发并回收协作文档，适合周报汇总、评审收集等流程。",
            "tencent",
            431_000,
            67
        ),
        entry!(
            "community.secretary-writing",
            "秘书材料星公文写作-抄笔",
            "✎",
            Office,
            "按机关公文格式生成通知、请示、纪要，自动套用红头与字体规范。",
            "community",
            288_000,
            41
        ),

        // ===== 知识管理 =====
        entry!(
            "skillhub.multi-search-engine",
            "多引擎搜索",
            "⌕",
            Knowledge,
            "提供 16 个搜索引擎（国内直连 + 国际）的跨引擎检索，自动去重与多源交叉验证。",
            "skillhub",
            11_035_000,
            180
        ),
        entry!(
            "ima.ima-skills",
            "ima-skills",
            "i",
            Knowledge,
            "腾讯 ima 知识库的检索与写入技能，支持把对话结论沉淀进个人知识库。",
            "ima",
            1_540_000,
            132
        ),
        entry!(
            "community.summarize",
            "Summarize",
            "≡",
            Knowledge,
            "对长文、会议记录、网页做分层摘要，可指定字数、语气与输出结构。",
            "community",
            2_310_000,
            245
        ),
        entry!(
            "community.web-search",
            "全网搜索",
            "🌐",
            Knowledge,
            "聚合新闻、论坛、GitHub 与学术源，返回带出处的结构化结果。",
            "community",
            1_180_000,
            96
        ),
        entry!(
            "community.web-tools-guide",
            "web-tools-guide",
            "⚒",
            Knowledge,
            "常用网页工具的调用指南：截图、抓取、表单填写与登录态复用。",
            "community",
            640_000,
            58
        ),
        entry!(
            "community.global-library",
            "全球 12 亿文献知识库",
            "◈",
            Knowledge,
            "覆盖 8000 万+ 论文与专利的语义检索，支持按引用关系追溯。",
            "community",
            402_000,
            73
        ),

        // ===== 开发编程 =====
        entry!(
            "skillhub.ake.dev-god",
            "编程大神.Skill",
            "<>",
            Dev,
            "【全栈能力】面向复杂真实项目的高阶全栈编程助手，不止生成代码，还负责从需求拆解到落地。",
            "skillhub",
            11_490_000,
            119
        ),
        entry!(
            "indiv.integrator.dev-expert",
            "编程专家Pro.Skill",
            "✦",
            Dev,
            "融合编程专家 2.0.3 工程专题与编程大神鸿蒙 ArkTS/ArkUI、微信小程序和鸿蒙原生应用开发能力。",
            "indiv-integrator",
            6_312_000,
            39
        ),
        entry!(
            "indiv-ebandao.dev-expert",
            "编程专家.Skill",
            "◇",
            Dev,
            "P8 级编程助手，25 年实战经验技能，覆盖 Java/PHP/网站/软件项目与 API 设计。",
            "indiv-ebandao",
            6_092_000,
            385
        ),
        entry!(
            "user.814bde54.dev-expert",
            "dev-expert",
            "DE",
            Dev,
            "P8 级编程助手，25 年实战经验，全栈：Java/PHP/网站/软件项目与 API 设计。",
            "user-814bde54",
            5_351_000,
            32
        ),
        entry!(
            "community.rust-review",
            "Rust 代码审查",
            "⚙",
            Dev,
            "按 clippy 规则与 Rust API 指南审查代码，指出所有权与并发问题并给出改法。",
            "community",
            186_000,
            52
        ),
        entry!(
            "community.gpui-guide",
            "GPUI 界面开发指南",
            "▦",
            Dev,
            "GPUI 布局、Entity 状态、IME 输入与自定义元素的实践要点与常见坑。",
            "community",
            24_800,
            31
        ),

        // ===== 内容创作 =====
        entry!(
            "community.libai-skill",
            "李白.Skill",
            "诗",
            Content,
            "古风与现代文案双模式创作，支持标题、正文、扩写与多轮改稿。",
            "community",
            3_260_000,
            412
        ),
        entry!(
            "community.ai-writing-tools",
            "文章去AI味工具",
            "✂",
            Content,
            "降低文本的机械感：调整句式节奏、替换套话、补充具体细节。",
            "community",
            1_720_000,
            203
        ),
        entry!(
            "community.libal-skill",
            "libal-skill",
            "L",
            Content,
            "长篇内容的结构化写作助手，先出骨架再逐节填充，保持术语一致。",
            "community",
            980_000,
            87
        ),
        entry!(
            "community.video-script",
            "视频号爆款短视频脚本",
            "▶",
            Content,
            "按平台算法偏好生成前 3 秒钩子、分镜脚本与口播稿。",
            "community",
            540_000,
            64
        ),
        entry!(
            "community.humanizer",
            "Humanizer",
            "H",
            Content,
            "把 AI 草稿改写为更自然的表达，控制口语度与专业度。",
            "community",
            610_000,
            118
        ),

        // ===== 设计多媒体 =====
        entry!(
            "community.video-media",
            "视频短视频图片音乐音频",
            "♪",
            Design,
            "视频/图片/音乐的批量处理与格式转换，支持抽帧、字幕与配乐匹配。",
            "community",
            2_140_000,
            176
        ),
        entry!(
            "community.digital-human",
            "AI 数字人口播与虚拟主播",
            "☺",
            Design,
            "生成数字人口播视频：文案转语音、口型对齐与背景替换。",
            "community",
            1_330_000,
            94
        ),
        entry!(
            "community.arch-diagram",
            "架构图一键生成",
            "▤",
            Design,
            "把系统描述转成分层架构图，可导出 SVG/PNG，节点可编辑。",
            "community",
            760_000,
            143
        ),
        entry!(
            "community.poster-studio",
            "海报设计工作室",
            "▣",
            Design,
            "按活动主题生成海报方案：版式、配色、字体搭配与文案层级。",
            "community",
            452_000,
            61
        ),
        entry!(
            "community.ui-design",
            "界面设计 · UI Design",
            "◱",
            Design,
            "从需求到界面方案：信息架构、组件选型、设计令牌与规范落地。",
            "community",
            388_000,
            102
        ),

        // ===== 数据分析 =====
        entry!(
            "community.sql-analyst",
            "SQL 数据分析师",
            "DB",
            Data,
            "自然语言转 SQL，解释执行计划，定位慢查询并给出索引建议。",
            "community",
            1_240_000,
            156
        ),
        entry!(
            "community.chart-maker",
            "图表生成器",
            "▥",
            Data,
            "把表格数据转成合适的图表，自动选择坐标轴、配色与标注。",
            "community",
            680_000,
            78
        ),
        entry!(
            "community.ab-test",
            "A/B 实验分析",
            "A/B",
            Data,
            "计算显著性、置信区间与样本量，解释结论并提示常见统计陷阱。",
            "community",
            212_000,
            44
        ),

        // ===== AI Agent =====
        entry!(
            "community.agent-architect",
            "Agent 架构师",
            "◈",
            Agent,
            "设计多 Agent 协作方案：角色划分、编排模式、上下文传递与失败回退。",
            "community",
            920_000,
            167
        ),
        entry!(
            "community.prompt-engineer",
            "提示词工程师",
            "⌘",
            Agent,
            "把模糊需求改写成可执行的系统提示词，含约束、示例与验收标准。",
            "community",
            1_450_000,
            289
        ),
        entry!(
            "community.tool-builder",
            "工具调用构建器",
            "⚒",
            Agent,
            "为 Agent 定义工具 schema、参数校验与错误处理，生成可用的调用样例。",
            "community",
            340_000,
            56
        ),

        // ===== 行业专业 =====
        entry!(
            "cic.aigc-cert",
            "AIGC过程存证服务",
            "CIC",
            Industry,
            "AIGC 创作过程自动存证：识别创作平台与生成工具，记录存证并覆盖公安备案要求。",
            "cic",
            6_055_000,
            11
        ),
        entry!(
            "cic.eb-signature",
            "全国电子印章互认平台",
            "印",
            Industry,
            "电子印章公安备案、全国互认，实现所有格式文件的电子签章与验签。",
            "cic",
            1_660_000,
            9
        ),
        entry!(
            "cic.doc-trace",
            "文档溯源取证",
            "⌗",
            Industry,
            "对文档做哈希存证与版本溯源，支持出证报告导出。",
            "cic",
            284_000,
            17
        ),

        // ===== 商业运营 =====
        entry!(
            "community.prd-analysis",
            "需求分析 PRD.Skill",
            "需",
            Business,
            "把模糊需求拆成用户故事、验收标准与边界条件，输出可直接评审的 PRD 骨架。",
            "community",
            728_000,
            96
        ),
        entry!(
            "community.req-review",
            "需求评审清单",
            "审",
            Business,
            "按完整性、一致性、可测试性检查需求文档，输出缺口清单与澄清问题。",
            "community",
            312_000,
            44
        ),
        entry!(
            "community.ecom-ops",
            "电商运营助手",
            "◫",
            Business,
            "商品标题与详情页优化、关键词布局、竞品价格带分析。",
            "community",
            1_080_000,
            134
        ),
        entry!(
            "community.growth-plan",
            "增长方案生成",
            "↗",
            Business,
            "按渠道与预算生成增长实验清单，标注预期成本与验证指标。",
            "community",
            396_000,
            48
        ),

        // ===== 生活服务 =====
        entry!(
            "indiv-ebandao.parenting",
            "育儿大师.Skill",
            "育",
            Life,
            "育儿大师 0—18 岁全场景家庭养育支持，服务对象是家长：只要孩子的问題涉及 0—18 岁，都可以问。",
            "indiv-ebandao",
            3_110_000,
            96
        ),
        entry!(
            "community.anti-fraud",
            "防骗大师.Skill",
            "盾",
            Life,
            "你身边的生活类防骗专家：当用户对疑似骗局的短信、聊天记录、邮件、链接、App 存疑时提供判断。",
            "indiv-ebandao",
            2_058_000,
            38
        ),
        entry!(
            "user.814bde54.anti-fraud",
            "anti-fraud",
            "AN",
            Life,
            "反电信诈骗（Anti-fraud）专家技能，分析疑似诈骗的短信、电话、链接与聊天记录。",
            "user-814bde54",
            1_743_000,
            2
        ),
        entry!(
            "indiv.anti-fraud-pro",
            "防诈专家",
            "⚠",
            Life,
            "覆盖常见诈骗话术库，给出处置步骤与官方举报入口。",
            "indiv",
            1_700_000,
            4
        ),

        // ===== IT 运维与安全 =====
        entry!(
            "community.k8s-ops",
            "K8s 排障助手",
            "☸",
            Ops,
            "根据事件、日志与 describe 输出定位 Pod 异常，给出 kubectl 诊断顺序。",
            "community",
            486_000,
            92
        ),
        entry!(
            "community.log-analysis",
            "日志分析",
            "≡",
            Ops,
            "海量日志的模式聚类与异常提取，输出时间线并定位首个错误。",
            "community",
            372_000,
            64
        ),
        entry!(
            "community.sec-audit",
            "安全审计",
            "⛨",
            Ops,
            "检查配置里的密钥泄漏、危险权限与已知 CVE，输出修复优先级。",
            "community",
            258_000,
            71
        ),
    ]
}

/// 完整目录（未合并安装状态）。
pub fn catalog() -> Vec<Skill> {
    entries()
        .into_iter()
        .map(|e| Skill {
            id: e.id.to_string(),
            name: e.name.to_string(),
            summary: e.summary.to_string(),
            detail: String::new(),
            category: e.category,
            author: e.author.to_string(),
            installs: e.installs,
            stars: e.stars,
            glyph: e.glyph.to_string(),
            installed: false,
        })
        .collect()
}

/// 把已安装状态合并到目录上，返回 (目录条目, 是否已安装)。
pub fn catalog_with_installed(installed_ids: &[String]) -> Vec<(Skill, bool)> {
    catalog()
        .into_iter()
        .map(|s| {
            let on = installed_ids.iter().any(|i| i == &s.id);
            (s, on)
        })
        .collect()
}

/// 某分类下安装量最高的前 n 个技能名（用于分类卡片预览）。
pub fn top_by_category(cat: SkillCategory, n: usize) -> Vec<Skill> {
    let mut list: Vec<Skill> = catalog()
        .into_iter()
        .filter(|s| s.category == cat)
        .collect();
    list.sort_by(|a, b| b.installs.cmp(&a.installs));
    list.truncate(n);
    list
}

/// 各分类的技能数量。
pub fn count_in_category(cat: SkillCategory) -> usize {
    catalog().iter().filter(|s| s.category == cat).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_not_empty_and_ids_unique() {
        let c = catalog();
        assert!(c.len() >= 40, "目录条目太少: {}", c.len());
        let mut ids: Vec<&str> = c.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "技能 id 必须唯一");
    }

    #[test]
    fn every_category_has_entries() {
        for cat in SkillCategory::all() {
            assert!(
                count_in_category(*cat) > 0,
                "分类 {} 没有技能",
                cat.label()
            );
        }
    }

    #[test]
    fn top_by_category_sorted_by_installs() {
        let top = top_by_category(SkillCategory::Dev, 3);
        assert_eq!(top.len(), 3);
        assert!(top[0].installs >= top[1].installs);
        assert!(top[1].installs >= top[2].installs);
    }

    #[test]
    fn merge_marks_installed() {
        let ids = vec!["skillhub.ake.dev-god".to_string()];
        let list = catalog_with_installed(&ids);
        let hit = list
            .iter()
            .find(|(s, _)| s.id == "skillhub.ake.dev-god")
            .unwrap();
        assert!(hit.1, "应标记为已安装");
        let other = list.iter().find(|(s, _)| s.id != "skillhub.ake.dev-god").unwrap();
        assert!(!other.1);
    }

    #[test]
    fn top_by_category_respects_limit() {
        assert_eq!(top_by_category(SkillCategory::Office, 100).len(), count_in_category(SkillCategory::Office));
    }
}
