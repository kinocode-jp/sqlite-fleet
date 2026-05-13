use rusqlite::{Connection, OptionalExtension};
use sqlite_fleet::{
    build_plan, check, checksum_sql, discover_databases, load_migrations, migrate, status_report,
    Config, DatabasesConfig, ExecutionConfig, MigrationsConfig,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn applies_migration_and_records_history() {
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
    Connection::open(&db_path).unwrap();
    let config = glob_migration_config(dir.path());

    let report = migrate(&config, false, None).unwrap();
    assert_eq!(report.failed_databases, 0);
    assert_eq!(report.pending_databases, 0);
    assert_eq!(report.applied_databases, 1);
    assert!(report.databases[0].pending.is_empty());
    assert_eq!(report.databases[0].applied.len(), 1);

    let conn = Connection::open(db_path).unwrap();
    let exists: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'items'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(exists.as_deref(), Some("items"));
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM _sqlite_fleet_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn dry_run_reports_without_applying_sql() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(
        migrations_dir.join("001_create_dry_run_items.sql"),
        "CREATE TABLE dry_run_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let db_path = data_dir.join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = glob_migration_config(dir.path());

    let report = migrate(&config, true, None).unwrap();
    assert_eq!(report.failed_databases, 0);
    assert_eq!(report.pending_databases, 1);
    assert_eq!(report.applied_databases, 0);
    assert_eq!(report.databases[0].pending.len(), 1);

    let conn = Connection::open(db_path).unwrap();
    let exists: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'dry_run_items'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(exists, None);

    let migration_table: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlite_fleet_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(migration_table, None);
}

#[test]
fn plan_does_not_create_migration_table() {
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
    Connection::open(&db_path).unwrap();
    let config = glob_migration_config(dir.path());

    let databases = discover_databases(&config).unwrap();
    let migrations = load_migrations(&config).unwrap();
    let plans = build_plan(&config, &databases, &migrations);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].pending.len(), 1);

    let conn = Connection::open(db_path).unwrap();
    let migration_table: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlite_fleet_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(migration_table, None);
}

#[test]
fn stop_on_error_reports_total_and_processed_counts() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(migrations_dir.join("001_bad.sql"), "CREATE TABLE broken(").unwrap();
    Connection::open(data_dir.join("a.db")).unwrap();
    Connection::open(data_dir.join("b.db")).unwrap();
    let config = Config {
        execution: ExecutionConfig {
            parallel: 1,
            continue_on_error: false,
            ..ExecutionConfig::default()
        },
        ..glob_migration_config(dir.path())
    };

    let report = migrate(&config, false, None).unwrap();
    assert_eq!(report.database_count, 2);
    assert_eq!(report.processed_databases, 1);
    assert_eq!(report.pending_databases, 1);
    assert_eq!(report.failed_databases, 1);
    assert_eq!(report.databases[0].pending.len(), 1);
}

#[test]
fn failed_migration_rolls_back_history_table_creation() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::write(migrations_dir.join("001_bad.sql"), "CREATE TABLE broken(").unwrap();
    let db_path = data_dir.join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = Config {
        execution: ExecutionConfig {
            parallel: 1,
            continue_on_error: true,
            ..ExecutionConfig::default()
        },
        ..glob_migration_config(dir.path())
    };

    let report = migrate(&config, false, None).unwrap();
    assert_eq!(report.failed_databases, 1);
    assert_eq!(report.pending_databases, 1);
    assert_eq!(report.databases[0].pending.len(), 1);
    assert!(report.databases[0].applied.is_empty());

    let conn = Connection::open(db_path).unwrap();
    let migration_table: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = '_sqlite_fleet_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(migration_table, None);
}

#[test]
fn unknown_applied_migration_is_reported_as_corrupt_and_blocks_migrate() {
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
            filename TEXT PRIMARY KEY NOT NULL,
            version TEXT NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO _sqlite_fleet_migrations (filename, version, name, checksum, applied_at, execution_ms)
         VALUES ('999_future_change.sql', '999', 'future_change', 'abc', 1, 1)",
        [],
    )
    .unwrap();
    let config = Config {
        execution: ExecutionConfig {
            parallel: 1,
            continue_on_error: true,
            ..ExecutionConfig::default()
        },
        ..glob_migration_config(dir.path())
    };

    let status = status_report(&config).unwrap();
    assert_eq!(status.corrupt, 1);
    assert_eq!(status.up_to_date, 0);
    assert_eq!(status.plans[0].unknown_applied[0].version, "999");

    let check_report = check(&config).unwrap();
    assert_eq!(check_report.failed, 1);
    assert_eq!(check_report.databases[0].unknown_applied[0].version, "999");

    let migrate_report = migrate(&config, false, None).unwrap();
    assert_eq!(migrate_report.failed_databases, 1);
    assert!(migrate_report.databases[0]
        .error
        .as_deref()
        .unwrap()
        .contains("ローカルに存在しない適用済みmigration"));
}

#[test]
fn checksum_error_reports_local_expected_and_database_actual() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    let sql = "CREATE TABLE items(id INTEGER PRIMARY KEY);";
    fs::write(migrations_dir.join("001_create_items.sql"), sql).unwrap();
    let db_path = data_dir.join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE _sqlite_fleet_migrations (
            filename TEXT PRIMARY KEY NOT NULL,
            version TEXT NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO _sqlite_fleet_migrations (filename, version, name, checksum, applied_at, execution_ms)
         VALUES ('001_create_items.sql', '001', 'create_items', 'stored-checksum', 1, 1)",
        [],
    )
    .unwrap();
    let config = glob_migration_config(dir.path());

    let status = status_report(&config).unwrap();
    let checksum_error = &status.plans[0].checksum_errors[0];
    assert_eq!(checksum_error.expected, checksum_sql(sql));
    assert_eq!(checksum_error.actual, "stored-checksum");
}

fn glob_migration_config(base_dir: &std::path::Path) -> Config {
    Config {
        base_dir: base_dir.to_path_buf(),
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
    }
}
