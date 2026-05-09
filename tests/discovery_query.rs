use rusqlite::Connection;
use sqlite_fleet::{discover_databases, Config, DatabasesConfig};
use std::fs;
use tempfile::tempdir;

#[test]
fn config_load_rejects_non_select_discovery_query() {
    let cases = [
        ("DELETE FROM tenants RETURNING id", "DELETE"),
        ("PRAGMA table_info(tenants)", "PRAGMA"),
        ("SELECT id FROM tenants; DELETE FROM tenants", "1文のみ"),
        (
            "WITH victims AS (SELECT rowid FROM tenants) DELETE FROM tenants RETURNING id",
            "本体は SELECT",
        ),
        ("/* unclosed", "コメントが閉じていません"),
    ];

    for (query, expected) in cases {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        fs::write(
            &config_path,
            format!(
                r#"
[databases]
discovery = "query"
source = "shared.db"
query = "{query}"
path_template = "data/{{id}}.db"
"#
            ),
        )
        .unwrap();

        let error = Config::load(&config_path).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn query_discovery_allows_select_with_leading_comments() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('tenant')", [])
        .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("\u{feff}/* discovery */\n-- tenants\nSELECT id FROM tenants".to_string()),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let databases = discover_databases(&config).unwrap();
    assert_eq!(databases[0].id, "tenant");
}

#[test]
fn query_discovery_allows_trailing_semicolon_and_comment() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('tenant')", [])
        .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT ';' AS marker, id FROM tenants; -- trailing".to_string()),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let databases = discover_databases(&config).unwrap();
    assert_eq!(databases[0].id, "tenant");
}

#[test]
fn query_discovery_applies_configured_busy_timeout_to_source_connection() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('tenant')", [])
        .unwrap();
    let lock = Connection::open(&source).unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        execution: sqlite_fleet::ExecutionConfig {
            lock_timeout_ms: 25,
            ..sqlite_fleet::ExecutionConfig::default()
        },
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT id FROM tenants".to_string()),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let started = std::time::Instant::now();
    let error = format!("{:#}", discover_databases(&config).unwrap_err());

    assert!(started.elapsed() >= std::time::Duration::from_millis(20));
    assert!(
        error.contains("database is locked") || error.contains("database table is locked"),
        "{error}"
    );
}

#[test]
fn query_discovery_allows_with_select_query() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('tenant')", [])
        .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some(
                "WITH listed AS (SELECT id FROM tenants) SELECT id FROM listed".to_string(),
            ),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let databases = discover_databases(&config).unwrap();
    assert_eq!(databases[0].id, "tenant");
}

#[test]
fn query_discovery_allows_with_materialization_hints() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('tenant')", [])
        .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some(
                "WITH listed AS NOT MATERIALIZED (SELECT id FROM tenants), \
                 echoed AS MATERIALIZED (SELECT id FROM listed) \
                 SELECT id FROM echoed"
                    .to_string(),
            ),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let databases = discover_databases(&config).unwrap();
    assert_eq!(databases[0].id, "tenant");
}

#[test]
fn query_discovery_allows_recursive_with_column_list() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('tenant')", [])
        .unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some(
                "WITH RECURSIVE listed(id) AS (SELECT id FROM tenants) SELECT id FROM listed"
                    .to_string(),
            ),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let databases = discover_databases(&config).unwrap();
    assert_eq!(databases[0].id, "tenant");
}

#[test]
fn query_discovery_rejects_non_select_query_before_opening_source() {
    let dir = tempdir().unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("missing.db".to_string()),
            query: Some("DELETE FROM tenants RETURNING id".to_string()),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("DELETE"), "{error}");
}

#[test]
fn query_discovery_rejects_non_readonly_with_query() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES ('tenant')", [])
        .unwrap();
    drop(conn);

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some(
                "WITH victims AS (SELECT rowid FROM tenants) DELETE FROM tenants RETURNING id"
                    .to_string(),
            ),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("本体は SELECT"), "{error}");
    let conn = Connection::open(&source).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM tenants", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}
