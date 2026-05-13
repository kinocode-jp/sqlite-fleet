use assert_cmd::Command;
use rusqlite::Connection;
use sqlite_fleet::{
    checksum_sql, discover_databases, load_migrations, migrate, parse_migration_file,
    render_path_template, Config, DatabasesConfig, MigrationsConfig,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn renders_plain_id_template() {
    let rendered = render_path_template("./data/{id}.db", "42").unwrap();
    assert_eq!(rendered, "./data/42.db");
}

#[test]
fn renders_split_template() {
    let rendered = render_path_template("./data/{id:08:split2}.db", "123").unwrap();
    assert_eq!(rendered, "./data/00/00/00000123.db");
}

#[test]
fn renders_split_template_with_non_default_split_width() {
    let rendered = render_path_template("./data/{id:06:split3}.db", "1").unwrap();
    assert_eq!(rendered, "./data/000/001/000001.db");
}

#[test]
fn init_creates_nested_config_parent_directory() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("nested").join("sqlite-fleet.toml");

    Command::cargo_bin("sqlite-fleet")
        .unwrap()
        .arg("--config")
        .arg(&config_path)
        .arg("init")
        .assert()
        .success();

    assert!(config_path.exists());
    assert!(dir.path().join("nested").join("migrations").is_dir());

    let config = Config::load(&config_path).unwrap();
    let migrations = load_migrations(&config).unwrap();
    assert!(migrations.is_empty());
}

#[test]
fn config_rejects_blank_required_discovery_values() {
    let dir = tempdir().unwrap();
    let glob_config = dir.path().join("glob.toml");
    fs::write(
        &glob_config,
        r#"
[databases]
discovery = "glob"
path_glob = ""
"#,
    )
    .unwrap();
    let error = Config::load(&glob_config).unwrap_err().to_string();
    assert!(error.contains("databases.path_glob が必要です"));

    let query_config = dir.path().join("query.toml");
    fs::write(
        &query_config,
        r#"
[databases]
discovery = "query"
source = " "
query = "SELECT id FROM tenants"
path_template = "tenants/{id}.db"
"#,
    )
    .unwrap();
    let error = Config::load(&query_config).unwrap_err().to_string();
    assert!(error.contains("databases.source が必要です"));
}

#[test]
fn parses_migration_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("001_create_users.sql");
    fs::write(&path, "CREATE TABLE users(id INTEGER);").unwrap();
    let migration = parse_migration_file(&path).unwrap();
    assert_eq!(migration.version, "001");
    assert_eq!(migration.version_number, 1);
    assert_eq!(migration.name, "create_users");
    assert_eq!(
        migration.checksum,
        checksum_sql("CREATE TABLE users(id INTEGER);")
    );
}

#[test]
fn parses_suffix_version_migration_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("create_users_001.sql");
    fs::write(&path, "CREATE TABLE users(id INTEGER);").unwrap();
    let migration = parse_migration_file(&path).unwrap();
    assert_eq!(migration.version, "001");
    assert_eq!(migration.version_number, 1);
    assert_eq!(migration.name, "create_users");
}

#[test]
fn rejects_non_numeric_migration_version() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("abc_create_users.sql");
    fs::write(&path, "CREATE TABLE users(id INTEGER);").unwrap();
    let error = parse_migration_file(&path).unwrap_err().to_string();
    assert!(error.contains("migration version は数値である必要があります"));
}

#[test]
fn sorts_migrations_by_numeric_version() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::write(migrations_dir.join("10_ten.sql"), "CREATE TABLE ten(id);").unwrap();
    fs::write(migrations_dir.join("2_two.sql"), "CREATE TABLE two(id);").unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        migrations: MigrationsConfig {
            dir: "migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };

    let migrations = load_migrations(&config).unwrap();
    assert_eq!(migrations[0].version, "2");
    assert_eq!(migrations[1].version, "10");
}

#[test]
fn allows_same_numeric_version_with_different_filenames() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::write(migrations_dir.join("001_one.sql"), "CREATE TABLE one(id);").unwrap();
    fs::write(
        migrations_dir.join("1_duplicate.sql"),
        "CREATE TABLE duplicate(id);",
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

    let migrations = load_migrations(&config).unwrap();
    assert_eq!(migrations.len(), 2);
    assert_eq!(migrations[0].filename, "001_one.sql");
    assert_eq!(migrations[1].filename, "1_duplicate.sql");
}

#[test]
fn sorts_same_version_and_name_by_filename() {
    let dir = tempdir().unwrap();
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir_all(&migrations_dir).unwrap();
    fs::write(
        migrations_dir.join("add_user_001.sql"),
        "CREATE TABLE add_user_suffix(id);",
    )
    .unwrap();
    fs::write(
        migrations_dir.join("001_add_user.sql"),
        "CREATE TABLE add_user_prefix(id);",
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

    let migrations = load_migrations(&config).unwrap();
    let filenames = migrations
        .iter()
        .map(|migration| migration.filename.as_str())
        .collect::<Vec<_>>();
    assert_eq!(filenames, vec!["001_add_user.sql", "add_user_001.sql"]);
}

#[test]
fn discovers_query_with_path_template() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id INTEGER PRIMARY KEY)", [])
        .unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES (7)", [])
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
            path_template: Some("tenants/{id:08:split2}.db".to_string()),
        },
        ..Config::default()
    };
    let databases = discover_databases(&config).unwrap();
    assert_eq!(databases.len(), 1);
    assert_eq!(databases[0].id, "7");
    assert!(databases[0].path.ends_with("tenants/00/00/00000007.db"));
}

#[test]
fn glob_discovery_rejects_duplicate_ids() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir_all(data_dir.join("a")).unwrap();
    fs::create_dir_all(data_dir.join("b")).unwrap();
    Connection::open(data_dir.join("a").join("tenant.db")).unwrap();
    Connection::open(data_dir.join("b").join("tenant.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/**/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DB ID が重複しています"));
}

#[test]
fn query_discovery_rejects_duplicate_ids() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', 'data/a.db')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', 'data/b.db')",
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
    assert!(error.contains("DB ID が重複しています"));
}

#[test]
fn query_discovery_rejects_duplicate_paths() {
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
        "INSERT INTO tenants(id, db_path) VALUES ('tenant_b', 'data/tenant.db')",
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
fn migrate_rejects_unknown_database_filter() {
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

    let error = migrate(&config, false, Some("missing"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("指定されたDBが見つかりません"));
}

#[test]
fn query_discovery_rejects_empty_id() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES (NULL)", [])
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
            path_template: Some("tenants/{id:08:split2}.db".to_string()),
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("id_column を取得できません"));
}

#[test]
fn query_discovery_rejects_empty_path_column() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', NULL)",
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
    assert!(error.contains("path_column を取得できません"));
}

#[test]
fn query_discovery_does_not_create_missing_source_db() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("missing-shared.db");
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("missing-shared.db".to_string()),
            query: Some("SELECT id FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: None,
            path_template: Some("tenants/{id:08:split2}.db".to_string()),
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("discovery source を開けません"));
    assert!(!source.exists());
}
