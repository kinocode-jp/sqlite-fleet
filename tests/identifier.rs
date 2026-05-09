use rusqlite::Connection;
use sqlite_fleet::{ensure_migrations_table, Config, MigrationsConfig};

#[test]
fn migration_table_identifier_rejects_sqlite_keyword() {
    let conn = Connection::open_in_memory().unwrap();

    let error = ensure_migrations_table(&conn, "select")
        .unwrap_err()
        .to_string();
    assert!(error.contains("SQLite予約語"));

    let config = Config {
        migrations: MigrationsConfig {
            table: "select".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("SQLite予約語"));
}

#[test]
fn migration_table_identifier_rejects_sqlite_internal_prefix() {
    let conn = Connection::open_in_memory().unwrap();

    let error = ensure_migrations_table(&conn, "sqlite_fleet_migrations")
        .unwrap_err()
        .to_string();
    assert!(error.contains("SQLite内部予約名"));

    let config = Config {
        migrations: MigrationsConfig {
            table: "SQLite_fleet_migrations".to_string(),
            ..MigrationsConfig::default()
        },
        ..Config::default()
    };
    let error = config.validate().unwrap_err().to_string();
    assert!(error.contains("SQLite内部予約名"));
}
