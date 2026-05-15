use rusqlite::Connection;
use sqlite_fleet::{
    discover_databases, load_migrations, write_report_json, Config, DatabasesConfig,
    MigrationsConfig, ReportConfig, SecurityConfig,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn runtime_file_operations_reject_paths_outside_base_dir() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: "../migrations".to_string(),
            ..MigrationsConfig::default()
        },
        report: ReportConfig {
            format: "json".to_string(),
            path: Some("../report.json".to_string()),
        },
        ..Config::default()
    };

    let migration_error = load_migrations(&config).unwrap_err().to_string();
    assert!(migration_error.contains("migrations.dir"));

    let report_error = write_report_json(&config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(report_error.contains("report.path"));
    assert!(!dir.path().join("../report.json").exists());
}

#[test]
fn configured_output_and_migration_paths_reject_parent_components() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "data/../migrations"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("migrations.dir に親ディレクトリ成分"),
        "{error}"
    );

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: "data/../migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };
    let error = load_migrations(&config).unwrap_err().to_string();
    assert!(
        error.contains("migrations.dir に親ディレクトリ成分"),
        "{error}"
    );

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "migrations"

[report]
path = "reports/../report.json"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("report.path に親ディレクトリ成分"),
        "{error}"
    );

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: "json".to_string(),
            path: Some("reports/../report.json".to_string()),
        },
        ..Config::default()
    };
    let error = write_report_json(&config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("report.path に親ディレクトリ成分"),
        "{error}"
    );
    assert!(!dir.path().join("report.json").exists());
}

#[test]
fn configured_output_and_migration_paths_reject_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = " migrations"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("migrations.dir の前後に空白"), "{error}");

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: "migrations ".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };
    let error = load_migrations(&config).unwrap_err().to_string();
    assert!(error.contains("migrations.dir の前後に空白"), "{error}");

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "migrations"

[report]
path = " reports/report.json"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("report.path の前後に空白"), "{error}");

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: "json".to_string(),
            path: Some("reports/report.json ".to_string()),
        },
        ..Config::default()
    };
    let error = write_report_json(&config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("report.path の前後に空白"), "{error}");
}

#[test]
fn report_format_rejects_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "migrations"

[report]
format = "json "
path = "reports/report.json"
"#,
    )
    .unwrap();

    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("report.format の前後に空白"), "{error}");

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: " json".to_string(),
            path: Some("reports/report.json".to_string()),
        },
        ..Config::default()
    };
    let error = write_report_json(&config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("report.format の前後に空白"), "{error}");
}

#[test]
fn runtime_discovery_rejects_configured_paths_outside_base_dir() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_db = outside.path().join("shared.db");
    Connection::open(&outside_db).unwrap();

    let glob_config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some(outside.path().join("*.db").to_string_lossy().to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };
    let glob_error = discover_databases(&glob_config).unwrap_err().to_string();
    assert!(glob_error.contains("databases.path_glob"));

    let query_config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            source: Some(outside_db.to_string_lossy().to_string()),
            query: Some("SELECT 'tenant' AS id".to_string()),
            path_template: Some("data/{id}.db".to_string()),
            path_glob: None,
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };
    let query_error = discover_databases(&query_config).unwrap_err().to_string();
    assert!(query_error.contains("databases.source"));
}

#[cfg(unix)]
#[test]
fn report_output_rejects_symlinked_report_path() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_report = outside.path().join("report.json");
    let linked_report = dir.path().join("report.json");
    symlink(&outside_report, &linked_report).unwrap();

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: "json".to_string(),
            path: Some("./report.json".to_string()),
        },
        ..Config::default()
    };

    let error = write_report_json(&config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("report.path"));
    assert!(error.contains("シンボリックリンク"));
    assert!(!outside_report.exists());
}

#[cfg(unix)]
#[test]
fn report_output_rejects_symlinked_parent_directory_outside_base() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let linked_dir = dir.path().join("reports");
    symlink(outside.path(), &linked_dir).unwrap();

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: "json".to_string(),
            path: Some("./reports/report.json".to_string()),
        },
        ..Config::default()
    };

    let error = write_report_json(&config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("report.path"));
    assert!(!outside.path().join("report.json").exists());
}

#[cfg(unix)]
#[test]
fn report_output_rejects_nested_symlinked_parent_before_creating_directories() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let linked_dir = dir.path().join("reports");
    symlink(outside.path(), &linked_dir).unwrap();

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: "json".to_string(),
            path: Some("./reports/nested/report.json".to_string()),
        },
        ..Config::default()
    };

    let error = write_report_json(&config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("report.path"));
    assert!(!outside.path().join("nested").exists());
}

#[test]
fn report_output_replaces_existing_file_atomically() {
    let dir = tempdir().unwrap();
    let report_path = dir.path().join("report.json");
    fs::write(&report_path, r#"{"old":true}"#).unwrap();

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: "json".to_string(),
            path: Some("./report.json".to_string()),
        },
        ..Config::default()
    };

    write_report_json(&config, &serde_json::json!({"ok": true})).unwrap();

    let text = fs::read_to_string(report_path).unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["ok"], true);
}

#[test]
fn allowed_roots_default_keeps_paths_under_config_directory() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    Connection::open(outside.path().join("tenant.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some(outside.path().join("*.db").display().to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("databases.path_glob"));
    assert!(error.contains("allowed_roots"));
}

#[test]
fn allowed_roots_absolute_path_enables_external_glob_discovery() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    Connection::open(outside.path().join("tenant.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        security: SecurityConfig {
            allowed_roots: vec![".".to_string(), outside.path().display().to_string()],
        },
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some(outside.path().join("*.db").display().to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let databases = discover_databases(&config).unwrap();
    assert_eq!(databases.len(), 1);
    assert_eq!(databases[0].id, "tenant");
}

#[cfg(unix)]
#[test]
fn allowed_roots_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    Connection::open(outside.path().join("tenant.db")).unwrap();
    let data = dir.path().join("data");
    fs::create_dir(&data).unwrap();
    symlink(outside.path().join("tenant.db"), data.join("tenant.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DBパス"));
    assert!(error.contains("許可ルート外"));
}
