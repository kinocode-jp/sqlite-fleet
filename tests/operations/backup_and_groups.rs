use rusqlite::Connection;
use sqlite_fleet::{
    backup, check, load_migrations, migrate_with_options, restore, schema_drift, status_report,
    write_audit_event, BackupConfig, Config, DatabaseSelection, DatabasesConfig, MigrateOptions,
    MigrationGroupConfig, MigrationsConfig,
};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::{collections::HashMap, fs};
use tempfile::tempdir;

fn base_config(dir: &tempfile::TempDir) -> Config {
    Config {
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
        backup: BackupConfig {
            dir: "backups".to_string(),
            keep_last: 10,
            before_migrate: false,
        },
        ..Config::default()
    }
}

fn create_db(path: &std::path::Path, table: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(&format!("CREATE TABLE {table}(id INTEGER PRIMARY KEY);"))
        .unwrap();
}

fn table_exists(path: &std::path::Path, table: &str) -> bool {
    let conn = Connection::open(path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .unwrap();
    count == 1
}

fn operation_lock_path(path: &std::path::Path) -> std::path::PathBuf {
    let file_name = path.file_name().unwrap();
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".sqlite-fleet.lock");
    path.with_file_name(lock_name)
}

#[test]
fn backup_creates_sqlite_copy_for_selected_database() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    create_db(&dir.path().join("data").join("tenant.db"), "items");
    let config = base_config(&dir);

    let report = backup(
        &config,
        DatabaseSelection {
            database: Some("tenant".to_string()),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    assert_eq!(report.backed_up, 1);
    let backup_path = report.backups[0].path.as_ref().unwrap();
    let conn = Connection::open(backup_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'items'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn backup_removes_database_operation_lock_after_success() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "items");
    let config = base_config(&dir);

    let report = backup(
        &config,
        DatabaseSelection {
            database: Some("tenant".to_string()),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    assert_eq!(report.backed_up, 1);
    assert!(!operation_lock_path(&db).exists());
}

#[test]
fn backup_reports_existing_sqlite_fleet_operation_lock() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "items");
    fs::write(operation_lock_path(&db), "pid=999999\noperation=test\n").unwrap();
    let mut config = base_config(&dir);
    config.execution.lock_timeout_ms = 1;

    let report = backup(
        &config,
        DatabaseSelection {
            database: Some("tenant".to_string()),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    assert_eq!(report.failed, 1);
    assert!(report.backups[0]
        .error
        .as_deref()
        .unwrap_or("")
        .contains("別のsqlite-fleet操作中"));
}

#[test]
fn backup_preserves_rowids_for_tables_without_integer_primary_key() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE notes(body TEXT NOT NULL);
        INSERT INTO notes(rowid, body) VALUES (42, 'alpha'), (7, 'beta'), (99, 'gamma');
        "#,
    )
    .unwrap();
    drop(conn);
    let config = base_config(&dir);

    let report = backup(
        &config,
        DatabaseSelection {
            database: Some("tenant".to_string()),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    let backup_path = report.backups[0].path.as_ref().unwrap();
    let backup_conn = Connection::open(backup_path).unwrap();
    let rows = backup_conn
        .prepare("SELECT rowid, body FROM notes ORDER BY body")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (42, "alpha".to_string()),
            (7, "beta".to_string()),
            (99, "gamma".to_string())
        ]
    );
}

#[test]
fn backup_paths_do_not_collide_after_sanitizing_database_ids() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    create_db(&dir.path().join("data").join("a:b.db"), "colon_items");
    create_db(&dir.path().join("data").join("a?b.db"), "question_items");
    let mut config = base_config(&dir);
    config.backup.keep_last = 1;

    let report = backup(&config, DatabaseSelection::default()).unwrap();

    assert_eq!(report.backed_up, 2);
    let first_path = report.backups[0].path.as_ref().unwrap();
    let second_path = report.backups[1].path.as_ref().unwrap();
    assert_ne!(first_path.parent(), second_path.parent());
    let question_backup_path = report
        .backups
        .iter()
        .find(|backup| backup.database.id == "a?b")
        .unwrap()
        .path
        .as_ref()
        .unwrap()
        .clone();

    let second_colon_report = backup(
        &config,
        DatabaseSelection {
            database: Some("a:b".to_string()),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    assert_eq!(second_colon_report.backed_up, 1);
    assert!(question_backup_path.exists());
}

#[test]
fn backup_keep_last_counts_new_backup_in_retention_limit() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "items");
    let mut config = base_config(&dir);
    config.backup.keep_last = 1;

    let first_report = backup(&config, DatabaseSelection::default()).unwrap();
    let first_path = first_report.backups[0].path.as_ref().unwrap().clone();
    let second_report = backup(&config, DatabaseSelection::default()).unwrap();
    let second_path = second_report.backups[0].path.as_ref().unwrap().clone();

    assert!(!first_path.exists());
    assert!(second_path.exists());
    let backup_files = fs::read_dir(second_path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "db")
        })
        .count();
    assert_eq!(backup_files, 1);
}

#[test]
fn group_selection_preserves_selector_order_before_limit() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    create_db(&dir.path().join("data").join("tenant-a.db"), "a_items");
    create_db(&dir.path().join("data").join("tenant-b.db"), "b_items");
    let mut config = base_config(&dir);
    config.groups = HashMap::from([(
        "canary".to_string(),
        vec!["tenant-b".to_string(), "tenant-a".to_string()],
    )]);

    let report = backup(
        &config,
        DatabaseSelection {
            group: Some("canary".to_string()),
            limit: Some(1),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    assert_eq!(report.backed_up, 1);
    assert_eq!(report.backups[0].database.id, "tenant-b");
}

#[test]
fn group_selection_deduplicates_duplicate_selectors_before_limit() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    create_db(&dir.path().join("data").join("tenant-a.db"), "a_items");
    create_db(&dir.path().join("data").join("tenant-b.db"), "b_items");
    let mut config = base_config(&dir);
    config.groups = HashMap::from([(
        "canary".to_string(),
        vec![
            "tenant-b".to_string(),
            "data/tenant-b.db".to_string(),
            "tenant-a".to_string(),
        ],
    )]);

    let report = backup(
        &config,
        DatabaseSelection {
            group: Some("canary".to_string()),
            limit: Some(2),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    assert_eq!(report.backed_up, 2);
    assert_eq!(report.backups[0].database.id, "tenant-b");
    assert_eq!(report.backups[1].database.id, "tenant-a");
}

#[test]
fn group_selection_rejects_missing_selector_even_when_other_selectors_match() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    create_db(&dir.path().join("data").join("tenant-a.db"), "a_items");
    let mut config = base_config(&dir);
    config.groups = HashMap::from([(
        "canary".to_string(),
        vec!["tenant-a".to_string(), "tenant-b".to_string()],
    )]);

    let error = backup(
        &config,
        DatabaseSelection {
            group: Some("canary".to_string()),
            ..DatabaseSelection::default()
        },
    )
    .unwrap_err()
    .to_string();

    assert!(
        error.contains("指定されたDB group selectorが見つかりません: tenant-b"),
        "{error}"
    );
}

#[test]
fn db_groups_alias_selects_databases_in_configured_order() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    create_db(&dir.path().join("data").join("tenant-a.db"), "a_items");
    create_db(&dir.path().join("data").join("tenant-b.db"), "b_items");
    let mut config = base_config(&dir);
    config.db_groups = HashMap::from([(
        "canary".to_string(),
        vec!["tenant-b".to_string(), "tenant-a".to_string()],
    )]);

    let report = backup(
        &config,
        DatabaseSelection {
            group: Some("canary".to_string()),
            limit: Some(1),
            ..DatabaseSelection::default()
        },
    )
    .unwrap();

    assert_eq!(report.backed_up, 1);
    assert_eq!(report.backups[0].database.id, "tenant-b");
}

