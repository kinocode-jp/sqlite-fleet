use rusqlite::Connection;
use sqlite_fleet::{
    checksum_sql, migrate_database, parse_migration_file, Config, Database, Migration,
};
use std::fs;
use tempfile::tempdir;

#[test]
fn migration_sql_rejects_attach_and_detach_database() {
    let cases = [
        ("001_attach.sql", "ATTACH DATABASE 'other.db' AS other;"),
        ("002_detach.sql", "DETACH DATABASE other;"),
        (
            "003_bom_attach.sql",
            "\u{feff}ATTACH DATABASE 'other.db' AS other;",
        ),
    ];

    for (file_name, sql) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join(file_name);
        fs::write(&path, sql).unwrap();

        let error = parse_migration_file(&path).unwrap_err().to_string();
        assert!(error.contains("外部DB接続文"), "{error}");
    }
}

#[test]
fn migration_sql_allows_attach_words_in_strings_and_comments() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("001_attach_words.sql");
    fs::write(
        &path,
        "CREATE TABLE notes(value TEXT DEFAULT 'ATTACH DATABASE');
         -- DETACH DATABASE other
         /* ATTACH DATABASE other */",
    )
    .unwrap();

    let migration = parse_migration_file(&path).unwrap();
    assert_eq!(migration.version, "001");
}

#[test]
fn migration_sql_rejects_unclosed_quotes_and_comments() {
    let cases = [
        (
            "001_unclosed_string.sql",
            "CREATE TABLE notes(value TEXT DEFAULT 'x);",
        ),
        ("002_unclosed_identifier.sql", "CREATE TABLE \"items(id);"),
        (
            "003_unclosed_block_comment.sql",
            "CREATE TABLE items(id); /* broken",
        ),
    ];

    for (file_name, sql) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join(file_name);
        fs::write(&path, sql).unwrap();

        let error = parse_migration_file(&path).unwrap_err().to_string();
        assert!(error.contains("閉じていません"), "{file_name}: {error}");
    }
}

#[test]
fn migration_sql_rejects_transaction_control_statements() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("001_transaction.sql");
    fs::write(&path, "CREATE TABLE items(id INTEGER PRIMARY KEY); COMMIT;").unwrap();

    let error = parse_migration_file(&path).unwrap_err().to_string();
    assert!(error.contains("transaction制御文"));

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
    let sql = "CREATE TABLE direct_items(id INTEGER PRIMARY KEY); ROLLBACK;";
    let migration = Migration {
        group: "main".to_string(),
        filename: "001_direct.sql".to_string(),
        version: "001".to_string(),
        version_number: 1,
        name: "direct".to_string(),
        checksum: checksum_sql(sql),
        path: dir.path().join("001_direct.sql"),
        sql: sql.to_string(),
    };

    let result = migrate_database(&config, &database, &[migration], false);
    assert!(!result.success);
    assert!(result.error.unwrap().contains("transaction制御文"));
}

#[test]
fn migration_sql_rejects_begin_and_end_transaction_forms() {
    let dir = tempdir().unwrap();
    for (file_name, sql) in [
        (
            "001_begin.sql",
            "BEGIN TRANSACTION; CREATE TABLE items(id);",
        ),
        (
            "002_begin_immediate.sql",
            "BEGIN IMMEDIATE; CREATE TABLE items(id);",
        ),
        (
            "003_end_transaction.sql",
            "CREATE TABLE items(id); END TRANSACTION;",
        ),
        ("004_end.sql", "CREATE TABLE items(id); END;"),
    ] {
        let path = dir.path().join(file_name);
        fs::write(&path, sql).unwrap();
        let error = parse_migration_file(&path).unwrap_err().to_string();
        assert!(error.contains("transaction制御文"));
    }
}

#[test]
fn migration_sql_rejects_transaction_after_malformed_trigger_prefix() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("001_bad_trigger.sql");
    fs::write(
        &path,
        "CREATE TRIGGER bad_trigger; BEGIN TRANSACTION; CREATE TABLE items(id);",
    )
    .unwrap();

    let error = parse_migration_file(&path).unwrap_err().to_string();
    assert!(error.contains("transaction制御文"));
}

#[test]
fn migration_sql_allows_transaction_words_in_strings_and_comments() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("001_words.sql");
    fs::write(
        &path,
        "CREATE TABLE notes(value TEXT DEFAULT 'COMMIT');
         CREATE TRIGGER notes_ai AFTER INSERT ON notes BEGIN SELECT 1; END;
         CREATE TEMP TRIGGER notes_bi BEFORE INSERT ON notes BEGIN SELECT 1; END;
         -- ROLLBACK
         /* SAVEPOINT */",
    )
    .unwrap();

    let migration = parse_migration_file(&path).unwrap();
    assert_eq!(migration.name, "words");
}

#[test]
fn migration_sql_allows_transaction_words_as_identifiers() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("001_identifiers.sql");
    fs::write(
        &path,
        "CREATE TABLE release(id INTEGER PRIMARY KEY, rollback TEXT);
         CREATE INDEX commit_idx ON release(rollback);",
    )
    .unwrap();

    let migration = parse_migration_file(&path).unwrap();
    assert_eq!(migration.name, "identifiers");
}

#[test]
fn migration_sql_rejects_vacuum() {
    let cases = [
        ("001_vacuum.sql", "VACUUM;"),
        ("002_vacuum_into.sql", "VACUUM INTO 'copy.db';"),
    ];

    for (file_name, sql) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join(file_name);
        fs::write(&path, sql).unwrap();

        let error = parse_migration_file(&path).unwrap_err().to_string();
        assert!(error.contains("非transaction文"), "{error}");
    }
}

#[test]
fn migration_sql_rejects_writable_schema_pragma() {
    let cases = [
        ("001_writable_schema.sql", "PRAGMA writable_schema = ON;"),
        ("002_schema_qualified.sql", "PRAGMA main.writable_schema=1;"),
        ("003_journal_mode_off.sql", "PRAGMA journal_mode = OFF;"),
        (
            "004_journal_mode_call.sql",
            "PRAGMA main.journal_mode(OFF);",
        ),
        (
            "005_journal_mode_string.sql",
            "PRAGMA journal_mode = 'OFF';",
        ),
        ("006_journal_mode_zero.sql", "PRAGMA journal_mode = 0;"),
        (
            "007_journal_mode_zero_padded.sql",
            "PRAGMA journal_mode = 00;",
        ),
        (
            "008_journal_mode_signed_zero.sql",
            "PRAGMA journal_mode = +0;",
        ),
        (
            "009_bom_journal_mode_off.sql",
            "\u{feff}PRAGMA journal_mode = OFF;",
        ),
    ];

    for (file_name, sql) in cases {
        let dir = tempdir().unwrap();
        let path = dir.path().join(file_name);
        fs::write(&path, sql).unwrap();

        let error = parse_migration_file(&path).unwrap_err().to_string();
        assert!(error.contains("危険PRAGMA"), "{error}");
    }
}

#[test]
fn direct_migration_sql_rejects_history_table_writes() {
    let cases = [
        "INSERT INTO _sqlite_fleet_migrations (version, name, checksum, applied_at, execution_ms) VALUES ('999', 'fake', 'x', 0, 0);",
        "UPDATE main.\"_sqlite_fleet_migrations\" SET checksum = 'x';",
        "UPDATE OR ROLLBACK \"main\".\"_sqlite_fleet_migrations\" SET checksum = 'x';",
        "DELETE FROM main._sqlite_fleet_migrations;",
        "DROP TABLE IF EXISTS main._sqlite_fleet_migrations;",
        "DROP VIEW IF EXISTS main._sqlite_fleet_migrations;",
        "DROP INDEX IF EXISTS main._sqlite_fleet_migrations;",
        "DROP TRIGGER IF EXISTS main._sqlite_fleet_migrations;",
        "ALTER TABLE _sqlite_fleet_migrations RENAME TO old_history;",
        "CREATE TABLE IF NOT EXISTS _sqlite_fleet_migrations(version TEXT);",
        "CREATE TEMP TABLE _sqlite_fleet_migrations(version TEXT);",
        "CREATE TEMPORARY TABLE _sqlite_fleet_migrations(version TEXT);",
        "CREATE VIEW _sqlite_fleet_migrations AS SELECT 1 AS version;",
        "CREATE TEMP VIEW _sqlite_fleet_migrations AS SELECT 1 AS version;",
        "CREATE VIRTUAL TABLE _sqlite_fleet_migrations USING fts5(version);",
        "CREATE INDEX history_checksum_idx ON _sqlite_fleet_migrations(checksum);",
        "CREATE TEMP INDEX history_checksum_idx ON _sqlite_fleet_migrations(checksum);",
        "CREATE TEMPORARY INDEX history_checksum_idx ON _sqlite_fleet_migrations(checksum);",
        "CREATE UNIQUE INDEX IF NOT EXISTS main.history_version_idx ON main._sqlite_fleet_migrations(version);",
        "CREATE TRIGGER history_ai AFTER INSERT ON _sqlite_fleet_migrations BEGIN SELECT 1; END;",
        "CREATE TEMP TRIGGER history_au AFTER UPDATE OF \"checksum\" ON main._sqlite_fleet_migrations BEGIN SELECT 1; END;",
        "CREATE TRIGGER items_ai AFTER INSERT ON items BEGIN INSERT INTO _sqlite_fleet_migrations (version, name, checksum, applied_at, execution_ms) VALUES ('999', 'fake', 'x', 0, 0); END;",
        "WITH stale AS (SELECT version FROM _sqlite_fleet_migrations) DELETE FROM _sqlite_fleet_migrations WHERE version IN stale;",
        "WITH RECURSIVE stale(version) AS (SELECT '001') UPDATE _sqlite_fleet_migrations SET checksum = 'x';",
        "WITH source(version) AS NOT MATERIALIZED (SELECT '999') INSERT INTO _sqlite_fleet_migrations (version, name, checksum, applied_at, execution_ms) SELECT version, 'fake', 'x', 0, 0 FROM source;",
        "INSERT OR IGNORE INTO main._sqlite_fleet_migrations (version, name, checksum, applied_at, execution_ms) VALUES ('999', 'fake', 'x', 0, 0);",
        "DELETE FROM _sqlite_fleet_migrations",
        "DROP TABLE _sqlite_fleet_migrations -- no trailing token",
        "\u{feff}INSERT INTO _sqlite_fleet_migrations (version, name, checksum, applied_at, execution_ms) VALUES ('999', 'fake', 'x', 0, 0);",
    ];

    for sql in cases {
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
        let migration = Migration {
            group: "main".to_string(),
            filename: "001_direct.sql".to_string(),
            version: "001".to_string(),
            version_number: 1,
            name: "direct".to_string(),
            checksum: checksum_sql(sql),
            path: dir.path().join("001_direct.sql"),
            sql: sql.to_string(),
        };

        let result = migrate_database(&config, &database, &[migration], false);
        assert!(!result.success);
        assert!(result.error.unwrap().contains("履歴テーブル"));
    }
}
