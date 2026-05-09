use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use tempfile::tempdir;

#[test]
fn doctor_uses_global_parallel_override() {
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
        .arg("doctor")
        .assert()
        .failure();
}

#[test]
fn doctor_json_reports_invalid_config_instead_of_failing_before_output() {
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

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--json")
        .arg("--parallel")
        .arg("0")
        .arg("doctor")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["config_ok"], false);
    assert_eq!(report["discovery_ok"], false);
    assert_eq!(report["migrations_ok"], false);
    assert!(report["errors"][0]
        .as_str()
        .unwrap()
        .contains("execution.parallel は1以上が必要です"));
}

#[test]
fn doctor_json_reports_toml_parse_error_instead_of_failing_before_output() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("sqlite-fleet.toml"), "[databases\n").unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--json")
        .arg("doctor")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["config_ok"], false);
    assert_eq!(report["discovery_ok"], false);
    assert_eq!(report["migrations_ok"], false);
    assert!(report["errors"][0]
        .as_str()
        .unwrap()
        .contains("設定ファイルのTOML解析に失敗しました"));
}

#[test]
fn doctor_json_reports_invalid_report_path_instead_of_failing_before_output() {
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
path = "../doctor-report.json"
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("--json")
        .arg("doctor")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["config_ok"], false);
    assert!(report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| error.as_str().unwrap().contains("report.path")));
}

#[test]
fn doctor_writes_configured_report_even_on_failure() {
    let dir = tempdir().unwrap();
    let report_path = dir.path().join("doctor-report.json");
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

    Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(dir.path().join("sqlite-fleet.toml"))
        .arg("doctor")
        .assert()
        .failure();

    let report_text = fs::read_to_string(report_path).unwrap();
    let report: Value = serde_json::from_str(&report_text).unwrap();
    assert_eq!(report["config_ok"], true);
    assert_eq!(report["migrations_ok"], false);
    assert!(!report["errors"].as_array().unwrap().is_empty());
}
