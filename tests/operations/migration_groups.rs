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

