use assert_cmd::Command;
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

#[test]
fn operation_failure_takes_precedence_over_report_write_failure() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir(dir.path().join("report.json")).unwrap();
    fs::write(
        migrations_dir.join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let db_path = data_dir.join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE _sqlite_fleet_migrations (
            version TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO _sqlite_fleet_migrations (version, name, checksum, applied_at, execution_ms)
         VALUES ('999', 'future_change', 'abc', 1, 1)",
        [],
    )
    .unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
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
continue_on_error = true

[report]
format = "json"
path = "./report.json"
"#,
    )
    .unwrap();

    for (command, expected) in [
        ("status", "status で問題を検出しました"),
        ("plan", "plan で問題を検出しました"),
        ("check", "DB検査に失敗しました"),
    ] {
        let output = Command::cargo_bin("sqlite-fleet")
            .unwrap()
            .arg("--config")
            .arg(&config_path)
            .arg(command)
            .output()
            .unwrap();

        assert!(!output.status.success(), "{command}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(expected), "{command}: {stderr}");
        assert!(!stderr.contains("report.path"), "{command}: {stderr}");
    }
}

#[test]
fn migrate_cli_continue_on_error_overrides_config() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(migrations_dir.join("001_bad.sql"), "CREATE TABLE broken(").unwrap();
    Connection::open(data_dir.join("a.db")).unwrap();
    Connection::open(data_dir.join("b.db")).unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
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
format = "json"
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(&config_path)
        .arg("--json")
        .arg("migrate")
        .arg("--continue-on-error")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["database_count"], 2);
    assert_eq!(report["processed_databases"], 2);
    assert_eq!(report["pending_databases"], 2);
    assert_eq!(report["failed_databases"], 2);
    assert_eq!(
        report["databases"][0]["pending"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn migrate_failure_takes_precedence_over_report_write_failure() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir(dir.path().join("report.json")).unwrap();
    fs::write(migrations_dir.join("001_bad.sql"), "CREATE TABLE broken(").unwrap();
    Connection::open(data_dir.join("tenant.db")).unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
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
format = "json"
path = "./report.json"
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(&config_path)
        .arg("migrate")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("マイグレーションに失敗しました"),
        "{stderr}"
    );
    assert!(!stderr.contains("report.path"), "{stderr}");
}

#[test]
fn global_parallel_override_is_validated() {
    let dir = tempdir().unwrap();
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
format = "json"
"#,
    )
    .unwrap();

    Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--parallel")
        .arg("0")
        .arg("plan")
        .assert()
        .failure();
}

#[test]
fn discover_does_not_require_migration_or_report_settings() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    Connection::open(data_dir.join("tenant.db")).unwrap();
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
format = "json"
path = "../report.json"
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--json")
        .arg("discover")
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let databases: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(databases.as_array().unwrap().len(), 1);
}

#[test]
fn status_json_stdout_survives_report_write_failure() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir(dir.path().join("report.json")).unwrap();
    Connection::open(data_dir.join("tenant.db")).unwrap();
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
format = "json"
path = "./report.json"
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--json")
        .arg("status")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["database_count"], 1);
}

#[test]
fn status_json_stdout_survives_invalid_report_path_config() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    Connection::open(data_dir.join("tenant.db")).unwrap();
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
format = "json"
path = "../report.json"
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--json")
        .arg("status")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["database_count"], 1);
}

#[test]
fn status_json_stdout_survives_invalid_report_format_config() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    Connection::open(data_dir.join("tenant.db")).unwrap();
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
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--json")
        .arg("status")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["database_count"], 1);
}
