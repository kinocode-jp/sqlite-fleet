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
fn doctor_reports_config_base_dir_for_runtime_path_errors() {
    let dir = tempdir().unwrap();
    let config_dir = dir.path().join("data");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("sqlite-fleet.toml"),
        r#"
[databases]
discovery = "query"
source = "./data/shared.db"
query = "SELECT id FROM tenants"
id_column = "id"
path_template = "./data/tenants/{id}.db"

[migrations]
dir = "./backend/migrations/fleet/users"
"#,
    )
    .unwrap();

    let report = doctor(config_dir.join("sqlite-fleet.toml"));

    assert!(report.config_ok);
    assert!(!report.discovery_ok);
    assert!(!report.migrations_ok);
    assert!(report
        .errors
        .iter()
        .any(|error| error.contains("設定基準ディレクトリ")
            && error.contains(&config_dir.display().to_string())));
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
fn gui_users_config_rejects_plain_token_matching_hash() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[gui_users.plain]
token = "same-token"

[gui_users.hashed]
token_hash = "sha256:00000000000000000000000000000000:719f2a6a3ef46e0e8928e97f28bd437126bb0a5c36b0a0f5ea332e3e6eb07b57"
"#,
    )
    .unwrap();

    let error = Config::load(dir.path().join("sqlite-fleet.toml"))
        .expect_err("plain token matching token_hash must be rejected")
        .to_string();
    assert!(
        error.contains("gui_users のtokenが重複しています"),
        "{error}"
    );
}

#[test]
fn gui_users_config_accepts_uppercase_token_hash_digest() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[gui_users.viewer]
token_hash = "sha256:00000000000000000000000000000000:719F2A6A3EF46E0E8928E97F28BD437126BB0A5C36B0A0F5EA332E3E6EB07B57"
"#,
    )
    .unwrap();

    let config = Config::load(dir.path().join("sqlite-fleet.toml")).unwrap();
    let viewer = config
        .effective_gui_permissions(Some("same-token"))
        .unwrap();
    assert!(viewer.allow_check);
}

#[test]
fn gui_users_config_rejects_token_hash_case_duplicate() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("sqlite-fleet.toml"),
        r#"
[gui_users.lower]
token_hash = "sha256:00000000000000000000000000000000:719f2a6a3ef46e0e8928e97f28bd437126bb0a5c36b0a0f5ea332e3e6eb07b57"

[gui_users.upper]
token_hash = "sha256:00000000000000000000000000000000:719F2A6A3EF46E0E8928E97F28BD437126BB0A5C36B0A0F5EA332E3E6EB07B57"
"#,
    )
    .unwrap();

    let error = Config::load(dir.path().join("sqlite-fleet.toml"))
        .expect_err("case-only token_hash duplicate must be rejected")
        .to_string();
    assert!(
        error.contains("gui_users のtoken_hashが重複しています"),
        "{error}"
    );
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
