use neverwork_core::config;

/// `NW_CONFIG_DIR` 是进程级环境变量，设它的测试必须串行，否则互相踩目录。
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn config_roundtrip() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 用临时目录隔离，避免污染真实用户目录。
    let tmp = std::env::temp_dir().join(format!("neverwork-test-{}", std::process::id()));
    std::env::set_var("NW_CONFIG_DIR", &tmp);
    let _ = std::fs::remove_dir_all(&tmp);

    let mut cfg = config::Config::default();
    assert_eq!(cfg.teams.len(), 1);
    cfg.teams[0].roles.push(neverwork_core::Role::new("extra", "Extra", "E"));

    config::save(&cfg).expect("save");
    let loaded = config::load();
    assert_eq!(loaded.teams.len(), 1);
    assert!(loaded.teams[0].roles.iter().any(|r| r.name == "Extra"));
    assert_eq!(loaded.active_team_id, cfg.active_team_id);

    let _ = std::fs::remove_dir_all(&tmp);
}

use neverwork_core::{ExportOptions, Settings};

#[test]
fn export_defaults_to_redacted() {
    let mut s = Settings::default();
    s.providers[0].api_key = "sk-must-not-leak".into();
    let cfg = config::Config::default();

    let text = config::export_all(&cfg, &s, ExportOptions::default()).unwrap();
    assert!(
        !text.contains("sk-must-not-leak"),
        "默认导出必须脱敏，但文件里出现了密钥"
    );

    let (_, back, includes_secrets) = config::import_all(&text).unwrap();
    assert!(!includes_secrets);
    assert!(back.providers.iter().all(|p| p.api_key.is_empty()));
}

#[test]
fn export_with_secrets_opt_in() {
    let mut s = Settings::default();
    s.providers[0].api_key = "sk-explicit".into();
    let cfg = config::Config::default();

    let text = config::export_all(&cfg, &s, ExportOptions::with_secrets()).unwrap();
    assert!(text.contains("sk-explicit"), "显式勾选后应包含密钥");

    let (_, back, includes_secrets) = config::import_all(&text).unwrap();
    assert!(includes_secrets);
    assert_eq!(back.providers[0].api_key, "sk-explicit");
}

#[test]
fn load_settings_upgrades_legacy_builtin_models() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = std::env::temp_dir().join(format!("neverwork-test-migrate-{}", std::process::id()));
    std::env::set_var("NW_CONFIG_DIR", &tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // 旧版配置：智谱还是 glm-4 一代（整代已下线），模型列表为纯字符串数组。
    let json = r#"{"providers":[{"id":"glm","preset":"glm","name":"智谱 GLM","base_url":"https://open.bigmodel.cn/api/paas/v4","api_key":"sk-glm","models":["glm-4-plus","glm-4","glm-4-flash","glm-4v"],"model":"glm-4","enabled":true}]}"#;
    std::fs::write(tmp.join("settings.json"), json).unwrap();

    let loaded = config::load_settings();
    let glm = loaded.providers.iter().find(|p| p.id == "glm").unwrap();
    assert_eq!(glm.model, "glm-5.3", "旧默认模型不在新列表时取第一个");
    assert!(glm.models.iter().any(|m| m.id == "glm-5.3-flash"));
    assert!(!glm.models.iter().any(|m| m.id.starts_with("glm-4")));

    // 迁移必须落盘，否则用户本次不改设置就白迁移。
    let text = std::fs::read_to_string(tmp.join("settings.json")).unwrap();
    assert!(text.contains("glm-5.3"));
    assert!(!text.contains("glm-4-plus"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn import_keeps_local_keys_when_export_was_redacted() {
    let mut local = Settings::default();
    local.providers[0].api_key = "sk-local-keep".into();

    let exported = config::export_all(
        &config::Config::default(),
        &local.redacted(),
        ExportOptions::default(),
    )
    .unwrap();
    let (_, mut imported, includes_secrets) = config::import_all(&exported).unwrap();
    assert!(!includes_secrets);
    assert!(imported.providers[0].api_key.is_empty());

    config::merge_keeping_local_keys(&mut imported, &local);
    assert_eq!(
        imported.providers[0].api_key, "sk-local-keep",
        "导入脱敏配置时不应清掉本地已填的 Key"
    );
}

#[test]
fn import_of_full_export_overrides_local() {
    let mut local = Settings::default();
    local.providers[0].api_key = "sk-old".into();
    let mut other = Settings::default();
    other.providers[0].api_key = "sk-new".into();

    let exported =
        config::export_all(&config::Config::default(), &other, ExportOptions::with_secrets()).unwrap();
    let (_, mut imported, includes_secrets) = config::import_all(&exported).unwrap();
    assert!(includes_secrets);
    config::merge_keeping_local_keys(&mut imported, &local);
    assert_eq!(imported.providers[0].api_key, "sk-new", "带密钥的导出应覆盖本地");
}
