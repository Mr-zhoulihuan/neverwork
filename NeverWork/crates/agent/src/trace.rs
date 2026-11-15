//! 极简 trace：一次运行一个 jsonl 文件，逐步记下「做了什么、花了多久、结果多大」。
//!
//! 排查「为什么它一直在读文件」「为什么这轮这么慢」时，先看 trace 再改提示词。
//! 写盘失败一律静默——trace 是旁路，不能拖垮主流程。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 同进程内区分同一毫秒内先后启动的多次运行（团队模式会连跑好几个角色）。
static SEQ: AtomicU64 = AtomicU64::new(0);

/// 一次运行的 trace。
pub struct Trace {
    /// `None` 表示 trace 不可用（目录建不出来），此时所有 step 都是空操作。
    path: Option<PathBuf>,
    run_id: String,
    started: std::time::Instant,
}

impl Trace {
    /// 落在 `config_dir()/traces/run-<epoch_ms>-<seq>.jsonl`。
    pub fn new() -> Self {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = neverwork_core::config::config_dir().join("traces");
        Self::new_in(&dir, format!("{epoch}-{seq}"))
    }

    /// 指定目录与运行编号（测试用）。
    pub fn new_in(dir: &Path, run_id: impl std::fmt::Display) -> Self {
        let run_id = run_id.to_string();
        let path = std::fs::create_dir_all(dir)
            .ok()
            .map(|_| dir.join(format!("run-{run_id}.jsonl")));
        Self {
            path,
            run_id,
            started: std::time::Instant::now(),
        }
    }

    /// trace 是否真的在落盘。
    pub fn enabled(&self) -> bool {
        self.path.is_some()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// 记一步。`fields` 会被补上 `t_ms / run / round / kind` 再写一行。
    pub fn step(&self, round: usize, kind: &str, mut fields: serde_json::Value) {
        let Some(path) = &self.path else { return };
        let Some(obj) = fields.as_object_mut() else {
            return;
        };
        obj.insert(
            "t_ms".into(),
            serde_json::json!(self.started.elapsed().as_millis() as u64),
        );
        obj.insert("run".into(), serde_json::json!(self.run_id));
        obj.insert("round".into(), serde_json::json!(round));
        obj.insert("kind".into(), serde_json::json!(kind));
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{fields}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nw-trace-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn writes_one_json_line_per_step() {
        let dir = tmp_dir("lines");
        let t = Trace::new_in(&dir, "t1");
        assert!(t.enabled());
        t.step(0, "model", serde_json::json!({"chars": 12}));
        t.step(1, "tool", serde_json::json!({"tool": "read_file", "ok": false}));

        let text = std::fs::read_to_string(t.path().unwrap()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let v: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v["kind"], "tool");
        assert_eq!(v["round"], 1);
        assert_eq!(v["run"], "t1");
        assert!(v["t_ms"].is_u64());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unusable_dir_is_silent() {
        // 拿一个已存在的**文件**当目录：create_dir_all 必然失败，trace 应降级为空操作
        let f = std::env::temp_dir().join(format!("nw-trace-file-{}.txt", std::process::id()));
        std::fs::write(&f, "x").unwrap();
        let t = Trace::new_in(&f, "t2");
        assert!(!t.enabled());
        t.step(0, "model", serde_json::json!({}));
        let _ = std::fs::remove_file(&f);
    }
}
