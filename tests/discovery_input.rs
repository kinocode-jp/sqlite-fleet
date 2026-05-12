use rusqlite::Connection;
use sqlite_fleet::{discover_databases, Config, DatabasesConfig};
use std::fs;
use tempfile::tempdir;

#[test]
fn glob_discovery_rejects_parent_components_in_path_glob() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/../*.db"
"#,
    )
    .unwrap();

    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_glob に親ディレクトリ成分"),
        "{error}"
    );

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some("data/../*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };
    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_glob に親ディレクトリ成分"),
        "{error}"
    );
}

#[test]
fn query_discovery_rejects_parent_components_in_source() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "data/../shared.db"
query = "SELECT id FROM tenants"
path_template = "data/{id}.db"
"#,
    )
    .unwrap();

    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.source に親ディレクトリ成分"),
        "{error}"
    );

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("data/../shared.db".to_string()),
            query: Some("SELECT id FROM tenants".to_string()),
            id_column: Some("id".to_string()),
            path_column: None,
            path_template: Some("data/{id}.db".to_string()),
        },
        ..Config::default()
    };
    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("databases.source に親ディレクトリ成分"),
        "{error}"
    );
}

#[test]
fn configured_database_paths_reject_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = " data/*.db"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_glob の前後に空白"),
        "{error}"
    );

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = " shared.db"
query = "SELECT id FROM tenants"
path_template = "data/{id}.db"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("databases.source の前後に空白"), "{error}");

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id FROM tenants"
path_template = "data/{id}.db "
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_template の前後に空白"),
        "{error}"
    );

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob".to_string(),
            path_glob: Some(" data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };
    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_glob の前後に空白"),
        "{error}"
    );
}

#[test]
fn discovery_mode_rejects_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = " glob"
path_glob = "data/*.db"
"#,
    )
    .unwrap();

    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.discovery の前後に空白"),
        "{error}"
    );

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "glob ".to_string(),
            path_glob: Some("data/*.db".to_string()),
            ..DatabasesConfig::default()
        },
        ..Config::default()
    };
    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("databases.discovery の前後に空白"),
        "{error}"
    );
}

#[test]
fn config_load_rejects_unknown_fields() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");

    for (field, toml) in [
        (
            "top_level_typo",
            r#"
top_level_typo = true

[databases]
discovery = "glob"
path_glob = "data/*.db"
"#,
        ),
        (
            "path_globb",
            r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"
path_globb = "other/*.db"
"#,
        ),
        (
            "table_name",
            r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migrations]
dir = "migrations"
table_name = "history"
"#,
        ),
        (
            "parallelism",
            r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[execution]
parallel = 1
parallelism = 2
"#,
        ),
        (
            "output",
            r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[report]
format = "json"
output = "report.json"
"#,
        ),
        (
            "display_name",
            r#"
[project]
name = "sqlite-fleet"
display_name = "SQLite Fleet"

[databases]
discovery = "glob"
path_glob = "data/*.db"
"#,
        ),
    ] {
        fs::write(&config_path, toml).unwrap();
        let error = Config::load(&config_path).unwrap_err().to_string();
        assert!(error.contains(field), "{field}: {error}");
    }
}

#[test]
fn config_load_rejects_unknown_migration_group_table_fields() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migration_groups.core]
dr = "migrations/core"
"#,
    )
    .unwrap();

    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("TOML解析に失敗しました"), "{error}");
}

#[test]
fn config_load_rejects_empty_migration_group_table() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "glob"
path_glob = "data/*.db"

[migration_groups.core]
"#,
    )
    .unwrap();

    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(error.contains("dir または migrations が必要"), "{error}");
}

#[test]
fn query_discovery_rejects_id_with_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT)", []).unwrap();
    conn.execute("INSERT INTO tenants(id) VALUES (' tenant ')", [])
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
            path_template: Some("data/{id}.db".to_string()),
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("id_column を取得できません"), "{error}");
}

#[test]
fn query_discovery_rejects_path_with_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', ' data/tenant.db ')",
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
    assert!(error.contains("path_column を取得できません"), "{error}");
}

#[test]
fn query_discovery_rejects_invalid_utf8_text_columns() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES (CAST(x'ff' AS TEXT), 'data/tenant.db')",
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
    assert!(error.contains("id_column を取得できません"), "{error}");

    conn.execute("DELETE FROM tenants", []).unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', CAST(x'ff' AS TEXT))",
        [],
    )
    .unwrap();

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("path_column を取得できません"), "{error}");
}

#[test]
fn query_discovery_rejects_column_config_with_surrounding_whitespace() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id FROM tenants"
id_column = " id "
path_template = "data/{id}.db"
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.id_column の前後に空白"),
        "{error}"
    );

    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id, db_path FROM tenants"
path_column = " db_path "
"#,
    )
    .unwrap();
    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_column の前後に空白"),
        "{error}"
    );
}

#[test]
fn config_load_rejects_parent_components_in_path_template() {
    let dir = tempdir().unwrap();
    let config_path = dir.path().join("sqlite-fleet.toml");
    fs::write(
        &config_path,
        r#"
[databases]
discovery = "query"
source = "shared.db"
query = "SELECT id FROM tenants"
id_column = "id"
path_template = "data/{id}/../shared.db"
"#,
    )
    .unwrap();

    let error = Config::load(&config_path).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_template に親ディレクトリ成分"),
        "{error}"
    );
}

#[test]
fn runtime_query_discovery_rejects_column_config_with_surrounding_whitespace() {
    let dir = tempdir().unwrap();
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
            id_column: Some(" id ".to_string()),
            path_column: Some("db_path".to_string()),
            path_template: None,
        },
        ..Config::default()
    };
    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("databases.id_column の前後に空白"),
        "{error}"
    );

    let config = Config {
        databases: DatabasesConfig {
            id_column: Some("id".to_string()),
            path_column: Some(" db_path ".to_string()),
            ..config.databases
        },
        ..config
    };
    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_column の前後に空白"),
        "{error}"
    );
}

#[test]
fn query_discovery_rejects_parent_components_in_db_paths() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    let conn = Connection::open(&source).unwrap();
    conn.execute("CREATE TABLE tenants(id TEXT, db_path TEXT)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO tenants(id, db_path) VALUES ('tenant', 'data/tenant/../shared.db')",
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
    assert!(
        error.contains("path_column に親ディレクトリ成分"),
        "{error}"
    );

    let config = Config {
        databases: DatabasesConfig {
            path_column: None,
            path_template: Some("data/{id}/../shared.db".to_string()),
            ..config.databases
        },
        ..config
    };
    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(
        error.contains("databases.path_template に親ディレクトリ成分"),
        "{error}"
    );
}

#[test]
fn runtime_query_discovery_rejects_absolute_path_template_outside_base() {
    let dir = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let source = dir.path().join("shared.db");
    Connection::open(&source).unwrap();
    let config = Config {
        base_dir: dir.path().to_path_buf(),
        databases: DatabasesConfig {
            discovery: "query".to_string(),
            path_glob: None,
            source: Some("shared.db".to_string()),
            query: Some("SELECT 'tenant' AS id".to_string()),
            id_column: Some("id".to_string()),
            path_column: None,
            path_template: Some(outside.path().join("{id}.db").to_string_lossy().to_string()),
        },
        ..Config::default()
    };

    let error = discover_databases(&config).unwrap_err().to_string();
    assert!(error.contains("databases.path_template"), "{error}");
}
