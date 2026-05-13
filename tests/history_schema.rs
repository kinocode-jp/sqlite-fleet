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
            filename TEXT,
            version TEXT NOT NULL,
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
            filename TEXT PRIMARY KEY,
            version TEXT NOT NULL,
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
            filename INTEGER PRIMARY KEY,
            version TEXT NOT NULL,
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
fn history_table_schema_rejects_composite_primary_key() {
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
            filename TEXT NOT NULL,
            version TEXT NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL,
            PRIMARY KEY(filename, version)
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
        .contains("主キーはfilename単独である必要があります"));
}

#[test]
fn ensure_migrations_table_rejects_existing_invalid_table() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE _sqlite_fleet_migrations (filename TEXT PRIMARY KEY)",
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
fn status_reads_legacy_version_primary_key_history() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    let migration_path = migrations_dir.join("001_create_items.sql");
    fs::write(
        &migration_path,
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let migration = parse_migration_file(&migration_path).unwrap();
    let db_path = data_dir.join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY);")
        .unwrap();
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
         VALUES (?1, ?2, ?3, 1, 1)",
        (&migration.version, &migration.name, &migration.checksum),
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
    assert_eq!(status.failed, 0);
    assert_eq!(status.up_to_date, 1);
    assert!(status.plans[0].pending.is_empty());
}

#[test]
fn migrate_upgrades_legacy_version_primary_key_history() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    let migration_path = migrations_dir.join("001_create_items.sql");
    fs::write(
        &migration_path,
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let migration = parse_migration_file(&migration_path).unwrap();
    let db_path = data_dir.join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY);")
        .unwrap();
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
         VALUES (?1, ?2, ?3, 1, 1)",
        (&migration.version, &migration.name, &migration.checksum),
    )
    .unwrap();
    drop(conn);
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
    let database = Database {
        id: "tenant".to_string(),
        path: db_path.clone(),
        exists: true,
        readable: true,
    };

    let result = migrate_database(&config, &database, &[migration], false);
    assert!(result.success, "{:?}", result.error);
    let conn = Connection::open(&db_path).unwrap();
    let applied = read_applied_migrations(&conn, "_sqlite_fleet_migrations").unwrap();
    assert_eq!(applied[0].filename, "001_create_items.sql");
}

#[test]
fn failed_migrate_does_not_upgrade_unresolved_legacy_history() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    let migration_path = migrations_dir.join("001_create_items.sql");
    fs::write(
        &migration_path,
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let migration = parse_migration_file(&migration_path).unwrap();
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
         VALUES ('999', 'missing', 'missing-checksum', 1, 1)",
        [],
    )
    .unwrap();
    drop(conn);
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
    let database = Database {
        id: "tenant".to_string(),
        path: db_path.clone(),
        exists: true,
        readable: true,
    };

    let result = migrate_database(&config, &database, &[migration], false);
    assert!(!result.success);
    assert!(result
        .error
        .as_deref()
        .unwrap()
        .contains("対象外またはローカルに存在しない適用済みmigration"));
    let conn = Connection::open(&db_path).unwrap();
    let filename_columns: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('_sqlite_fleet_migrations') WHERE name = 'filename'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(filename_columns, 0);
}

#[test]
fn migration_history_rejects_extra_columns() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE _sqlite_fleet_migrations (
            filename TEXT PRIMARY KEY NOT NULL,
            version TEXT NOT NULL,
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
