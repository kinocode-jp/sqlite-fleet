use rusqlite::Connection;
use sqlite_fleet::{migrate, Config, DatabasesConfig, ExecutionConfig, MigrationsConfig};
use std::fs;
use tempfile::tempdir;

#[test]
fn migrate_database_selector_accepts_config_relative_path() {
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
    Connection::open(data_dir.join("tenant.db")).unwrap();
    Connection::open(data_dir.join("other.db")).unwrap();
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

    let report = migrate(&config, false, Some("./data/tenant.db")).unwrap();
    assert_eq!(report.database_count, 1);
    assert_eq!(report.applied_databases, 1);

    let tenant_conn = Connection::open(data_dir.join("tenant.db")).unwrap();
    tenant_conn
        .query_row("SELECT COUNT(*) FROM items", [], |_| Ok(()))
        .unwrap();
    let other_conn = Connection::open(data_dir.join("other.db")).unwrap();
    let other_has_items: bool = other_conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'items')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!other_has_items);
}

#[cfg(unix)]
#[test]
fn migrate_database_selector_accepts_canonical_path_for_symlinked_database() {
    use std::os::unix::fs::symlink;

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
    let real_db = data_dir.join("real.db");
    let link_db = data_dir.join("tenant.db");
    Connection::open(&real_db).unwrap();
    symlink(&real_db, &link_db).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/tenant.db".to_string()),
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

    let report = migrate(&config, false, Some(&real_db.to_string_lossy())).unwrap();
    assert_eq!(report.database_count, 1);
    assert_eq!(report.applied_databases, 1);
}
