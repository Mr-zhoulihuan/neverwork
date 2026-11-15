use neverwork_core::config;

#[test]
fn config_roundtrip() {
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
