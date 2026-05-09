use rusqlite::Connection;
use sqlite_fleet::{
    build_database_plan, build_plan, check_database, discover_databases, migrate, migrate_database,
    render_path_template, Config, Database, DatabasesConfig, MigrationsConfig,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

#[test]
fn database_paths_must_be_regular_files() {
    let dir = tempdir().unwrap();
    let db_dir = dir.path().join("tenant.db");
    fs::create_dir(&db_dir).unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', 'tenant.db')",
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
            path_column: Some("db_path".to_string()),
            path_template: None,
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let discovery_error = discover_databases(&config).unwrap_err().to_string();
    assert!(discovery_error.contains("通常ファイル"));

    let database = Database {
        id: "tenant".to_string(),
        path: db_dir,
        exists: true,
        readable: true,
    };
    let plan = build_database_plan(&config, &database, &[]);
    assert!(plan.error.unwrap().contains("通常ファイル"));
    let migrate_result = migrate_database(&config, &database, &[], true);
    assert!(!migrate_result.success);
    assert!(migrate_result.error.unwrap().contains("通常ファイル"));
    let check_result = check_database(&config, &database, &[]);
    assert!(!check_result.success);
    assert!(check_result.error.unwrap().contains("通常ファイル"));
}

#[test]
fn direct_database_apis_reject_invalid_database_id() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    Connection::open(&db_path).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let database = Database {
        id: " tenant ".to_string(),
        path: db_path,
        exists: true,
        readable: true,
    };

    let plan = build_database_plan(&config, &database, &[]);
    assert!(plan.error.unwrap().contains("DB ID として不正です"));

    let migrate_result = migrate_database(&config, &database, &[], true);
    assert!(!migrate_result.success);
    assert!(migrate_result
        .error
        .unwrap()
        .contains("DB ID として不正です"));

    let check_result = check_database(&config, &database, &[]);
    assert!(!check_result.success);
    assert!(check_result.error.unwrap().contains("DB ID として不正です"));
}

#[test]
fn direct_build_plan_rejects_duplicate_database_set() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let databases = vec![
        Database {
            id: "tenant".to_string(),
            path: dir.path().join("a.db"),
            exists: false,
            readable: false,
        },
        Database {
            id: "tenant".to_string(),
            path: dir.path().join("b.db"),
            exists: false,
            readable: false,
        },
    ];

    let plans = build_plan(&config, &databases, &[]);
    assert_eq!(plans.len(), 2);
    assert!(plans.iter().all(|plan| plan
        .error
        .as_deref()
        .is_some_and(|error| error.contains("DB ID が重複しています"))));
}

#[test]
fn direct_build_plan_rejects_duplicate_database_paths() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let db_path = dir.path().join("tenant.db");
    let databases = vec![
        Database {
            id: "tenant_a".to_string(),
            path: db_path.clone(),
            exists: false,
            readable: false,
        },
        Database {
            id: "tenant_b".to_string(),
            path: db_path,
            exists: false,
            readable: false,
        },
    ];

    let plans = build_plan(&config, &databases, &[]);
    assert_eq!(plans.len(), 2);
    assert!(plans.iter().all(|plan| plan
        .error
        .as_deref()
        .is_some_and(|error| error.contains("DBパスが重複しています"))));
}

#[test]
fn glob_discovery_rejects_non_file_matches() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir(&data_dir).unwrap();
    fs::create_dir(data_dir.join("tenant.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("通常ファイル"), "{error}");
}

#[test]
fn glob_discovery_rejects_ids_with_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().join("data");
    fs::create_dir(&data_dir).unwrap();
    Connection::open(data_dir.join(" tenant.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DB ID として不正です"), "{error}");
}

#[test]
fn query_discovery_source_must_be_regular_file() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("shared.db")).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT 'tenant' AS id".to_string()),
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("通常ファイル"), "{error}");
}

#[test]
fn database_operation_results_refresh_stale_database_state() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("tenant.db");
    Connection::open(&db_path).unwrap();
    fs::remove_file(&db_path).unwrap();
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
    assert!(!plan.database.exists);
    assert!(!plan.database.readable);
    assert!(plan.error.unwrap().contains("DBファイルが存在しません"));

    let migrate_result = migrate_database(&config, &database, &[], true);
    assert!(!migrate_result.database.exists);
    assert!(!migrate_result.database.readable);
    assert!(!migrate_result.success);
    assert!(migrate_result
        .error
        .unwrap()
        .contains("DBファイルが存在しません"));

    let check_result = check_database(&config, &database, &[]);
    assert!(!check_result.database.exists);
    assert!(!check_result.database.readable);
    assert!(!check_result.success);
    assert!(check_result
        .error
        .unwrap()
        .contains("DBファイルが存在しません"));
}

#[test]
fn database_state_is_not_refreshed_before_base_dir_validation() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_db = outside.path().join("tenant.db");
    Connection::open(&outside_db).unwrap();
    fs::write(outside.path().join("tenant.db-wal"), "outside wal").unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let database = Database {
        id: "tenant".to_string(),
        path: outside_db,
        exists: false,
        readable: false,
    };

    let plan = build_database_plan(&config, &database, &[]);
    assert!(!plan.database.exists);
    assert!(!plan.database.readable);
    assert!(plan.error.unwrap().contains("DBパス"));

    let migrate_result = migrate_database(&config, &database, &[], true);
    assert!(!migrate_result.database.exists);
    assert!(!migrate_result.database.readable);
    assert!(!migrate_result.success);
    assert!(migrate_result.error.unwrap().contains("DBパス"));

    let check_result = check_database(&config, &database, &[]);
    assert!(!check_result.database.exists);
    assert!(!check_result.database.readable);
    assert!(!check_result.success);
    assert_eq!(check_result.wal_bytes, None);
    assert!(check_result.error.unwrap().contains("DBパス"));
}

#[test]
fn migrate_database_selector_rejects_outside_path_without_canonicalizing_it() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_db = outside.path().join("tenant.db");
    Connection::open(&outside_db).unwrap();
    let data_dir = dir.path().join("data");
    let migrations_dir = dir.path().join("migrations");
    fs::create_dir(&data_dir).unwrap();
    fs::create_dir(&migrations_dir).unwrap();
    Connection::open(data_dir.join("tenant.db")).unwrap();
    fs::write(
        migrations_dir.join("001_create_items.sql"),
        "CREATE TABLE items(id INTEGER PRIMARY KEY);",
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

    let error = migrate(&config, true, Some(&outside_db.to_string_lossy()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("指定されたDBが見つかりません"));
}

#[test]
fn query_discovery_rejects_path_like_ids_when_path_column_is_used() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('../outside', 'data/tenant.db')",
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
    assert!(error.contains("DB ID として不正です"), "{error}");
}

#[test]
fn query_discovery_rejects_blank_path_template_even_with_path_column() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id, db_path FROM tenants"
path_column = "db_path"
path_template = " "
"#,
    )
    .unwrap();
    let load_error = Config::load(&config_path).unwrap_err().to_string();
    assert!(load_error.contains("databases.path_template は空にできません"));

    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id, db_path FROM tenants".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: Some(" ".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let runtime_error = discover_databases(&config).unwrap_err().to_string();
    assert!(runtime_error.contains("databases.path_template は空にできません"));
}

#[test]
fn config_load_rejects_invalid_path_template_syntax() {
    let cases = [
        ("data/{tenant}.db", "未対応の path_template 置換式です"),
        ("data/{id:0:split2}.db", "ゼロ埋め幅は1以上"),
        ("data/{id:01:split2}.db", "ゼロ埋め幅はsplit指定以上"),
        ("data/{id:08:split0}.db", "split指定は1以上が必要です"),
        ("data/{id:x:split2}.db", "ゼロ埋め幅が不正です"),
        ("data/{id:+8:split2}.db", "ゼロ埋め幅が不正です"),
        ("data/{id:1025:split2}.db", "ゼロ埋め幅は1024以下"),
        ("data/{id:08:split}.db", "split指定が不正です"),
        ("data/{id:08:splitsplit2}.db", "split指定が不正です"),
        ("data/{id:08:split1025}.db", "split指定は1024以下"),
        ("data/{id.db", "置換式が閉じていません"),
        ("data/{id}.db}", "置換式が開いていません"),
        ("data/static.db", "path_template には {id}"),
    ];

    for (template, expected) in cases {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id FROM tenants"
path_template = "{template}"
"#
            ),
        )
        .unwrap();

        let error = Config::load(&config_path).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn query_discovery_rejects_invalid_path_template_before_opening_source() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("missing.db".to_string()),
            query: Some("SELECT id FROM tenants".to_string()),
            path_column: None,
            path_template: Some("data/{tenant}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("未対応の path_template 置換式です"),
        "{error}"
    );
}

#[test]
fn render_path_template_rejects_unmatched_closing_brace() {
    let error = render_path_template("data/{id}.db}", "tenant")
        .unwrap_err()
        .to_string();
    assert!(error.contains("置換式が開いていません"), "{error}");
}

#[test]
fn render_path_template_rejects_ids_with_surrounding_whitespace() {
    let error = render_path_template("data/{id}.db", " tenant ")
        .unwrap_err()
        .to_string();
    assert!(error.contains("path_template に埋め込むIDとして不正です"));
}

#[test]
fn render_path_template_requires_id_placeholder() {
    let error = render_path_template("data/static.db", "tenant")
        .unwrap_err()
        .to_string();
    assert!(error.contains("path_template には {id}"), "{error}");
}

#[test]
fn render_path_template_rejects_invalid_split_expression() {
    let error = render_path_template("data/{id:08:splitsplit2}.db", "tenant")
        .unwrap_err()
        .to_string();
    assert!(error.contains("split指定が不正です"), "{error}");
}

#[test]
fn render_path_template_splits_non_ascii_ids_by_character() {
    let rendered = render_path_template("data/{id:04:split2}.db", "あいう").unwrap();
    assert_eq!(rendered, "data/0あ/いう/0あいう.db");
}

#[test]
fn config_load_canonicalizes_relative_base_dir() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::path::PathBuf::from("target").join(format!("config-base-{unique}"));
    fs::create_dir_all(&dir).unwrap();
    let config_path = dir.join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "migrations"
"#,
    )
    .unwrap();

    let config = Config::load(&config_path).unwrap();
    assert!(config.base_dir.is_absolute());
    assert_eq!(config.resolve_path("data"), config.base_dir.join("data"));
}
