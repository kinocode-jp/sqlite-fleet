use assert_cmd::Command;
use rusqlite::Connection;
use sqlite_fleet::{
    check, discover_databases, migrate, parse_migration_file, render_path_template, Config,
    DatabasesConfig, ExecutionConfig, MigrationsConfig,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn query_discovery_rejects_lexically_duplicate_paths() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant_a', 'data/tenant.db')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant_b', './data/tenant.db')",
        [],
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id, db_path FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: None,
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DBパスが重複しています"));
}

#[test]
fn parse_migration_rejects_empty_sql() {
    let dir = tempdir().unwrap();
    let migration_path = dir.path().join("001_empty.sql");
    fs::write(&migration_path, " \n\t").unwrap();

    let error = parse_migration_file(&migration_path)
        .unwrap_err()
        .to_string();
    assert!(error.contains("migration SQL が空です"));
}

#[test]
fn query_discovery_rejects_blank_id_and_path_values() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('  ', 'data/tenant.db')",
        [],
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id, db_path FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: None,
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("id_column を取得できません"));
}

#[cfg(unix)]
#[test]
fn glob_discovery_rejects_symlinked_duplicate_database() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let real_db = data_dir.join("real.db");
    let alias_db = data_dir.join("alias.db");
    Connection::open(&real_db).unwrap();
    symlink(&real_db, &alias_db).unwrap();
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
    assert!(error.contains("DBパスが重複しています"));
}

#[test]
fn migrate_rejects_invalid_history_table_before_applying_sql() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        migrations_dir.join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let db_path = data_dir.join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE _sqlite_fleet_migrations (version TEXT PRIMARY KEY)",
        [],
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            ..MigrationsConfig::default()
        },
        execution: ExecutionConfig {
            parallel: 1,
            ..ExecutionConfig::default()
        },
        ..Config::default()
    };

    let report = migrate(&config, false, None).unwrap();
    assert_eq!(report.failed_databases, 1);
    assert_eq!(report.pending_databases, 0);
    assert!(report.databases[0].pending.is_empty());
    assert!(report.databases[0]
        .error
        .as_deref()
        .unwrap()
        .contains("migration 管理テーブルの列が不足しています"));

    let items_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'items')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!items_exists);
}

#[test]
fn status_reports_history_name_used_by_view_as_failure() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        migrations_dir.join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let db_path = data_dir.join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE VIEW _sqlite_fleet_migrations AS SELECT '001' AS version",
        [],
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };

    let status = sqlite_fleet::status_report(&config).unwrap();
    assert_eq!(status.failed, 1);
    assert!(status.plans[0]
        .error
        .as_deref()
        .unwrap()
        .contains("table以外で使用されています"));
}

#[test]
fn config_rejects_blank_optional_operational_fields() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id, db_path FROM tenants"
id_column = ""
path_column = "db_path"

[migrations]
dir = "migrations"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("databases.id_column"));

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id, db_path FROM tenants"
path_column = "db_path"

[migrations]
dir = "migrations"
"#,
    )
    .unwrap();
    Config::load(&config_path).unwrap();

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = ""
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("migrations.dir は空にできません"));

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[report]
path = " "
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("report.path は空にできません"));
}

#[test]
fn migrate_and_check_reject_empty_database_set() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::write(
        migrations_dir.join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };

    let migrate_error = migrate(&config, false, None).unwrap_err().to_string();
    assert!(migrate_error.contains("対象DBが見つかりません"));
    let check_error = check(&config).unwrap_err().to_string();
    assert!(check_error.contains("対象DBが見つかりません"));
    let status_error = sqlite_fleet::status_report(&config)
        .unwrap_err()
        .to_string();
    assert!(status_error.contains("対象DBが見つかりません"));
}

#[test]
fn plan_cli_rejects_empty_database_set() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::write(
        migrations_dir.join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "migrations"
"#,
    )
    .unwrap();

    Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(&config_path)
        .arg("plan")
        .assert()
        .failure();
}

#[test]
fn path_template_rejects_path_like_ids() {
    let slash_error = render_path_template("data/{id}.db", "../outside")
        .unwrap_err()
        .to_string();
    assert!(slash_error.contains("path_template に埋め込むIDとして不正です"));

    let dot_error = render_path_template("data/{id}.db", "..")
        .unwrap_err()
        .to_string();
    assert!(dot_error.contains("path_template に埋め込むIDとして不正です"));
}

#[test]
fn query_discovery_rejects_path_template_id_traversal() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('../outside')", [])
        .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("path_template に埋め込むIDとして不正です"));
}

#[test]
fn query_discovery_rejects_path_column_outside_base_dir() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', '../outside.db')",
        [],
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id, db_path FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: None,
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DBパスが設定ディレクトリ外"));
}

#[test]
fn query_discovery_rejects_absolute_path_column_outside_base_dir() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES (?1, ?2)",
        rusqlite::params!["tenant", outside.path().join("tenant.db").to_string_lossy()],
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id, db_path FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: None,
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DBパスが設定ディレクトリ外"));
}

#[cfg(unix)]
#[test]
fn query_discovery_rejects_symlinked_path_column_outside_base_dir() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    let outside_db = outside.path().join("tenant.db");
    Connection::open(&outside_db).unwrap();
    symlink(&outside_db, dir.path().join("linked.db")).unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', 'linked.db')",
        [],
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id, db_path FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: None,
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DBパスが設定ディレクトリ外"));
}

#[cfg(unix)]
#[test]
fn glob_discovery_rejects_symlinked_database_outside_base_dir() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let outside_db = outside.path().join("tenant.db");
    Connection::open(&outside_db).unwrap();
    symlink(&outside_db, data_dir.join("tenant.db")).unwrap();
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
    assert!(error.contains("DBパスが設定ディレクトリ外"));
}

#[test]
fn config_rejects_paths_outside_base_dir() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "../data/*.db"

[migrations]
dir = "migrations"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("databases.path_glob"));

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "../shared.db"
query = "SELECT id FROM tenants"
path_template = "data/{id}.db"

[migrations]
dir = "migrations"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("databases.source"));

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id FROM tenants"
path_template = "../data/{id}.db"

[migrations]
dir = "migrations"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("databases.path_template"));

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "../migrations"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("migrations.dir"));

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[report]
path = "../report.json"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("report.path"));
}
