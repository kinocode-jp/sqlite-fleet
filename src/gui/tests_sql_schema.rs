    #[test]
    fn sql_dry_run_rejects_attach_and_detach_outside_comments_and_strings() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch("CREATE TABLE existing(id INTEGER PRIMARY KEY);")
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        assert!(api_sql(
            &config,
            "tenant",
            true,
            br#"{"sql":"ATTACH DATABASE 'other.db' AS other;"}"#
        )
        .is_err());
        assert!(api_sql(
            &config,
            "tenant",
            true,
            br#"{"sql":"CREATE TABLE attach(id INTEGER); INSERT INTO attach(id) VALUES (1);"}"#
        )
        .is_ok());
        assert!(!sql_contains_statement_keyword(
            "SELECT 'ATTACH DATABASE'; -- DETACH DATABASE\n/* ATTACH */",
            &["ATTACH", "DETACH"]
        ));
        assert!(!sql_contains_statement_keyword(
            "SELECT [name]] with ATTACH and DETACH keywords] FROM existing;",
            &["ATTACH", "DETACH"]
        ));
        assert!(sql_contains_statement_keyword(
            "EXPLAIN ATTACH DATABASE 'other.db' AS other;",
            &["ATTACH", "DETACH"]
        ));
        assert!(sql_contains_statement_keyword(
            "EXPLAIN QUERY PLAN ATTACH DATABASE 'other.db' AS other;",
            &["ATTACH", "DETACH"]
        ));
        assert!(!sql_contains_statement_keyword(
            "EXPLAIN QUERY PLAN SELECT attach FROM existing;",
            &["ATTACH", "DETACH"]
        ));
        assert!(!sql_contains_statement_keyword(
            "EXPLAIN SELECT attach FROM existing;",
            &["ATTACH", "DETACH"]
        ));
    }

    #[test]
    fn gui_sql_rejects_attach_for_dry_run_and_apply() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch("CREATE TABLE existing(id INTEGER PRIMARY KEY);")
            .unwrap();
        let attached_path = dir.path().join("attached.db");
        Connection::open(&attached_path)
            .unwrap()
            .execute_batch("CREATE TABLE existing(id INTEGER PRIMARY KEY);")
            .unwrap();
        let sql = format!(
            "ATTACH DATABASE {} AS other; CREATE TABLE other.created(id INTEGER); DETACH DATABASE other;",
            quote_sql_string(&attached_path.display().to_string())
        );
        let body = serde_json::to_vec(&serde_json::json!({ "sql": sql })).unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        assert!(api_sql(&config, "tenant", true, &body).is_err());
        assert!(api_sql(&config, "tenant", false, &body).is_err());

        let created_count: i64 = Connection::open(&attached_path)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'created'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(created_count, 0);
    }

    #[test]
    fn sql_dry_run_rejects_vacuum_into_but_allows_plain_vacuum() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch("CREATE TABLE existing(id INTEGER PRIMARY KEY);")
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        assert!(api_sql(
            &config,
            "tenant",
            true,
            br#"{"sql":"VACUUM INTO 'copy.db';"}"#
        )
        .is_err());
        assert!(api_sql(
            &config,
            "tenant",
            true,
            br#"{"sql":"VACUUM main INTO 'copy.db';"}"#
        )
        .is_err());
        assert!(api_sql(&config, "tenant", true, br#"{"sql":"VACUUM;"}"#).is_ok());
        assert!(!sql_contains_vacuum_into(
            "SELECT 'VACUUM INTO copy.db'; -- VACUUM INTO\n/* VACUUM INTO */",
        ));
        assert!(sql_contains_vacuum_into("VACUUM main INTO 'copy.db';"));
        assert!(!sql_contains_vacuum_into(
            "VACUUM; INSERT INTO existing(id) VALUES (1);"
        ));
        assert!(!sql_contains_vacuum_into("VACUUM; INTO existing;"));
        assert!(!sql_contains_vacuum_into(
            "WITH vacuum(id) AS (SELECT 1) INSERT INTO existing(id) SELECT id FROM vacuum;"
        ));
        assert!(sql_contains_vacuum_into(
            "EXPLAIN QUERY PLAN VACUUM main INTO 'copy.db';"
        ));
        assert!(sql_contains_vacuum_into(
            "EXPLAIN VACUUM main INTO 'copy.db';"
        ));
    }

    #[test]
    fn gui_sql_allows_vacuum_into_only_when_applying() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE existing(id INTEGER PRIMARY KEY); INSERT INTO existing(id) VALUES (1);",
            )
            .unwrap();
        let copy_path = dir.path().join("copy.db");
        let sql = format!(
            "VACUUM INTO {};",
            quote_sql_string(&copy_path.display().to_string())
        );
        let body = serde_json::to_vec(&serde_json::json!({ "sql": sql })).unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        assert!(api_sql(&config, "tenant", true, &body).is_err());
        assert!(!copy_path.exists());
        assert!(api_sql(&config, "tenant", false, &body).is_ok());

        let copied_count: i64 = Connection::open(&copy_path)
            .unwrap()
            .query_row("SELECT count(*) FROM existing", [], |row| row.get(0))
            .unwrap();
        assert_eq!(copied_count, 1);

        let mixed_sql = format!(
            "VACUUM INTO {}; INSERT INTO existing(id) VALUES (2);",
            quote_sql_string(&dir.path().join("mixed-copy.db").display().to_string())
        );
        let mixed_body = serde_json::to_vec(&serde_json::json!({ "sql": mixed_sql })).unwrap();
        assert!(api_sql(&config, "tenant", false, &mixed_body).is_err());
    }

    #[test]
    fn dry_run_database_copy_is_removed_after_sql_error() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch("CREATE TABLE existing(id INTEGER PRIMARY KEY);")
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };
        let database = find_database(&config, "tenant").unwrap();
        let copy = create_dry_run_database_copy(&config, &database).unwrap();
        let path = copy.path().to_path_buf();
        assert!(path.exists());

        let result = execute_sql_on_dry_run_copy(copy, "CREATE TABLE broken(", 1000);

        assert!(result.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn dry_run_database_copy_removes_sqlite_sidecars_after_sql_error() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch("CREATE TABLE existing(id INTEGER PRIMARY KEY);")
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };
        let database = find_database(&config, "tenant").unwrap();
        let copy = create_dry_run_database_copy(&config, &database).unwrap();
        let path = copy.path().to_path_buf();
        let file_name = path.file_name().unwrap().to_string_lossy();
        let wal_path = path.with_file_name(format!("{file_name}-wal"));
        let shm_path = path.with_file_name(format!("{file_name}-shm"));
        let journal_path = path.with_file_name(format!("{file_name}-journal"));
        std::fs::write(&wal_path, b"wal").unwrap();
        std::fs::write(&shm_path, b"shm").unwrap();
        std::fs::write(&journal_path, b"journal").unwrap();

        let result = execute_sql_on_dry_run_copy(copy, "CREATE TABLE broken(", 1000);

        assert!(result.is_err());
        assert!(!path.exists());
        assert!(!wal_path.exists());
        assert!(!shm_path.exists());
        assert!(!journal_path.exists());
    }

    #[test]
    fn sqlite_database_file_cleanup_handles_non_utf8_file_names() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let db_path = {
            use std::os::unix::ffi::OsStringExt;
            dir.path()
                .join(OsString::from_vec(b"dry-run-\xFF.db".to_vec()))
        };
        #[cfg(not(unix))]
        let db_path = dir.path().join("dry-run.db");
        let file_name = db_path.file_name().unwrap().to_os_string();
        let wal_path = db_path.with_file_name(append_os_suffix(&file_name, "-wal"));
        let shm_path = db_path.with_file_name(append_os_suffix(&file_name, "-shm"));
        let journal_path = db_path.with_file_name(append_os_suffix(&file_name, "-journal"));
        if std::fs::write(&db_path, b"db").is_err() {
            return;
        }
        std::fs::write(&wal_path, b"wal").unwrap();
        std::fs::write(&shm_path, b"shm").unwrap();
        std::fs::write(&journal_path, b"journal").unwrap();

        remove_sqlite_database_files(&db_path);

        assert!(!db_path.exists());
        assert!(!wal_path.exists());
        assert!(!shm_path.exists());
        assert!(!journal_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn append_os_suffix_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let file_name = OsString::from_vec(b"dry-run-\xFF.db".to_vec());
        let suffixed = append_os_suffix(&file_name, "-wal");

        assert_eq!(suffixed.as_os_str().as_bytes(), b"dry-run-\xFF.db-wal");
    }

    #[test]
    fn schema_reads_generated_columns_with_table_xinfo() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                r#"
                CREATE TABLE generated_example(
                    value INTEGER,
                    value_text TEXT GENERATED ALWAYS AS (printf('%d', value)) VIRTUAL
                );
                "#,
            )
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        let schema = api_schema(&config, "tenant").unwrap();
        let table = schema
            .tables
            .iter()
            .find(|table| table.name == "generated_example")
            .unwrap();
        let generated_column = table
            .columns
            .iter()
            .find(|column| column.name == "value_text")
            .unwrap();

        assert_eq!(generated_column.hidden, 2);
    }

    #[test]
    fn schema_includes_views_indexes_and_triggers() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                r#"
                CREATE TABLE events(id INTEGER PRIMARY KEY, name TEXT);
                CREATE INDEX events_name_idx ON events(name);
                CREATE VIEW event_names AS SELECT name FROM events;
                CREATE TRIGGER events_ai AFTER INSERT ON events BEGIN
                  UPDATE events SET name = NEW.name WHERE id = NEW.id;
                END;
                "#,
            )
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        let schema = api_schema(&config, "tenant").unwrap();

        assert!(schema
            .objects
            .iter()
            .any(|object| object.object_type == "index"
                && object.name == "events_name_idx"
                && object.table_name == "events"));
        assert!(schema
            .objects
            .iter()
            .any(|object| object.object_type == "view"
                && object.name == "event_names"
                && object.sql.as_deref().unwrap_or("").contains("CREATE VIEW")));
        assert!(schema
            .objects
            .iter()
            .any(|object| object.object_type == "trigger"
                && object.name == "events_ai"
                && object
                    .sql
                    .as_deref()
                    .unwrap_or("")
                    .contains("CREATE TRIGGER")));
        assert!(schema.tables.iter().any(|table| table.object_type == "view"
            && table.name == "event_names"
            && table.columns.iter().any(|column| column.name == "name")));
    }

    #[test]
    fn schema_keeps_user_objects_with_sqlite_like_names() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                r#"
                CREATE TABLE "sqliteX_user"(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT);
                CREATE INDEX "sqliteX_user_name_idx" ON "sqliteX_user"(name);
                "#,
            )
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        let schema = api_schema(&config, "tenant").unwrap();

        assert!(schema
            .tables
            .iter()
            .any(|table| table.name == "sqliteX_user"));
        assert!(!schema
            .tables
            .iter()
            .any(|table| table.name == "sqlite_sequence"));
        assert!(schema
            .objects
            .iter()
            .any(|object| object.name == "sqliteX_user_name_idx"));
    }

    #[test]
    fn schema_includes_virtual_tables_but_omits_shadow_tables() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(r#"CREATE VIRTUAL TABLE docs USING fts5(title, body);"#)
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };

        let schema = api_schema(&config, "tenant").unwrap();

        assert!(schema
            .tables
            .iter()
            .any(|table| table.object_type == "virtual"
                && table.name == "docs"
                && table.columns.iter().any(|column| column.name == "title")));
        assert!(!schema
            .tables
            .iter()
            .any(|table| table.name.starts_with("docs_")));
    }

    #[test]
    fn validates_loopback_host_header() {
        let mut headers = HashMap::new();
        let bind_ip = "127.0.0.1".parse().unwrap();
        headers.insert("host".to_string(), "127.0.0.1:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_ok());

        headers.insert("host".to_string(), "localhost:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_ok());

        headers.insert("host".to_string(), "localhost.:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_ok());

        let ipv6_bind_ip = "::1".parse().unwrap();
        headers.insert("host".to_string(), "localhost:8765".to_string());
        assert!(validate_host_header(&headers, ipv6_bind_ip, 8765).is_ok());

        headers.insert("host".to_string(), "[::1]:8765".to_string());
        assert!(validate_host_header(&headers, ipv6_bind_ip, 8765).is_ok());
    }

    #[test]
    fn rejects_non_loopback_host_header() {
        let mut headers = HashMap::new();
        let bind_ip = "127.0.0.1".parse().unwrap();
        headers.insert("host".to_string(), "example.com:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "127.0.0.1:9000".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "127.0.0.1:08765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "127.0.0.1:87x5".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), ":8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "[]:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "[localhost]:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "[127.0.0.1]:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "127.0.0.1".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "127.0.0.1.:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        headers.insert("host".to_string(), "[::1]:8765".to_string());
        assert!(validate_host_header(&headers, bind_ip, 8765).is_err());

        let ipv6_bind_ip = "::1".parse().unwrap();
        headers.insert("host".to_string(), "::1".to_string());
        assert!(validate_host_header(&headers, ipv6_bind_ip, 80).is_err());

        let alternate_loopback_ip = "127.0.0.2".parse().unwrap();
        headers.insert("host".to_string(), "localhost:8765".to_string());
        assert!(validate_host_header(&headers, alternate_loopback_ip, 8765).is_err());

        assert!(validate_host_header(&HashMap::new(), bind_ip, 8765).is_err());
    }

    #[test]
    fn gui_host_must_be_loopback() {
        assert!(validate_gui_host("127.0.0.1").is_ok());
        assert!(validate_gui_host("localhost").is_ok());
        assert!(validate_gui_host("0.0.0.0").is_err());
    }

    #[test]
    fn csrf_token_is_hex_256_bits() {
        let token = generate_csrf_token().unwrap();

        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

