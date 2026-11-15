//! 示例数据：驱动界面展示（对齐设计稿内容）。

use neverwork_core::{
    CodeBlock, Message, MessageKind, ParallelItem, Participant, Session,
};

/// 当前工作区（远程示例）。


fn participant(name: &str, color_index: usize, initial: &str) -> Participant {
    Participant {
        role_id: name.to_lowercase(),
        role_name: name.into(),
        color_index,
        initial: initial.into(),
    }
}

/// 主会话内容。
pub fn session() -> Session {
    Session {
        id: "s1".into(),
        title: "重构支付回调".into(),
        subtitle: "3 个角色 · 流水线 · 步骤 2/3".into(),
        team_name: "默认协作团队".into(),
        updated_label: "2分钟前".into(),
        preview: "已抽出 handler 并加幂等校验".into(),
        status: neverwork_core::SessionStatus::Done,
        meta: "2m 12s".into(),
        accent: 2,
        icon: "✦".into(),
        badge: "⚡".into(),
        action: neverwork_core::SessionAction::Resume,
        tag: neverwork_core::SessionTag::Feature,
        updated_at: now_secs().saturating_sub(120),
        pinned: true,
        messages: vec![
            Message {
                kind: MessageKind::User,
                body: "重构支付回调，拆成独立的 handler，并补上幂等处理。".into(),
                code: None,
                elapsed_ms: 0,
            },
            Message {
                kind: MessageKind::Handoff {
                    from: "Planner".into(),
                    to: "Planner".into(),
                    action: "已接手 → 拆解任务".into(),
                },
                body: String::new(),
                code: None,
                elapsed_ms: 0,
            },
            Message {
                kind: MessageKind::Agent {
                    participant: participant("Planner", 0, "P"),
                },
                body: "## What I can do\n\nI'm a programming assistant working inside your IDE, `E:\\AI\\ide` as the workspace with `\\go` root.\n\nOn the \"can you modify code\" part — yes, but with a caveat you'll notice immediately:\n\nI create, overwrite, and patch files — including multi-file refactors committed as one atomic transaction (all changes apply together). Every `write_f`, `apply_pa`, `multi_pa` pops an approval prompt before it lands on disk after you approve. Reads are free and instant; writes always need your explicit go-ahead. If you reject one, I'll propose a different approach rather than retrying the same write.\n\n- `read` / `list` / `search` — free and instant\n- `write_f` / `apply_pa` — needs approval\n\n```rust\nlet add = |a, b| a + b;\n```\n".into(),
                code: None,
                elapsed_ms: 0,
            },
            Message {
                kind: MessageKind::Handoff {
                    from: "Coder".into(),
                    to: "Coder".into(),
                    action: "开始执行 ③ 个子任务".into(),
                },
                body: String::new(),
                code: None,
                elapsed_ms: 0,
            },
            Message {
                kind: MessageKind::Agent {
                    participant: participant("Coder", 1, "C"),
                },
                body: "已抽出 handler 并加幂等校验。".into(),
                code: Some(CodeBlock {
                    filename: "callback.rs".into(),
                    added: 24,
                    removed: 8,
                    code: "impl PaymentCallbackHandler {\n    pub async fn handle(&self, evt: Event) -> AppResult<()> {\n        if self.seen.contains(&evt.id) { return Ok(()) }\n        self.apply(evt).await?\n    }\n}".into(),
                }),
                elapsed_ms: 0,
            },
            Message {
                kind: MessageKind::Tool {
                    summary: "已写入 2 个文件 · 幂等键校验已加".into(),
                    files: vec!["callback.rs".into(), "events.rs".into()],
                },
                body: String::new(),
                code: None,
                elapsed_ms: 0,
            },
            Message {
                kind: MessageKind::Parallel {
                    items: vec![
                        ParallelItem {
                            role_name: "Reviewer".into(),
                            color_index: 2,
                            initial: "R".into(),
                            task: "审查逻辑".into(),
                            done: true,
                        },
                        ParallelItem {
                            role_name: "Coder".into(),
                            color_index: 1,
                            initial: "C".into(),
                            task: "补写单测".into(),
                            done: false,
                        },
                    ],
                },
                body: String::new(),
                code: None,
                elapsed_ms: 0,
            },
            Message {
                kind: MessageKind::Agent {
                    participant: participant("Reviewer", 2, "R"),
                },
                body: "## What I can do\n\nI'm a programming assistant working inside your IDE, `E:\\AI\\ide` as the workspace with `\\go` root.\n\nOn the \"can you modify code\" part — yes, but with a caveat you'll notice immediately:\n\nI create, overwrite, and patch files — including multi-file refactors committed as one atomic transaction (all changes apply together). Every `write_f`, `apply_pa`, `multi_pa` pops an approval prompt before it lands on disk after you approve. Reads are free and instant; writes always need your explicit go-ahead. If you reject one, I'll propose a different approach rather than retrying the same write.\n\n- `read` / `list` / `search` — free and instant\n- `write_f` / `apply_pa` — needs approval\n\n```rust\nlet add = |a, b| a + b;\n```\n".into(),
                code: None,
                elapsed_ms: 0,
            },
        ],
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 演示用的会话列表：第一条带完整对话，其余为空会话，
/// 用不同状态覆盖「工作中 / 已完成 / 已废弃」以及右上角四种圆圈样式。
///
/// `updated_at` 是真实的秒级时间戳——列表要按它倒序排，
/// 单靠「4小时前」这种字符串排不了序。
pub fn demo_sessions() -> Vec<Session> {
    use neverwork_core::{SessionAction, SessionStatus, SessionTag};

    let mut out = vec![session()];
    let now = now_secs();

    // (标题, 摘要, 状态, 胶囊文字, 色索引, 图标, 徽标, 右上角, 分类, 多少分钟前)
    let rows: [(&str, &str, SessionStatus, &str, usize, &str, &str, SessionAction, SessionTag, u64); 9] = [
        ("每天 07:30 搜索 30 条今日时间线", "想跟你确认一下「发送到哪里」", SessionStatus::Dropped, "", 0, "◔", "", SessionAction::Replay, SessionTag::Debug, 240),
        ("了解 agent 开发需要掌握的技能", "已完成「专家进阶」（第 1–3 章）", SessionStatus::Done, "已完成", 3, "◈", "", SessionAction::Replay, SessionTag::Docs, 780),
        ("阅读原型中的对象存储模块", "The doc/ dir exists. Let me check…", SessionStatus::Idle, "", 4, "◎", "", SessionAction::None, SessionTag::Refactor, 900),
        ("共享存储二期改造完成度复核", "用户停止测试，改为让我编译代码…", SessionStatus::Done, "已完成", 1, "▤", "", SessionAction::Done, SessionTag::Bug, 1080),
        ("共享存储二期改造完成度复核", "用户要求提交并推送代码。三个仓…", SessionStatus::Working, "4m 42s", 2, "◐", "", SessionAction::Resume, SessionTag::Feature, 1140),
        ("本地复制成功之后 进行克隆恢…", "[附件]", SessionStatus::Done, "已完成", 1, "▤", "⑂", SessionAction::Done, SessionTag::Debug, 1200),
        ("测试共享存储的复制任务", "32 容灾节点的三个程序…", SessionStatus::Done, "已完成", 1, "▤", "⑂", SessionAction::Failed, SessionTag::Experiment, 1260),
        ("共享存储二期改造测试方法", "修复上面两个问题：root…", SessionStatus::Done, "已完成", 1, "▤", "", SessionAction::None, SessionTag::Refactor, 1320),
        ("开发进度与待办功能", "I'll inspect the current s…", SessionStatus::Done, "已完成", 5, "✦", "", SessionAction::None, SessionTag::Feature, 1380),
    ];

    for (i, (title, preview, status, meta, accent, icon, badge, action, tag, mins_ago)) in
        rows.into_iter().enumerate()
    {
        let at = now.saturating_sub(mins_ago * 60);
        out.push(Session {
            id: format!("s{}", i + 2),
            title: title.into(),
            subtitle: String::new(),
            team_name: "默认协作团队".into(),
            messages: Vec::new(),
            updated_label: neverwork_core::relative_label(at),
            preview: preview.into(),
            status,
            meta: meta.into(),
            accent,
            icon: icon.into(),
            badge: badge.into(),
            action,
            tag,
            updated_at: at,
            // 演示数据里把第一条置顶，直观展示置顶效果
            pinned: i == 0,
        });
    }
    out
}
