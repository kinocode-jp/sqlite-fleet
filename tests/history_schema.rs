use rusqlite::Connection;
use sqlite_fleet::{
    ensure_migrations_table, migrate_database, parse_migration_file, read_applied_migrations,
    status_report, Config, Database, DatabasesConfig, MigrationsConfig,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn history_table_schema_rejects_missing_primary_key_constraint() {
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
        "CREATE TABLE _sqlite_fleet_migrations (
            version TEXT,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )",
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

    let status = status_report(&config).unwrap();
    assert_eq!(status.failed, 1);
    assert!(status.plans[0]
        .error
        .as_deref()
        .unwrap()
        .contains("主キー制約が不足しています"));
}

#[test]
fn history_table_schema_rejects_nullable_version_primary_key() {
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
        "CREATE TABLE _sqlite_fleet_migrations (
            version TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )",
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

    let status = status_report(&config).unwrap();
    assert_eq!(status.failed, 1);
    assert!(status.plans[0]
        .error
        .as_deref()
        .unwrap()
        .contains("NOT NULL制約が不足しています"));
}

#[test]
fn history_table_schema_rejects_wrong_column_types() {
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
        "CREATE TABLE _sqlite_fleet_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )",
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

    let status = status_report(&config).unwrap();
    assert_eq!(status.failed, 1);
    assert!(status.plans[0]
        .error
        .as_deref()
        .unwrap()
        .contains("migration 管理テーブルの列型が不正です"));
}

#[test]
fn ensure_migrations_table_rejects_existing_invalid_table() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE _sqlite_fleet_migrations (version TEXT PRIMARY KEY)",
        [],
    )
    .unwrap();

    let error = ensure_migrations_table(&conn, "_sqlite_fleet_migrations")
        .unwrap_err()
        .to_string();
    assert!(error.contains("列が不足しています"));
}

#[test]
fn migration_history_uses_main_schema_when_temp_table_shadows_name() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    Connection::open(&db_path).unwrap();

    let migration_path = dir.path().join("001_temp_shadow.sql");
    fs::write(
        &migration_path,
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let migration = parse_migration_file(&migration_path).unwrap();

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: db_path.clone(),
        exists: true,
        readable: true,
    };

    let result = migrate_database(&config, &database, &[migration], false);
    assert!(result.success, "{:?}", result.error);

    let conn = Connection::open(&db_path).unwrap();
    conn.execute("CREATE TEMP TABLE _sqlite_fleet_migrations(dummy TEXT)", [])
        .unwrap();
    let applied_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM main._sqlite_fleet_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(applied_count, 1);
    let applied = read_applied_migrations(&conn, "_sqlite_fleet_migrations").unwrap();
    assert_eq!(applied.len(), 1);
}

#[test]
fn migration_history_rejects_extra_columns() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE _sqlite_fleet_migrations (
            version TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL,
            operator TEXT NOT NULL
        )",
        [],
    )
    .unwrap();

    let error = read_applied_migrations(&conn, "_sqlite_fleet_migrations")
        .unwrap_err()
        .to_string();
    assert!(error.contains("想定外の列があります"), "{error}");
}
