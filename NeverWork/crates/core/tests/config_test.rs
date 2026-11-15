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
