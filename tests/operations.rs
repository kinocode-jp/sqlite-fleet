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

#[test]
fn migration_groups_limit_pending_migrations_per_database() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/core")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/premium")).unwrap();
    create_db(
        &dir.path().join("data").join("tenant-core.db"),
        "base_items",
    );
    create_db(
        &dir.path().join("data").join("tenant-premium.db"),
        "base_items",
    );
    fs::write(
        dir.path()
            .join("migrations/core")
            .join("001_create_core_items.sql"),
        "CREATE TABLE core_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("migrations/premium")
            .join("101_create_premium_items.sql"),
        "CREATE TABLE premium_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let mut config = base_config(&dir);
    config.migration_groups = HashMap::from([
        (
            "core".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/core".to_string()),
        ),
        (
            "premium".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/premium".to_string()),
        ),
    ]);
    config.database_migration_groups = HashMap::from([
        ("tenant-core".to_string(), vec!["core".to_string()]),
        (
            "tenant-premium".to_string(),
            vec!["core".to_string(), "premium".to_string()],
        ),
    ]);

    let report = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();

    assert_eq!(report.applied_databases, 2);
    assert!(table_exists(
        &dir.path().join("data").join("tenant-core.db"),
        "core_items"
    ));
    assert!(!table_exists(
        &dir.path().join("data").join("tenant-core.db"),
        "premium_items"
    ));
    assert!(table_exists(
        &dir.path().join("data").join("tenant-premium.db"),
        "core_items"
    ));
    assert!(table_exists(
        &dir.path().join("data").join("tenant-premium.db"),
        "premium_items"
    ));
    let premium_result = report
        .databases
        .iter()
        .find(|result| result.database.id == "tenant-premium")
        .unwrap();
    assert_eq!(premium_result.applied[0].group, "core");
    assert_eq!(premium_result.applied[1].group, "premium");
}

#[test]
fn legacy_dir_migration_groups_do_not_require_default_migrations_dir() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("migrations/core")).unwrap();
    fs::write(
        dir.path()
            .join("migrations/core")
            .join("001_create_core_items.sql"),
        "CREATE TABLE core_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let mut config = base_config(&dir);
    config.migrations.dir = "missing-default-migrations-dir".to_string();
    config.migration_groups = HashMap::from([(
        "core".to_string(),
        MigrationGroupConfig::legacy_dir("migrations/core".to_string()),
    )]);

    let migrations = load_migrations(&config).unwrap();

    assert_eq!(migrations.len(), 1);
    assert_eq!(migrations[0].group, "core");
    assert_eq!(migrations[0].version, "001");
}

#[test]
fn migration_group_membership_can_share_fixed_migration_files() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "base_items");
    fs::write(
        dir.path().join("migrations").join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("migrations")
            .join("002_create_premium_items.sql"),
        "CREATE TABLE premium_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();

    let mut config = base_config(&dir);
    config.migration_groups = HashMap::from([
        (
            "core".to_string(),
            MigrationGroupConfig::versions(vec!["001".to_string()]),
        ),
        (
            "premium".to_string(),
            MigrationGroupConfig::versions(vec!["001".to_string(), "002".to_string()]),
        ),
    ]);
    config.database_migration_groups =
        HashMap::from([("tenant".to_string(), vec!["core".to_string()])]);

    let first = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();
    assert_eq!(first.databases[0].applied.len(), 1);
    assert!(table_exists(&db, "items"));
    assert!(!table_exists(&db, "premium_items"));

    config.database_migration_groups =
        HashMap::from([("tenant".to_string(), vec!["premium".to_string()])]);
    let second = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();

    assert_eq!(second.databases[0].applied.len(), 1);
    assert_eq!(second.databases[0].applied[0].version, "002");
    assert!(table_exists(&db, "items"));
    assert!(table_exists(&db, "premium_items"));
}

#[test]
fn runtime_migrate_rejects_unknown_database_migration_group() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/core")).unwrap();
    create_db(&dir.path().join("data").join("tenant.db"), "base_items");
    fs::write(
        dir.path()
            .join("migrations/core")
            .join("001_create_core_items.sql"),
        "CREATE TABLE core_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let mut config = base_config(&dir);
    config.migration_groups = HashMap::from([(
        "core".to_string(),
        MigrationGroupConfig::legacy_dir("migrations/core".to_string()),
    )]);
    config.database_migration_groups =
        HashMap::from([("tenant".to_string(), vec!["typo".to_string()])]);

    let error = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("マイグレーショングループが見つかりません: typo"));
    assert!(!table_exists(
        &dir.path().join("data").join("tenant.db"),
        "core_items"
    ));
}

#[test]
fn database_migration_groups_union_all_matching_selectors_deterministically() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/core")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/premium")).unwrap();
    let db_path = dir.path().join("data").join("tenant-a.db");
    create_db(&db_path, "base_items");
    fs::write(
        dir.path()
            .join("migrations/core")
            .join("001_create_core_items.sql"),
        "CREATE TABLE core_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("migrations/premium")
            .join("101_create_premium_items.sql"),
        "CREATE TABLE premium_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let mut config = base_config(&dir);
    config.migration_groups = HashMap::from([
        (
            "core".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/core".to_string()),
        ),
        (
            "premium".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/premium".to_string()),
        ),
    ]);
    config.database_migration_groups = HashMap::from([
        ("tenant-a".to_string(), vec!["core".to_string()]),
        ("data/tenant-a.db".to_string(), vec!["premium".to_string()]),
    ]);

    let report = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();

    assert_eq!(report.applied_databases, 1);
    assert!(table_exists(&db_path, "core_items"));
    assert!(table_exists(&db_path, "premium_items"));
    let applied_groups = report.databases[0]
        .applied
        .iter()
        .map(|migration| migration.group.as_str())
        .collect::<Vec<_>>();
    assert_eq!(applied_groups, vec!["core", "premium"]);
}

#[test]
fn check_reports_history_for_migration_group_that_is_no_longer_targeted() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/core")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/premium")).unwrap();
    let db_path = dir.path().join("data").join("tenant-a.db");
    create_db(&db_path, "base_items");
    fs::write(
        dir.path()
            .join("migrations/core")
            .join("001_create_core_items.sql"),
        "CREATE TABLE core_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("migrations/premium")
            .join("101_create_premium_items.sql"),
        "CREATE TABLE premium_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let mut config = base_config(&dir);
    config.migration_groups = HashMap::from([
        (
            "core".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/core".to_string()),
        ),
        (
            "premium".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/premium".to_string()),
        ),
    ]);
    config.database_migration_groups = HashMap::from([(
        "tenant-a".to_string(),
        vec!["core".to_string(), "premium".to_string()],
    )]);
    migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();

    config
        .database_migration_groups
        .insert("tenant-a".to_string(), vec!["core".to_string()]);

    let check_report = check(&config).unwrap();
    assert_eq!(check_report.ok, 0);
    assert_eq!(check_report.failed, 1);
    assert!(!check_report.databases[0].success);
    assert_eq!(check_report.databases[0].unknown_applied[0].version, "101");
    assert!(check_report.databases[0].checksum_errors.is_empty());

    let status = status_report(&config).unwrap();
    assert_eq!(status.corrupt, 1);
    assert_eq!(status.failed, 0);
    assert_eq!(status.plans[0].unknown_applied[0].version, "101");
    assert!(status.plans[0].checksum_errors.is_empty());
}

#[test]
fn migrate_blocks_history_for_migration_group_that_is_no_longer_targeted() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/core")).unwrap();
    fs::create_dir_all(dir.path().join("migrations/premium")).unwrap();
    let db_path = dir.path().join("data").join("tenant-a.db");
    create_db(&db_path, "base_items");
    fs::write(
        dir.path()
            .join("migrations/core")
            .join("001_create_core_items.sql"),
        "CREATE TABLE core_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("migrations/premium")
            .join("101_create_premium_items.sql"),
        "CREATE TABLE premium_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    let mut config = base_config(&dir);
    config.migration_groups = HashMap::from([
        (
            "core".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/core".to_string()),
        ),
        (
            "premium".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/premium".to_string()),
        ),
    ]);
    config.database_migration_groups = HashMap::from([(
        "tenant-a".to_string(),
        vec!["core".to_string(), "premium".to_string()],
    )]);
    migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();
    fs::write(
        dir.path()
            .join("migrations/core")
            .join("002_create_more_core_items.sql"),
        "CREATE TABLE more_core_items(id INTEGER PRIMARY KEY);",
    )
    .unwrap();
    config
        .database_migration_groups
        .insert("tenant-a".to_string(), vec!["core".to_string()]);

    let dry_run = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: true,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();
    assert_eq!(dry_run.failed_databases, 1);
    assert_eq!(dry_run.pending_databases, 0);
    assert!(dry_run.databases[0]
        .error
        .as_deref()
        .unwrap()
        .contains("対象外またはローカルに存在しない適用済みmigration"));

    let report = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection::default(),
            backup_before_migrate: Some(false),
        },
    )
    .unwrap();
    assert_eq!(report.failed_databases, 1);
    assert_eq!(report.applied_databases, 0);
    assert!(!table_exists(&db_path, "more_core_items"));
}

#[test]
fn restore_replaces_database_after_taking_pre_restore_backup() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let config = base_config(&dir);
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap().clone();
    Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);")
        .unwrap();

    let report = restore(&config, "tenant", &backup_path).unwrap();

    assert!(report.success, "{report:?}");
    assert!(report.pre_restore_backup.unwrap().success);
    let conn = Connection::open(&db).unwrap();
    let old_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'old_items'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_count, 1);
}

#[test]
fn restore_accepts_backup_path_relative_to_config_base_dir() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let config = base_config(&dir);
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap();
    let relative_backup_path = backup_path.strip_prefix(dir.path()).unwrap().to_path_buf();
    Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);")
        .unwrap();

    let report = restore(&config, "tenant", &relative_backup_path).unwrap();

    assert!(report.success, "{report:?}");
    assert_eq!(report.restored_from, backup_path.to_path_buf());
    let conn = Connection::open(&db).unwrap();
    let old_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'old_items'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_count, 1);
}

#[test]
fn restore_from_wal_source_includes_uncheckpointed_transactions() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    fs::create_dir_all(dir.path().join("external")).unwrap();
    let target = dir.path().join("data").join("tenant.db");
    let source = dir.path().join("external").join("source.db");
    create_db(&target, "old_items");
    let source_conn = Connection::open(&source).unwrap();
    source_conn
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA wal_autocheckpoint = 0;
             CREATE TABLE restored_items(id INTEGER PRIMARY KEY, name TEXT);
             INSERT INTO restored_items(name) VALUES ('from wal');",
        )
        .unwrap();
    assert!(
        source.with_extension("db-wal").exists(),
        "test setup should leave a WAL sidecar next to the restore source"
    );
    let config = base_config(&dir);

    let report = restore(&config, "tenant", &source).unwrap();

    assert!(report.success, "{report:?}");
    assert!(table_exists(&target, "restored_items"));
    let restored_conn = Connection::open(&target).unwrap();
    let restored_name: String = restored_conn
        .query_row("SELECT name FROM restored_items WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(restored_name, "from wal");
}

#[test]
fn restore_does_not_prune_restore_source_when_creating_pre_restore_backup() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let mut config = base_config(&dir);
    config.backup.keep_last = 1;
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap().clone();
    Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);")
        .unwrap();

    let report = restore(&config, "tenant", &backup_path).unwrap();

    assert!(report.success, "{report:?}");
    assert!(report.pre_restore_backup.unwrap().path.unwrap().exists());
    assert!(backup_path.exists());
    assert!(table_exists(&db, "old_items"));
}

#[test]
fn restore_pre_backup_failure_keeps_failed_backup_details_in_report() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let mut config = base_config(&dir);
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap().clone();
    Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);")
        .unwrap();
    fs::write(dir.path().join("blocked-backups"), "not a directory").unwrap();
    config.backup.dir = "blocked-backups".to_string();

    let report = restore(&config, "tenant", &backup_path).unwrap();

    assert!(!report.success);
    let pre_restore_backup = report
        .pre_restore_backup
        .as_ref()
        .expect("failed pre-restore backup should be reported");
    assert!(!pre_restore_backup.success);
    assert!(pre_restore_backup.path.is_none());
    assert_eq!(report.error, pre_restore_backup.error);
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("backup ディレクトリ"));
    assert!(table_exists(&db, "new_items"));
    assert!(!table_exists(&db, "old_items"));
}

#[cfg(unix)]
#[test]
fn restore_failure_after_pre_backup_keeps_pre_backup_in_report() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let config = base_config(&dir);
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap().clone();
    Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);")
        .unwrap();
    let mut permissions = fs::metadata(&db).unwrap().permissions();
    permissions.set_mode(0o444);
    fs::set_permissions(&db, permissions).unwrap();

    let report = restore(&config, "tenant", &backup_path).unwrap();

    let mut permissions = fs::metadata(&db).unwrap().permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&db, permissions).unwrap();
    assert!(!report.success);
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("読み取り専用"));
    let pre_restore_path = report
        .pre_restore_backup
        .as_ref()
        .and_then(|backup| backup.path.as_ref())
        .expect("pre-restore backup path should be reported after later restore failure");
    assert!(pre_restore_path.exists());
    assert!(table_exists(&db, "new_items"));
}

#[test]
fn restore_fails_while_concurrent_writer_holds_database_lock() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let mut config = base_config(&dir);
    config.execution.lock_timeout_ms = 50;
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap().clone();
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "PRAGMA journal_mode=WAL; DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);",
        )
        .unwrap();
    let writer = Connection::open(&db).unwrap();
    writer
        .execute_batch("BEGIN IMMEDIATE; INSERT INTO new_items(id) VALUES (1);")
        .unwrap();

    let report = restore(&config, "tenant", &backup_path).unwrap();

    writer.execute_batch("ROLLBACK;").unwrap();
    assert!(!report.success);
    assert!(report.error.as_deref().unwrap_or("").contains("排他ロック"));
    assert!(table_exists(&db, "new_items"));
}

#[cfg(unix)]
#[test]
fn restore_rewrites_existing_database_file_without_replacing_inode() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let config = base_config(&dir);
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap().clone();
    Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);")
        .unwrap();
    let inode_before = fs::metadata(&db).unwrap().ino();

    let report = restore(&config, "tenant", &backup_path).unwrap();

    let inode_after = fs::metadata(&db).unwrap().ino();
    assert!(report.success);
    assert_eq!(inode_before, inode_after);
    assert!(table_exists(&db, "old_items"));
    assert!(!table_exists(&db, "new_items"));
}

#[test]
fn restore_preserves_wal_sidecars_for_sqlite_to_manage() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "old_items");
    let config = base_config(&dir);
    let backup_report = backup(&config, DatabaseSelection::default()).unwrap();
    let backup_path = backup_report.backups[0].path.as_ref().unwrap().clone();
    let writer = Connection::open(&db).unwrap();
    writer
        .execute_batch(
            "PRAGMA journal_mode=WAL; DROP TABLE old_items; CREATE TABLE new_items(id INTEGER);",
        )
        .unwrap();
    let reader = Connection::open(&db).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM new_items;")
        .unwrap();
    assert!(db.with_extension("db-wal").exists());

    let report = restore(&config, "tenant", &backup_path).unwrap();

    reader.execute_batch("ROLLBACK;").unwrap();
    assert!(report.success, "{report:?}");
    assert!(table_exists(&db, "old_items"));
    assert!(!table_exists(&db, "new_items"));
}

#[test]
fn restore_rejects_non_sqlite_backup_without_replacing_target() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    fs::create_dir_all(dir.path().join("backups")).unwrap();
    let db = dir.path().join("data").join("tenant.db");
    create_db(&db, "live_items");
    let bad_backup = dir.path().join("backups").join("not-a-database.db");
    fs::write(&bad_backup, "not sqlite").unwrap();
    let config = base_config(&dir);

    let report = restore(&config, "tenant", &bad_backup).unwrap();

    assert!(!report.success);
    assert!(report
        .error
        .as_deref()
        .unwrap_or("")
        .contains("restore元backup"));
    assert!(table_exists(&db, "live_items"));
}

#[test]
fn migrate_can_backup_before_apply_and_select_group_with_limit() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    Connection::open(dir.path().join("data").join("a.db")).unwrap();
    Connection::open(dir.path().join("data").join("b.db")).unwrap();
    fs::write(
        dir.path().join("migrations").join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER);",
    )
    .unwrap();
    let mut config = base_config(&dir);
    config.groups = HashMap::from([("canary".to_string(), vec!["a".to_string(), "b".to_string()])]);

    let report = migrate_with_options(
        &config,
        MigrateOptions {
            dry_run: false,
            selection: DatabaseSelection {
                group: Some("canary".to_string()),
                limit: Some(1),
                ..DatabaseSelection::default()
            },
            backup_before_migrate: Some(true),
        },
    )
    .unwrap();

    assert_eq!(report.processed_databases, 1);
    assert!(report.databases[0].pre_backup.as_ref().unwrap().success);
    assert_eq!(report.applied_databases, 1);
}

#[test]
fn schema_drift_reports_changed_schema_against_first_database() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("migrations")).unwrap();
    create_db(&dir.path().join("data").join("a.db"), "items");
    create_db(&dir.path().join("data").join("b.db"), "orders");
    let config = base_config(&dir);

    let report = schema_drift(&config, DatabaseSelection::default()).unwrap();

    assert_eq!(report.drifted, 1);
    assert!(report
        .databases
        .iter()
        .any(|database| database.database.id == "b" && !database.matches_baseline));
}

#[test]
fn audit_log_appends_jsonl_events() {
    let dir = tempdir().unwrap();
    let mut config = base_config(&dir);
    config.audit.path = Some("audit.jsonl".to_string());

    write_audit_event(&config, "unit.test", &serde_json::json!({"ok": true})).unwrap();

    let text = fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
    assert!(text.contains(r#""operation":"unit.test""#));
    assert!(text.ends_with('\n'));
}
