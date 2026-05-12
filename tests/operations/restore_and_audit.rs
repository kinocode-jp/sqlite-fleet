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
