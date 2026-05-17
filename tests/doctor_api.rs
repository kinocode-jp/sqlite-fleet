use sqlite_fleet::{doctor, doctor_config, Config, ExecutionConfig};
use std::fs;
use tempfile::tempdir;

#[test]
fn doctor_config_rejects_zero_parallel_execution_without_config_load() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        execution: ExecutionConfig {
            parallel: 0,
            ..ExecutionConfig::default()
        },
        ..Config::default()
    };

    let report = doctor_config(&config);
    assert!(!report.config_ok);
    assert!(!report.discovery_ok);
    assert!(!report.migrations_ok);
    assert!(report.errors[0].contains("execution.parallel は1以上が必要です"));
}

#[test]
fn doctor_library_api_does_not_write_configured_report_file() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[databases]
discovery = "glob"
path_glob = "./data/*.db"

[migrations]
dir = "./missing-migrations"
table = "_sqlite_fleet_migrations"

[execution]
parallel = 1
lock_timeout_ms = 5000
continue_on_error = false

[report]
format = "json"
path = "./doctor-report.json"
"#,
    )
    .unwrap();

    let report = doctor(dir.path().join("sqlite-fleet.toml"));
    assert!(!report.migrations_ok);
    assert!(!dir.path().join("doctor-report.json").exists());
}

#[test]
fn gui_partial_config_only_enables_explicit_flags() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[gui]
allow_config_edit = true
"#,
    )
    .unwrap();

    let config = Config::load(dir.path().join("sqlite-fleet.toml")).unwrap();
    assert!(config.gui.allow_check);
    assert!(config.gui.allow_config_edit);
    assert!(!config.gui.allow_migrate);
    assert!(!config.gui.allow_backup);
    assert!(!config.gui.allow_restore);
    assert!(!config.gui.allow_sql_apply);
    assert!(!config.gui.allow_migration_edit);
    assert!(!config.gui.allow_gui_permission_edit);
}

#[test]
fn gui_users_config_assigns_permissions_by_token() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[gui_users.viewer]
token = "viewer-token"

[gui_users.operator]
token = "operator-token"
allow_migrate = true
allow_backup = true
"#,
    )
    .unwrap();

    let config = Config::load(dir.path().join("sqlite-fleet.toml")).unwrap();
    let viewer = config
        .effective_gui_permissions(Some("viewer-token"))
        .unwrap();
    let operator = config
        .effective_gui_permissions(Some("operator-token"))
        .unwrap();
    assert!(viewer.allow_check);
    assert!(!viewer.allow_migrate);
    assert!(operator.allow_check);
    assert!(operator.allow_migrate);
    assert!(operator.allow_backup);
    assert!(config.effective_gui_permissions(Some("missing")).is_err());
}

#[test]
fn config_load_variants_validate_only_their_scope() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[databases]
discovery = "glob"
path_glob = "./data/*.db"

[migrations]
dir = " "
table = "_sqlite_fleet_migrations"

[execution]
parallel = 1
lock_timeout_ms = 5000
continue_on_error = false

[report]
format = "text"
path = "../report.json"
"#,
    )
    .unwrap();

    let discovery_config = Config::load_for_discovery(dir.path().join("sqlite-fleet.toml"));
    assert!(discovery_config.is_ok());

    let operation_error = Config::load_for_operation(dir.path().join("sqlite-fleet.toml"))
        .unwrap_err()
        .to_string();
    assert!(operation_error.contains("migrations.dir"));

    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[databases]
discovery = "glob"
path_glob = "./data/*.db"

[migrations]
dir = "./migrations"
table = "_sqlite_fleet_migrations"

[execution]
parallel = 1
lock_timeout_ms = 5000
continue_on_error = false

[report]
format = "text"
path = "./report.json"
"#,
    )
    .unwrap();

    let operation_config = Config::load_for_operation(dir.path().join("sqlite-fleet.toml"));
    assert!(operation_config.is_ok());

    let full_error = Config::load(dir.path().join("sqlite-fleet.toml"))
        .unwrap_err()
        .to_string();
    assert!(full_error.contains("report.format"));
}
