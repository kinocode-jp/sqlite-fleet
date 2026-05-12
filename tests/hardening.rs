use rusqlite::Connection;
use sqlite_fleet::{
    build_database_plan, check, check_database, discover_databases, ensure_migrations_table,
    load_migrations, migrate, migrate_database, status_report, write_report_json, Config, Database,
    DatabasesConfig, ExecutionConfig, Migration, MigrationGroupConfig, MigrationsConfig,
    ReportConfig,
};
use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;

#[test]
fn migration_table_identifier_rejects_leading_digit() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    let conn = Connection::open(&db_path).unwrap();

    let error = ensure_migrations_table(&conn, "1_history")
        .unwrap_err()
        .to_string();
    assert!(error.contains("SQLite識別子として不正です"));

    let config = Config {
        migrations: MigrationsConfig {
            table: "1_history".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("SQLite識別子として不正です"));
}

#[test]
fn direct_database_apis_reject_database_outside_base_dir() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let db_path = outside.path().join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: db_path,
        exists: true,
        readable: true,
    };

    let plan = build_database_plan(&config, &database, &[]);
    assert!(plan.error.unwrap().contains("DBパス"));

    let migrate_result = migrate_database(&config, &database, &[], false);
    assert!(!migrate_result.success);
    assert!(migrate_result.error.unwrap().contains("DBパス"));

    let check_result = check_database(&config, &database, &[]);
    assert!(!check_result.success);
    assert!(check_result.error.unwrap().contains("DBパス"));
}

#[test]
fn migrate_rejects_zero_parallel_execution_without_config_load() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        execution: ExecutionConfig {
            parallel: 0,
            ..ExecutionConfig::default()
        },
        ..Config::default()
    };

    let error = sqlite_fleet::migrate(&config, true, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("execution.parallel は1以上が必要です"));
}

#[test]
fn status_and_check_reject_zero_parallel_execution_without_config_load() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        execution: ExecutionConfig {
            parallel: 0,
            ..ExecutionConfig::default()
        },
        ..Config::default()
    };

    let status_error = status_report(&config).unwrap_err().to_string();
    assert!(status_error.contains("execution.parallel は1以上が必要です"));

    let check_error = check(&config).unwrap_err().to_string();
    assert!(check_error.contains("execution.parallel は1以上が必要です"));
}

#[test]
fn build_plan_rejects_zero_parallel_execution_without_config_load() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        execution: ExecutionConfig {
            parallel: 0,
            ..ExecutionConfig::default()
        },
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: db_path,
        exists: true,
        readable: true,
    };

    let plan = build_database_plan(&config, &database, &[]);
    assert!(plan
        .error
        .unwrap()
        .contains("execution.parallel は1以上が必要です"));
}

#[test]
fn config_allows_unicode_group_names() {
    let dir = tempdir().unwrap();
    let mut config = Config {
        base_dir: dir.path().to_path_buf(),
        migration_groups: HashMap::from([(
            "顧客管理".to_string(),
            MigrationGroupConfig::legacy_dir("migrations/customer".to_string()),
        )]),
        database_migration_groups: HashMap::from([(
            "tenant-a".to_string(),
            vec!["顧客管理".to_string()],
        )]),
        db_groups: HashMap::from([("本番".to_string(), vec!["tenant-a".to_string()])]),
        ..Config::default()
    };

    config.validate().unwrap();

    config.migration_groups.insert(
        "顧客 管理".to_string(),
        MigrationGroupConfig::legacy_dir("migrations/customer-space".to_string()),
    );
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("空白なしの非空文字列"));

    config.migration_groups.clear();
    config.migration_groups.insert(
        ".".to_string(),
        MigrationGroupConfig::legacy_dir("migrations/dot".to_string()),
    );
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("特殊なパス成分"));

    config.migration_groups.clear();
    config.migration_groups.insert(
        "..".to_string(),
        MigrationGroupConfig::legacy_dir("migrations/dotdot".to_string()),
    );
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("特殊なパス成分"));
}

#[test]
fn runtime_apis_reject_blank_config_fields_without_config_load() {
    let dir = tempdir().unwrap();
    let glob_config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some(" ".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };
    let glob_error = discover_databases(&glob_config).unwrap_err().to_string();
    assert!(glob_error.contains("databases.path_glob が必要です"));

    let query_config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some(" ".to_string()),
            query: Some("SELECT id FROM tenants".to_string()),
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };
    let source_error = discover_databases(&query_config).unwrap_err().to_string();
    assert!(source_error.contains("databases.source が必要です"));

    let migrations_config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: " ".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };
    let migrations_error = load_migrations(&migrations_config).unwrap_err().to_string();
    assert!(migrations_error.contains("migrations.dir は空にできません"));

    let report_config = Config {
        base_dir: dir.path().to_path_buf(),
        report: ReportConfig {
            format: "json".to_string(),
            path: Some(" ".to_string()),
        },
        ..Config::default()
    };
    let report_error = write_report_json(&report_config, &serde_json::json!({"ok": true}))
        .unwrap_err()
        .to_string();
    assert!(report_error.contains("report.path は空にできません"));
}

#[test]
fn query_discovery_rejects_blob_id_and_path_columns() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id BLOB, db_path BLOB)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES (?1, ?2)",
        rusqlite::params![&[0xff_u8, 0x00][..], &[0x66_u8, 0x6f, 0x6f][..]],
    )
    .unwrap();

    let id_config = Config {
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
    let id_error = discover_databases(&id_config).unwrap_err().to_string();
    assert!(id_error.contains("id_column を取得できません"));

    let path_config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT 'tenant' AS id, db_path FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: None,
        },
        ..Config::default()
    };
    let path_error = discover_databases(&path_config).unwrap_err().to_string();
    assert!(path_error.contains("path_column を取得できません"));
}

#[test]
fn render_path_template_rejects_empty_id() {
    let error = sqlite_fleet::render_path_template("data/{id}.db", " ")
        .unwrap_err()
        .to_string();
    assert!(error.contains("path_template に埋め込むIDとして不正です"));
}

#[cfg(unix)]
#[test]
fn load_migrations_rejects_symlinked_file_outside_base_dir() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(&migrations_dir).unwrap();
    let outside_migration = outside.path().join("001_outside.sql");
    fs::write(&outside_migration, "CREATE TABLE outside_items(id);").unwrap();
    symlink(&outside_migration, migrations_dir.join("001_outside.sql")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };

    let error = sqlite_fleet::load_migrations(&config)
        .unwrap_err()
        .to_string();
    assert!(error.contains("migration ファイル"));
}

#[test]
fn load_migrations_rejects_uppercase_sql_extension() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::write(
        migrations_dir.join("001_create_items.SQL"),
        "CREATE TABLE items(id);",
    )
    .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };

    let error = sqlite_fleet::load_migrations(&config)
        .unwrap_err()
        .to_string();
    assert!(error.contains("小文字 .sql"));
}

#[cfg(unix)]
#[test]
fn glob_discovery_reports_broken_symlink_metadata_error() {
    use std::os::unix::fs::symlink;

    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    symlink(data_dir.join("missing.db"), data_dir.join("tenant.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = sqlite_fleet::discover_databases(&config)
        .unwrap_err()
        .to_string();
    assert!(error.contains("DBメタデータを取得できません"));
}

#[test]
fn parse_migration_rejects_signed_version() {
    let dir = tempdir().unwrap();
    let migration_path = dir.path().join("+1_create_items.sql");
    fs::write(&migration_path, "CREATE TABLE items(id);").unwrap();

    let error = sqlite_fleet::parse_migration_file(&migration_path)
        .unwrap_err()
        .to_string();
    assert!(error.contains("ASCII数字のみ"));
}

#[test]
fn parse_migration_rejects_unsafe_name_characters() {
    let dir = tempdir().unwrap();
    let migration_path = dir.path().join("001_create items.sql");
    fs::write(&migration_path, "CREATE TABLE items(id);").unwrap();

    let error = sqlite_fleet::parse_migration_file(&migration_path)
        .unwrap_err()
        .to_string();
    assert!(error.contains("migration name は英数字、_、- のみ"));
}

#[test]
fn parse_migration_rejects_non_sql_extension() {
    let dir = tempdir().unwrap();
    let migration_path = dir.path().join("001_create_items.txt");
    fs::write(&migration_path, "CREATE TABLE items(id);").unwrap();

    let error = sqlite_fleet::parse_migration_file(&migration_path)
        .unwrap_err()
        .to_string();
    assert!(error.contains("小文字 .sql"));
}

#[test]
fn load_migrations_rejects_sql_directory_entry() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(migrations_dir.join("001_directory.sql")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };

    let error = sqlite_fleet::load_migrations(&config)
        .unwrap_err()
        .to_string();
    assert!(error.contains("通常ファイルである必要があります"));
}

#[test]
fn direct_database_apis_reject_duplicate_migration_versions() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: db_path,
        exists: true,
        readable: true,
    };
    let migrations = vec![
        migration_summary_input("001", 1, "one", dir.path()),
        migration_summary_input("1", 1, "duplicate", dir.path()),
    ];

    let plan = build_database_plan(&config, &database, &migrations);
    assert!(plan.error.unwrap().contains("数値として重複"));

    let migrate_result = migrate_database(&config, &database, &migrations, true);
    assert!(!migrate_result.success);
    assert!(migrate_result.error.unwrap().contains("数値として重複"));

    let check_result = check_database(&config, &database, &migrations);
    assert!(!check_result.success);
    assert!(check_result.error.unwrap().contains("数値として重複"));
}

#[test]
fn direct_database_apis_reject_migration_checksum_mismatch() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: db_path,
        exists: true,
        readable: true,
    };
    let mut migration = migration_summary_input("001", 1, "one", dir.path());
    migration.checksum = "wrong-checksum".to_string();
    let migrations = vec![migration];

    let plan = build_database_plan(&config, &database, &migrations);
    assert!(plan
        .error
        .unwrap()
        .contains("checksum がSQL内容と一致しません"));

    let migrate_result = migrate_database(&config, &database, &migrations, true);
    assert!(!migrate_result.success);
    assert!(migrate_result
        .error
        .unwrap()
        .contains("checksum がSQL内容と一致しません"));

    let check_result = check_database(&config, &database, &migrations);
    assert!(!check_result.success);
    assert!(check_result
        .error
        .unwrap()
        .contains("checksum がSQL内容と一致しません"));
}

#[test]
fn direct_database_apis_reject_migration_path_outside_base_dir() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: db_path,
        exists: true,
        readable: true,
    };
    let mut migration = migration_summary_input("001", 1, "one", dir.path());
    migration.path = outside.path().join("001_one.sql");
    let migrations = vec![migration];

    let plan = build_database_plan(&config, &database, &migrations);
    assert!(plan.error.unwrap().contains("migration ファイル"));

    let migrate_result = migrate_database(&config, &database, &migrations, true);
    assert!(!migrate_result.success);
    assert!(migrate_result.error.unwrap().contains("migration ファイル"));

    let check_result = check_database(&config, &database, &migrations);
    assert!(!check_result.success);
    assert!(check_result.error.unwrap().contains("migration ファイル"));
}

#[test]
fn runtime_apis_reject_invalid_migrations_table_name() {
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
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            table: "1_invalid".to_string(),
        },
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: db_path,
        exists: true,
        readable: true,
    };
    let migrations = vec![migration_summary_input("001", 1, "one", dir.path())];

    assert!(status_report(&config)
        .unwrap_err()
        .to_string()
        .contains("SQLite識別子"));
    assert!(migrate(&config, true, None)
        .unwrap_err()
        .to_string()
        .contains("SQLite識別子"));
    assert!(check(&config)
        .unwrap_err()
        .to_string()
        .contains("SQLite識別子"));

    let plan = build_database_plan(&config, &database, &migrations);
    assert!(plan.error.unwrap().contains("SQLite識別子"));

    let migrate_result = migrate_database(&config, &database, &migrations, true);
    assert!(!migrate_result.success);
    assert!(migrate_result.error.unwrap().contains("SQLite識別子"));

    let check_result = check_database(&config, &database, &migrations);
    assert!(!check_result.success);
    assert!(check_result.error.unwrap().contains("SQLite識別子"));
}

fn migration_summary_input(
    version: &str,
    version_number: u64,
    name: &str,
    base_dir: &std::path::Path,
) -> Migration {
    let sql = format!("CREATE TABLE {name}(id);");
    Migration {
        group: "default".to_string(),
        version: version.to_string(),
        version_number,
        name: name.to_string(),
        checksum: sqlite_fleet::checksum_sql(&sql),
        path: base_dir.join(format!("{version}_{name}.sql")),
        sql,
    }
}
