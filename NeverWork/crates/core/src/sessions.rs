//! 会话持久化：每个工作区一个文件，`~/.neverwork/sessions/<slug>-<hash>.json`。
//!
//! 为什么按工作区隔离：会话里的路径、diff、工具输出都只对某个项目有意义。
//! 全部塞进一个文件的话，切到别的项目时列表里会冒出一堆跟它无关的记录。
//!
//! 写入方式与 `config.json` / `settings.json` 一致：临时文件 + rename 原子替换，
//! 避免写到一半崩溃留下半个 JSON 把历史全废掉。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::config_dir;
use crate::model::Session;
use crate::{AppError, AppResult};

/// 磁盘上的会话库。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionStore {
    /// 上次激活的会话 id。重启后恢复到它，而不是永远跳回第一条。
    #[serde(default)]
    pub active_id: String,
    #[serde(default)]
    pub sessions: Vec<Session>,
}

fn sessions_dir() -> PathBuf {
    config_dir().join("sessions")
}

/// FNV-1a：给文件名加一段稳定短哈希。
/// 只靠 slug 会撞——`a/b` 与 `a-b` 的 slug 完全相同，却是两个工作区。
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// 工作区路径 → 文件名：`E:\AI\ide` → `E-AI-ide-xxxxxxxx.json`。
fn file_name_for(root: &Path) -> String {
    let raw = root.to_string_lossy();
    let mut slug = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    // 文件名有 255 字符上限：深路径截断，尾部的哈希负责区分。
    let head: String = slug.chars().take(48).collect();
    format!("{head}-{:08x}.json", fnv1a(&raw))
}

/// 某个工作区的会话文件路径。
pub fn store_path(root: &Path) -> PathBuf {
    sessions_dir().join(file_name_for(root))
}

/// 加载会话库；文件不存在或损坏时返回空库。
///
/// 损坏时宁可当空库重新开始，也不要把错误弹给用户——历史丢了已经够糟，
/// 再拦一道启动失败没意义。
///
/// 加载后会做一次崩溃恢复（见 `normalize_after_crash`）。
pub fn load(root: &Path) -> SessionStore {
    load_with_recovery(root).0
}

/// 加载结果的损伤报告，供启动时提示。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadReport {
    /// 崩溃恢复（Running/AwaitingApproval → Interrupted）的会话数。
    pub recovered: usize,
    /// 解析失败被跳过的会话数（坏记录不拖累好的）。
    pub dropped: usize,
}

/// 与 `load` 相同，但额外返回损伤报告，供启动时提示「可续跑 / 有记录损坏」。
pub fn load_with_recovery(root: &Path) -> (SessionStore, LoadReport) {
    let path = store_path(root);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut report = LoadReport::default();
    let mut store = match serde_json::from_str::<SessionStore>(&text) {
        Ok(s) => s,
        // 一条坏记录（比如新版本才有的枚举值、手改坏了的字段）不该
        // 让整个库归零：退到逐条解析，能救回来的都留着。
        Err(_) if !text.trim().is_empty() => {
            let (salvaged, dropped) = salvage(&text);
            report.dropped = dropped;
            // 先留一份原始文件：能解析失败也可能是「这个版本读不懂」
            // （回滚到旧版本、字段将来被扩展），而启动后的第一次保存
            // 就会把读不懂的记录永久抹掉。
            let _ = std::fs::write(path.with_extension("json.bak"), &text);
            salvaged
        }
        Err(_) => SessionStore::default(),
    };
    report.recovered = normalize_after_crash(&mut store);
    (store, report)
}

/// 整体解析失败时的兜底：只保留能单独解析的会话，其余计数丢弃。
///
/// 会话文件是崩溃恢复的载体，载体本身不能「一条坏就全废」——真实踩过：
/// 一个非法枚举值让整库反序列化失败，界面直接变成一个全新的空会话，
/// 用户看到的是历史凭空消失。
fn salvage(text: &str) -> (SessionStore, usize) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return (SessionStore::default(), 0);
    };
    let active_id = v
        .get("active_id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let arr = v.get("sessions").and_then(|x| x.as_array());
    let Some(arr) = arr else {
        return (SessionStore { active_id, sessions: Vec::new() }, 0);
    };
    let total = arr.len();
    let sessions: Vec<Session> = arr
        .iter()
        .filter_map(|item| serde_json::from_value::<Session>(item.clone()).ok())
        .collect();
    let dropped = total - sessions.len();
    (SessionStore { active_id, sessions }, dropped)
}

/// 崩溃恢复：把磁盘上的「进行中」降级成 `Interrupted`。
///
/// 为什么必须降级：正常收尾会写 `Done`/`Failed`/`Cancelled`，所以文件里
/// 还留着 `Running`/`AwaitingApproval` 就说明进程在任务途中被杀。
/// 不降级的话界面会永远转圈，用户既等不到结果，也不知道可以重来。
///
/// 返回被降级的会话数，供调用方提示「有 N 个任务被中断」。
pub fn normalize_after_crash(store: &mut SessionStore) -> usize {
    let mut n = 0;
    for s in &mut store.sessions {
        if s.stage.is_active() {
            s.stage = crate::model::TaskStage::Interrupted;
            // 胶囊也要跟着回落，否则卡片仍显示「工作中」
            if s.status == crate::model::SessionStatus::Working {
                s.status = crate::model::SessionStatus::Idle;
            }
            s.sync_action();
            n += 1;
        }
    }
    n
}

/// 原子保存会话库。
pub fn save(root: &Path, store: &SessionStore) -> AppResult<()> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let path = store_path(root);
    let tmp = path.with_extension("json.tmp");
    let text =
        serde_json::to_string(store).map_err(|e| AppError::Config(e.to_string()))?;
    std::fs::write(&tmp, text)?;
    // Windows 的 rename 不允许覆盖已存在的目标，先删旧的。
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_path_maps_to_same_file() {
        let a = file_name_for(Path::new("E:\\AI\\ide"));
        let b = file_name_for(Path::new("E:\\AI\\ide"));
        assert_eq!(a, b);
        assert!(a.ends_with(".json"));
        assert!(a.starts_with("E-AI-ide-"), "文件名要能看出是哪个项目: {a}");
    }

    #[test]
    fn different_paths_do_not_collide() {
        let a = file_name_for(Path::new("E:\\AI\\a-b"));
        let b = file_name_for(Path::new("E:\\AI\\a\\b"));
        assert_ne!(a, b, "分隔符不同的两个工作区不能映射到同一个文件");
    }

    #[test]
    fn long_path_is_truncated_but_still_unique() {
        let deep = format!("E:\\{}", "verylongsegment\\".repeat(20));
        let name = file_name_for(Path::new(&deep));
        assert!(name.len() <= 64, "文件名要短于上限: {}", name.len());
    }

    fn session(stage: crate::model::TaskStage) -> Session {
        Session {
            id: "s".into(),
            title: "t".into(),
            subtitle: String::new(),
            team_name: String::new(),
            mode: crate::model::SessionMode::Solo,
            messages: Vec::new(),
            updated_label: String::new(),
            preview: String::new(),
            status: crate::model::SessionStatus::Working,
            meta: String::new(),
            accent: 0,
            icon: String::new(),
            badge: String::new(),
            action: crate::model::SessionAction::None,
            tag: crate::model::SessionTag::None,
            updated_at: 0,
            pinned: false,
            stage,
            resume_prompt: String::new(),
        }
    }

    #[test]
    fn crash_leaves_active_stages_interrupted() {
        use crate::model::{SessionStatus, TaskStage};
        let mut store = SessionStore {
            active_id: String::new(),
            sessions: vec![
                session(TaskStage::Running),
                session(TaskStage::AwaitingApproval),
                session(TaskStage::Done),
                session(TaskStage::Idle),
                session(TaskStage::Cancelled),
            ],
        };
        let n = normalize_after_crash(&mut store);
        assert_eq!(n, 2, "只有 Running / AwaitingApproval 是崩溃证据");
        assert_eq!(store.sessions[0].stage, TaskStage::Interrupted);
        assert_eq!(store.sessions[1].stage, TaskStage::Interrupted);
        assert_eq!(store.sessions[2].stage, TaskStage::Done, "已完成不能被改写");
        assert_eq!(store.sessions[3].stage, TaskStage::Idle);
        assert_eq!(store.sessions[4].stage, TaskStage::Cancelled);
        // 胶囊回落，否则左侧卡片仍显示「工作中」
        assert_eq!(store.sessions[0].status, SessionStatus::Idle);
        assert_eq!(store.sessions[0].action, crate::model::SessionAction::Resume);
    }

    #[test]
    fn legacy_store_without_stage_loads_as_idle() {
        // 旧版本写出的 JSON 没有 stage 字段，不能因为缺字段整库当损坏丢掉。
        let old = r#"{"active_id":"a","sessions":[{
            "id":"a","title":"旧会话","subtitle":"","team_name":"",
            "messages":[],"updated_label":"","preview":""}]}"#;
        let mut store: SessionStore = serde_json::from_str(old).expect("旧格式要能读");
        assert_eq!(store.sessions[0].stage, crate::model::TaskStage::Idle);
        assert_eq!(normalize_after_crash(&mut store), 0);
    }

    #[test]
    fn resume_prompt_survives_round_trip() {
        let mut s = session(crate::model::TaskStage::Interrupted);
        s.resume_prompt = "把 logger 换成 tracing".into();
        let text = serde_json::to_string(&s).unwrap();
        let back: Session = serde_json::from_str(&text).unwrap();
        assert_eq!(back.stage, crate::model::TaskStage::Interrupted);
        assert_eq!(back.resume_prompt, "把 logger 换成 tracing");
    }

    #[test]
    fn a_bad_record_does_not_wipe_the_whole_store() {
        // 真实踩过：一条会话里有个非法枚举值（`tag:"dev"`），
        // 整体反序列化失败 → 整库归零 → 用户看到历史凭空消失。
        let mut good_session = session(crate::model::TaskStage::Done);
        good_session.id = "good".into();
        let good = serde_json::to_string(&good_session).unwrap();
        let bad = r#"{"id":"bad","title":"坏会话","subtitle":"","team_name":"",
            "messages":[],"updated_label":"","preview":"","tag":"not-a-tag"}"#;
        let text = format!(
            r#"{{"active_id":"good","sessions":[{good},{bad}]}}"#
        );
        let (store, dropped) = salvage(&text);
        assert_eq!(store.sessions.len(), 1, "能解析的会话要留着");
        assert_eq!(store.sessions[0].id, "good");
        assert_eq!(store.active_id, "good");
        assert_eq!(dropped, 1, "丢了几条要能报告给用户");
    }

    #[test]
    fn total_garbage_is_reported_as_dropped_zero() {
        // 文件彻底不是 JSON：没有可救的东西，也不该谎报「丢了几条」。
        let (store, dropped) = salvage("not json at all");
        assert!(store.sessions.is_empty());
        assert_eq!(dropped, 0);
    }

    /// 真实文件走一遍：坏记录不拖垮好记录，并且留一份原始备份。
    #[test]
    fn corrupt_file_is_salvaged_and_backed_up() {
        let dir = std::env::temp_dir().join(format!("nw-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("NW_CONFIG_DIR", &dir);

        let root = Path::new("E:\\no-such-workspace");
        let path = store_path(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut good = session(crate::model::TaskStage::Done);
        good.id = "good".into();
        let good = serde_json::to_string(&good).unwrap();
        let bad = r#"{"id":"bad","title":"坏","subtitle":"","team_name":"",
            "messages":[],"updated_label":"","preview":"","tag":"no-such-tag"}"#;
        std::fs::write(
            &path,
            format!(r#"{{"active_id":"good","sessions":[{good},{bad}]}}"#),
        )
        .unwrap();

        let (store, report) = load_with_recovery(root);
        assert_eq!(report.dropped, 1, "坏记录要计数");
        assert_eq!(report.recovered, 0);
        assert_eq!(store.sessions.len(), 1, "好记录要留住");
        assert_eq!(store.sessions[0].id, "good");
        assert!(
            path.with_extension("json.bak").exists(),
            "原始文件要留备份，否则下一次保存就永久抹掉读不懂的记录"
        );

        std::env::remove_var("NW_CONFIG_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
