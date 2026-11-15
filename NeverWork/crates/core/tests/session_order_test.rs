//! 会话排序与分类标记的测试。

use neverwork_core::{sessions_by_recent, relative_label, Session, SessionStatus, SessionTag};

fn s(id: &str, updated_at: u64, tag: SessionTag) -> Session {
    sp(id, updated_at, tag, false)
}

fn sp(id: &str, updated_at: u64, tag: SessionTag, pinned: bool) -> Session {
    Session {
        id: id.into(),
        title: id.into(),
        subtitle: String::new(),
        team_name: String::new(),
        messages: Vec::new(),
        updated_label: String::new(),
        preview: String::new(),
        status: SessionStatus::Idle,
        meta: String::new(),
        accent: 0,
        icon: String::new(),
        badge: String::new(),
        action: neverwork_core::SessionAction::None,
        tag,
        updated_at,
        pinned,
    }
}

#[test]
fn newest_session_comes_first() {
    let list = vec![
        s("old", 100, SessionTag::None),
        s("newest", 900, SessionTag::Feature),
        s("mid", 500, SessionTag::Bug),
    ];
    let order = sessions_by_recent(&list);
    assert_eq!(order, vec![1, 2, 0], "应按时间倒序");
    assert_eq!(list[order[0]].id, "newest");
}

#[test]
fn equal_timestamps_keep_original_order() {
    // 演示数据常常时间戳相同，不能让顺序乱跳
    let list = vec![
        s("a", 100, SessionTag::None),
        s("b", 100, SessionTag::None),
        s("c", 100, SessionTag::None),
    ];
    assert_eq!(sessions_by_recent(&list), vec![0, 1, 2]);
}

#[test]
fn empty_list_is_safe() {
    assert!(sessions_by_recent(&[]).is_empty());
}

#[test]
fn single_session() {
    let list = vec![s("only", 5, SessionTag::None)];
    assert_eq!(sessions_by_recent(&list), vec![0]);
}

#[test]
fn touch_sets_timestamp_and_label() {
    let mut x = s("x", 0, SessionTag::None);
    x.touch();
    assert!(x.updated_at > 0, "应写入当前时间");
    assert_eq!(x.updated_label, "刚刚");
}

#[test]
fn tag_cycles_through_all_then_back() {
    let mut t = SessionTag::None;
    let mut seen = vec![t];
    for _ in 0..SessionTag::all().len() - 1 {
        t = t.next();
        seen.push(t);
    }
    assert_eq!(t.next(), SessionTag::None, "转一圈应回到起点");
    // 全部标签互不相同
    let mut uniq = seen.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), seen.len(), "分类不应重复");
}

#[test]
fn tag_labels_and_colors() {
    assert_eq!(SessionTag::None.label(), "");
    assert_eq!(SessionTag::Feature.label(), "功能");
    assert_eq!(SessionTag::Bug.label(), "修复");
    // 每个有名字的分类颜色应不同，便于区分
    let mut hues: Vec<u32> = SessionTag::all()
        .iter()
        .filter(|t| **t != SessionTag::None)
        .map(|t| t.hue())
        .collect();
    hues.sort();
    hues.dedup();
    assert_eq!(hues.len(), SessionTag::all().len() - 1);
}

#[test]
fn relative_label_forms() {
    assert_eq!(relative_label(0), "刚刚");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(relative_label(now.saturating_sub(30)), "刚刚");
    assert!(relative_label(now.saturating_sub(120)).contains("分钟前"));
    assert!(relative_label(now.saturating_sub(7200)).contains("小时前"));
    assert!(relative_label(now.saturating_sub(200_000)).contains("天前"));
}

#[test]
fn old_config_without_new_fields_still_loads() {
    // 旧配置里没有 tag / updated_at，反序列化不能失败
    let json = r#"{"id":"s1","title":"t","subtitle":"","team_name":"",
        "messages":[],"updated_label":"2m","preview":"","status":"done",
        "meta":"","accent":0,"icon":"","badge":"","action":"done"}"#;
    let x: Session = serde_json::from_str(json).expect("旧配置应能加载");
    assert_eq!(x.tag, SessionTag::None);
    assert_eq!(x.updated_at, 0);
}

#[test]
fn pinned_sessions_come_first_regardless_of_time() {
    let list = vec![
        sp("newest", 9000, SessionTag::None, false),
        sp("pinned_old", 100, SessionTag::Bug, true),
        sp("middle", 5000, SessionTag::None, false),
    ];
    let order = sessions_by_recent(&list);
    assert_eq!(order[0], 1, "置顶的应排第一，即使它最旧");
    assert_eq!(list[order[0]].id, "pinned_old");
    // 其余仍按时间倒序
    assert_eq!(list[order[1]].id, "newest");
    assert_eq!(list[order[2]].id, "middle");
}

#[test]
fn multiple_pinned_sorted_by_time_among_themselves() {
    let list = vec![
        sp("p_old", 100, SessionTag::None, true),
        sp("p_new", 800, SessionTag::None, true),
        sp("normal", 9999, SessionTag::None, false),
    ];
    let order = sessions_by_recent(&list);
    assert_eq!(list[order[0]].id, "p_new", "置顶内部也按时间倒序");
    assert_eq!(list[order[1]].id, "p_old");
    assert_eq!(list[order[2]].id, "normal");
}

#[test]
fn pinned_count_works() {
    let list = vec![
        sp("a", 1, SessionTag::None, true),
        sp("b", 2, SessionTag::None, false),
        sp("c", 3, SessionTag::None, true),
    ];
    assert_eq!(neverwork_core::pinned_count(&list), 2);
}

#[test]
fn no_pinned_falls_back_to_time_order() {
    let list = vec![
        sp("a", 100, SessionTag::None, false),
        sp("b", 300, SessionTag::None, false),
    ];
    let order = sessions_by_recent(&list);
    assert_eq!(list[order[0]].id, "b");
    assert_eq!(neverwork_core::pinned_count(&list), 0);
}

#[test]
fn pinned_survives_serialization() {
    let x = sp("p", 123, SessionTag::Docs, true);
    let text = serde_json::to_string(&x).unwrap();
    let back: Session = serde_json::from_str(&text).unwrap();
    assert!(back.pinned, "置顶状态应能持久化");
    assert_eq!(back.tag, SessionTag::Docs);
}

#[test]
fn old_config_defaults_to_unpinned() {
    let json = r#"{"id":"s","title":"t","subtitle":"","team_name":"",
        "messages":[],"updated_label":""}"#;
    let x: Session = serde_json::from_str(json).unwrap();
    assert!(!x.pinned, "旧配置默认不置顶");
    assert_eq!(x.updated_at, 0);
}

#[test]
fn cancelled_tool_status_is_not_pending() {
    // 用户中断生成后，未完成的工具卡片要落到「已中断」，
    // 不能再被 is_pending 当成还在转圈。
    use neverwork_core::ToolRunStatus;
    assert!(!ToolRunStatus::Cancelled.is_pending());
    assert_eq!(ToolRunStatus::Cancelled.label(), "已中断");
    assert!(ToolRunStatus::Running.is_pending());
    assert!(ToolRunStatus::Awaiting.is_pending());
}
