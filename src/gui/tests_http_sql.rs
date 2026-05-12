    fn assert_response_security_headers(response: &str) {
        assert_content_length_matches_body(response);
        assert!(
            response.contains("\r\nCache-Control: no-store\r\n"),
            "{response}"
        );
        assert!(response.contains("\r\nConnection: close\r\n"), "{response}");
        assert!(
            response.contains("\r\nX-Content-Type-Options: nosniff\r\n"),
            "{response}"
        );
        assert!(
            response.contains("\r\nX-Frame-Options: DENY\r\n"),
            "{response}"
        );
        assert!(
            response.contains("\r\nReferrer-Policy: no-referrer\r\n"),
            "{response}"
        );
        assert!(
            response.contains("\r\nContent-Security-Policy: default-src 'none';"),
            "{response}"
        );
    }

    fn assert_content_length_matches_body(response: &str) {
        let (headers, body) = response
            .split_once("\r\n\r\n")
            .unwrap_or_else(|| panic!("response headers/body separator missing: {response}"));
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap_or_else(|| panic!("Content-Length header missing: {response}"))
            .parse::<usize>()
            .unwrap();

        assert_eq!(content_length, body.len(), "{response}");
    }

    #[test]
    fn detects_complete_http_header() {
        assert_eq!(
            http_header_end(b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody"),
            Some(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n".len())
        );
        assert!(http_header_end(b"GET / HTTP/1.1\nHost: x\n\nbody").is_none());
        assert!(http_header_end(b"GET / HTTP/1.1\r\nHost: x").is_none());
    }

    #[test]
    fn rejects_bare_lf_in_http_header_lines() {
        assert!(validate_crlf_lines("GET / HTTP/1.1\r\nHost: x\r\n\r\n").is_ok());
        assert!(validate_crlf_lines("GET / HTTP/1.1\nHost: x\r\n\r\n").is_err());
        assert!(validate_crlf_lines("GET / HTTP/1.1\r\nHost: x\n\r\n").is_err());
        assert!(validate_crlf_lines("GET / HTTP/1.1\rHost: x\r\n\r\n").is_err());
    }

    #[test]
    fn parses_request_line_strictly() {
        assert_eq!(parse_request_line("GET / HTTP/1.1").unwrap(), ("GET", "/"));
        assert_eq!(
            parse_request_line("POST /api/check HTTP/1.0").unwrap(),
            ("POST", "/api/check")
        );
    }

    #[test]
    fn rejects_malformed_request_line() {
        assert!(parse_request_line("GET /").is_err());
        assert!(parse_request_line("GET http://127.0.0.1/ HTTP/1.1").is_err());
        assert!(parse_request_line("GET //127.0.0.1/ HTTP/1.1").is_err());
        assert!(parse_request_line("GET /#fragment HTTP/1.1").is_err());
        assert_eq!(
            parse_request_line("GET /api/schema?database=tenant%27one HTTP/1.1").unwrap(),
            ("GET", "/api/schema?database=tenant%27one")
        );
        assert!(parse_request_line("GET /api%2Fstate HTTP/1.1").is_err());
        assert!(parse_request_line("GET /api%2fstate HTTP/1.1").is_err());
        assert!(parse_request_line("GET /api%5Cstate HTTP/1.1").is_err());
        assert!(parse_request_line("GET /api%73tate HTTP/1.1").is_err());
        assert!(parse_request_line("GET /api\\state HTTP/1.1").is_err());
        assert!(parse_request_line("GET /\u{1f} HTTP/1.1").is_err());
        assert!(parse_request_line("GET\t/ HTTP/1.1").is_err());
        assert!(parse_request_line("GET  / HTTP/1.1").is_err());
        assert!(parse_request_line("PUT / HTTP/1.1").is_err());
        assert!(parse_request_line("GET / HTTP/2").is_err());
        assert!(parse_request_line("GET / HTTP/1.1 extra").is_err());
    }

    #[test]
    fn validates_api_token() {
        let mut headers = HashMap::new();
        headers.insert("x-sqlite-fleet-token".to_string(), "secret".to_string());

        assert!(validate_api_token(&headers, "secret").is_ok());
        assert!(validate_api_token(&headers, "different").is_err());
        assert!(validate_api_token(&HashMap::new(), "secret").is_err());
    }

    #[test]
    fn compares_api_tokens_without_prefix_acceptance() {
        assert!(constant_time_eq("secret", "secret"));
        assert!(!constant_time_eq("secret", "secrex"));
        assert!(!constant_time_eq("secret", "secret-longer"));
        assert!(!constant_time_eq("secret-longer", "secret"));
        assert!(!constant_time_eq("", "secret"));
    }

    #[test]
    fn rejects_request_body_for_post_api() {
        let mut headers = HashMap::new();
        assert!(validate_no_request_body(&headers, false).is_ok());
        assert!(validate_no_request_body(&headers, true).is_err());

        headers.insert("content-length".to_string(), "0".to_string());
        assert!(validate_no_request_body(&headers, false).is_ok());
        assert!(validate_no_request_body(&headers, true).is_err());

        headers.insert("content-length".to_string(), "00".to_string());
        assert!(validate_no_request_body(&headers, false).is_err());

        headers.insert("content-length".to_string(), "1".to_string());
        assert!(validate_no_request_body(&headers, false).is_err());

        headers.insert("content-length".to_string(), "invalid".to_string());
        assert!(validate_no_request_body(&headers, false).is_err());

        headers.clear();
        headers.insert("transfer-encoding".to_string(), "chunked".to_string());
        assert!(validate_no_request_body(&headers, false).is_err());
    }

    #[test]
    fn sql_api_requires_json_content_type() {
        let mut headers = HashMap::new();
        assert!(validate_json_content_type(&headers).is_err());

        headers.insert("content-type".to_string(), "text/plain".to_string());
        assert!(validate_json_content_type(&headers).is_err());

        headers.insert(
            "content-type".to_string(),
            "Application/JSON; charset=utf-8".to_string(),
        );
        assert!(validate_json_content_type(&headers).is_ok());
    }

    #[test]
    fn sql_api_requires_content_length_over_http() {
        let response = send_test_http_request(
            "POST /api/sql?dry_run=true&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\n\r\n{\"sql\":\"SELECT 1;\"}",
        );

        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
        assert_response_security_headers(&response);
        assert!(response.contains("Content-Length が必要です"), "{response}");
    }

    #[test]
    fn sql_api_rejects_body_larger_than_content_length_over_http() {
        let response = send_test_http_request(
            "POST /api/sql?dry_run=true&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}extra",
        );

        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
        assert_response_security_headers(&response);
        assert!(
            response.contains("HTTP request body がContent-Lengthを超えています"),
            "{response}"
        );
    }

    #[test]
    fn sql_request_rejects_unknown_json_fields() {
        let error = serde_json::from_slice::<SqlRequest>(br#"{"sql":"SELECT 1;","mode":"apply"}"#)
            .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn sql_json_body_limit_allows_wrapper_overhead() {
        let max_sql_bytes = std::hint::black_box(MAX_SQL_BYTES);
        let max_http_body_bytes = std::hint::black_box(MAX_HTTP_BODY_BYTES);
        let json_body_len_for_max_sql = r#"{"sql":""}"#.len() + max_sql_bytes;

        assert!(max_http_body_bytes > max_sql_bytes);
        assert!(json_body_len_for_max_sql <= max_http_body_bytes);
    }

    #[test]
    fn sql_request_rejects_sql_larger_than_limit_before_db_lookup() {
        let body = serde_json::to_vec(&serde_json::json!({
            "sql": "x".repeat(MAX_SQL_BYTES + 1)
        }))
        .unwrap();
        match api_sql(&Config::default(), "missing", true, &body) {
            Ok(_) => panic!("oversized SQL should fail before database lookup"),
            Err(error) => assert!(error.to_string().contains("SQL が大きすぎます")),
        }
    }

    #[test]
    fn sql_request_size_limit_counts_utf8_bytes_before_db_lookup() {
        let sql = "あ".repeat((MAX_SQL_BYTES / "あ".len()) + 1);
        assert!(sql.chars().count() < MAX_SQL_BYTES);
        assert!(utf8_byte_len(&sql) > MAX_SQL_BYTES);
        let body = serde_json::to_vec(&serde_json::json!({ "sql": sql })).unwrap();

        match api_sql(&Config::default(), "missing", true, &body) {
            Ok(_) => panic!("oversized UTF-8 SQL should fail before database lookup"),
            Err(error) => assert!(error.to_string().contains("SQL が大きすぎます")),
        }
    }

    #[test]
    fn rejects_request_body_for_get_page() {
        let response = send_test_http_request(
            "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 1\r\n\r\nx",
        );

        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
    }

    #[test]
    fn rejects_implicit_request_body_for_get_page() {
        let response = send_test_http_request("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\nx");

        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{response}"
        );
    }

    #[test]
    fn sql_dry_run_supports_transaction_and_vacuum_without_changing_source() {
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
        let body =
            br#"{"sql":"BEGIN IMMEDIATE; CREATE TABLE tx_test(id INTEGER); COMMIT; VACUUM;"}"#;

        let result = api_sql(&config, "tenant", true, body).unwrap();
        assert!(result.dry_run);

        let conn = Connection::open(&db_path).unwrap();
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'tx_test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
    }

    #[test]
    fn gui_sql_apply_rolls_back_batch_on_error() {
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
        let body = br#"{"sql":"CREATE TABLE should_rollback(id INTEGER); INSERT INTO missing_table(id) VALUES (1);"}"#;

        assert!(api_sql(&config, "tenant", false, body).is_err());

        let conn = Connection::open(&db_path).unwrap();
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'should_rollback'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
    }

    #[test]
    fn gui_sql_apply_rejects_transaction_control() {
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
        let body = br#"{"sql":"BEGIN IMMEDIATE; CREATE TABLE tx_test(id INTEGER); COMMIT;"}"#;

        let error = match api_sql(&config, "tenant", false, body) {
            Ok(_) => panic!("transaction control SQL should be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("transaction制御文"));
        let conn = Connection::open(&db_path).unwrap();
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'tx_test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);

        let body = br#"{"sql":"END TRANSACTION; CREATE TABLE end_tx_test(id INTEGER);"}"#;
        let error = match api_sql(&config, "tenant", false, body) {
            Ok(_) => panic!("END TRANSACTION SQL should be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("END TRANSACTION"));
    }

    #[test]
    fn gui_sql_apply_allows_create_trigger_body() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE existing(id INTEGER PRIMARY KEY, touched INTEGER DEFAULT 0);",
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
        let body = br#"{"sql":"CREATE TRIGGER trg_existing_insert AFTER INSERT ON existing BEGIN UPDATE existing SET touched = 1 WHERE id = NEW.id; END;"}"#;

        assert!(api_sql(&config, "tenant", false, body).is_ok());
        let body = br#"{"sql":"CREATE TRIGGER trg_existing_update AFTER UPDATE ON existing BEGIN SELECT RAISE(IGNORE); END; CREATE TABLE after_trigger(id INTEGER);"}"#;
        assert!(api_sql(&config, "tenant", false, body).is_ok());

        let trigger_count: i64 = Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'trigger' AND name IN ('trg_existing_insert', 'trg_existing_update')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trigger_count, 2);
        let table_count: i64 = Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'after_trigger'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);
    }

    #[test]
    fn gui_sql_enforces_foreign_keys_for_apply_and_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                r#"
                CREATE TABLE parent(id INTEGER PRIMARY KEY);
                CREATE TABLE child(
                    id INTEGER PRIMARY KEY,
                    parent_id INTEGER NOT NULL REFERENCES parent(id)
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
        let body = br#"{"sql":"INSERT INTO child(id, parent_id) VALUES (1, 404);"}"#;

        assert!(api_sql(&config, "tenant", true, body).is_err());
        assert!(api_sql(&config, "tenant", false, body).is_err());

        let count: i64 = Connection::open(&db_path)
            .unwrap()
            .query_row("SELECT count(*) FROM child", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn gui_sql_rejects_disabling_foreign_keys() {
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

        for body in [
            br#"{"sql":"PRAGMA foreign_keys = OFF;"}"#.as_slice(),
            br#"{"sql":"PRAGMA foreign_keys(0);"}"#.as_slice(),
            br#"{"sql":"PRAGMA foreign_keys = false;"}"#.as_slice(),
            br#"{"sql":"PRAGMA foreign_keys = no;"}"#.as_slice(),
            br#"{"sql":"PRAGMA foreign_keys = 'OFF';"}"#.as_slice(),
            br#"{"sql":"PRAGMA foreign_keys = \"0\";"}"#.as_slice(),
            br#"{"sql":"PRAGMA main.foreign_keys = OFF;"}"#.as_slice(),
            br#"{"sql":"PRAGMA main /* schema */ . /* pragma */ foreign_keys = OFF;"}"#.as_slice(),
            br#"{"sql":"PRAGMA main.foreign_keys = 'OFF';"}"#.as_slice(),
            br#"{"sql":"PRAGMA foreign_keys = 00;"}"#.as_slice(),
            br#"{"sql":"PRAGMA foreign_keys = 'off ';"}"#.as_slice(),
            br#"{"sql":"PRAGMA [foreign_keys] = OFF;"}"#.as_slice(),
            br#"{"sql":"PRAGMA `foreign_keys` = OFF;"}"#.as_slice(),
            br#"{"sql":"PRAGMA [main].[foreign_keys] = OFF;"}"#.as_slice(),
        ] {
            assert!(api_sql(&config, "tenant", true, body).is_err());
            assert!(api_sql(&config, "tenant", false, body).is_err());
        }
        assert!(api_sql(
            &config,
            "tenant",
            false,
            br#"{"sql":"PRAGMA foreign_keys = ON;"}"#
        )
        .is_ok());
        assert!(sql_unsafe_pragma(
            "SELECT 'PRAGMA foreign_keys = OFF'; -- PRAGMA foreign_keys = OFF\n/* PRAGMA foreign_keys = 0 */"
        )
        .is_none());
        assert!(sql_unsafe_pragma("SELECT 'PRAGMA', 'foreign_keys', 'OFF';").is_none());
        assert!(sql_unsafe_pragma("PRAGMA foreign_keys = '';").is_none());
        assert!(sql_unsafe_pragma("PRAGMA main /* . */ foreign_keys = OFF;").is_none());
        assert!(sql_unsafe_pragma("PRAGMA foreign_keys OFF;").is_none());
        assert!(sql_unsafe_pragma("PRAGMA main.foreign_keys OFF;").is_none());
        assert_eq!(
            sql_unsafe_pragma("PRAGMA foreign_keys /* comment */ = /* value */ OFF;"),
            Some("foreign_keys")
        );
        assert_eq!(
            sql_unsafe_pragma("PRAGMA main.foreign_keys /* comment */ ( /* value */ OFF );"),
            Some("foreign_keys")
        );
    }

    #[test]
    fn gui_sql_rejects_ignoring_check_constraints() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch("CREATE TABLE existing(id INTEGER PRIMARY KEY CHECK(id > 0));")
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

        for body in [
            br#"{"sql":"PRAGMA ignore_check_constraints = ON;"}"#.as_slice(),
            br#"{"sql":"PRAGMA ignore_check_constraints(1);"}"#.as_slice(),
            br#"{"sql":"PRAGMA main.ignore_check_constraints = 'YES';"}"#.as_slice(),
            br#"{"sql":"PRAGMA main /* schema */ . /* pragma */ ignore_check_constraints = ON;"}"#
                .as_slice(),
            br#"{"sql":"PRAGMA main.ignore_check_constraints /* sep */ ( /* value */ YES );"}"#
                .as_slice(),
            br#"{"sql":"PRAGMA ignore_check_constraints = true;"}"#.as_slice(),
            br#"{"sql":"PRAGMA ignore_check_constraints = 2;"}"#.as_slice(),
            br#"{"sql":"PRAGMA ignore_check_constraints = '+2';"}"#.as_slice(),
            br#"{"sql":"PRAGMA [ignore_check_constraints] = ON;"}"#.as_slice(),
            br#"{"sql":"PRAGMA `main`.`ignore_check_constraints` = YES;"}"#.as_slice(),
        ] {
            assert!(api_sql(&config, "tenant", true, body).is_err());
            assert!(api_sql(&config, "tenant", false, body).is_err());
        }
        assert!(api_sql(
            &config,
            "tenant",
            false,
            br#"{"sql":"PRAGMA ignore_check_constraints = OFF;"}"#
        )
        .is_ok());
        assert!(sql_unsafe_pragma(
            "SELECT 'PRAGMA foreign_keys = OFF'; -- PRAGMA foreign_keys = OFF\n/* PRAGMA foreign_keys = 0 */"
        )
        .is_none());
        assert!(sql_unsafe_pragma("PRAGMA ignore_check_constraints = -1;").is_none());
        assert!(sql_unsafe_pragma("PRAGMA main /* . */ ignore_check_constraints = ON;").is_none());
        assert!(sql_unsafe_pragma("PRAGMA ignore_check_constraints ON;").is_none());
    }

    #[test]
    fn gui_sql_rejects_schema_corruption_pragmas() {
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

        for body in [
            br#"{"sql":"PRAGMA writable_schema = ON;"}"#.as_slice(),
            br#"{"sql":"PRAGMA main.writable_schema = 1;"}"#.as_slice(),
            br#"{"sql":"PRAGMA [main].[writable_schema] = ON;"}"#.as_slice(),
            br#"{"sql":"PRAGMA journal_mode = OFF;"}"#.as_slice(),
            br#"{"sql":"PRAGMA main.journal_mode(OFF);"}"#.as_slice(),
            br#"{"sql":"PRAGMA main /* schema */ . /* pragma */ journal_mode(OFF);"}"#.as_slice(),
            br#"{"sql":"PRAGMA main.journal_mode /* sep */ = /* value */ OFF;"}"#.as_slice(),
            br#"{"sql":"PRAGMA journal_mode = 'OFF';"}"#.as_slice(),
            br#"{"sql":"PRAGMA journal_mode = 0;"}"#.as_slice(),
            br#"{"sql":"PRAGMA journal_mode = +0;"}"#.as_slice(),
            br#"{"sql":"PRAGMA [journal_mode] = \"OFF\";"}"#.as_slice(),
        ] {
            assert!(api_sql(&config, "tenant", true, body).is_err());
            assert!(api_sql(&config, "tenant", false, body).is_err());
        }
        assert!(api_sql(
            &config,
            "tenant",
            false,
            br#"{"sql":"PRAGMA journal_mode = WAL;"}"#
        )
        .is_ok());
        assert!(api_sql(
            &config,
            "tenant",
            false,
            br#"{"sql":"PRAGMA journal_mode = WAL; CREATE TABLE mixed_journal_mode(id INTEGER);"}"#
        )
        .is_err());
        let mixed_table_count: i64 = Connection::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = 'mixed_journal_mode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mixed_table_count, 0);
        assert!(sql_unsafe_pragma(
            "SELECT 'PRAGMA writable_schema = ON'; -- PRAGMA journal_mode = OFF"
        )
        .is_none());
        assert!(sql_unsafe_pragma("SELECT 'PRAGMA', 'writable_schema', 'ON';").is_none());
        assert!(sql_unsafe_pragma("PRAGMA journal_mode OFF;").is_none());
        assert!(sql_unsafe_pragma("PRAGMA main /* . */ journal_mode = OFF;").is_none());
        assert!(api_sql(
            &config,
            "tenant",
            true,
            br#"{"sql":"PRAGMA table_info(writable_schema);"}"#
        )
        .is_ok());
        assert_eq!(
            sql_unsafe_pragma("PRAGMA main.writable_schema = ON;"),
            Some("writable_schema")
        );
        assert_eq!(
            sql_unsafe_pragma("PRAGMA main /* schema */ . /* pragma */ writable_schema = ON;"),
            Some("writable_schema")
        );
        assert!(sql_unsafe_pragma("PRAGMA main /* . */ writable_schema = ON;").is_none());
    }

    #[test]
    fn unsafe_pragma_detection_only_checks_statement_start() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path)
            .unwrap()
            .execute_batch(
                r#"
                CREATE TABLE settings(
                    pragma TEXT,
                    foreign_keys TEXT,
                    writable_schema TEXT,
                    off TEXT
                );
                INSERT INTO settings VALUES ('p', 'f', 'w', 'o');
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

        assert!(api_sql(
            &config,
            "tenant",
            true,
            br#"{"sql":"SELECT pragma, foreign_keys, off FROM settings;"}"#
        )
        .is_ok());
        assert!(api_sql(
            &config,
            "tenant",
            true,
            br#"{"sql":"SELECT pragma, writable_schema, off FROM settings;"}"#
        )
        .is_ok());
        assert_eq!(
            sql_unsafe_pragma("SELECT 1; PRAGMA foreign_keys = OFF;"),
            Some("foreign_keys")
        );
    }

