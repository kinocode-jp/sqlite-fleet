    #[test]
    fn parses_headers_case_insensitively() {
        let headers = parse_headers(
            "POST /api/check HTTP/1.1\r\nHost: localhost\r\nX-SQLite-Fleet-Token: abc\r\n\r\n",
        )
        .unwrap();

        assert_eq!(
            headers.get("x-sqlite-fleet-token").map(String::as_str),
            Some("abc")
        );
    }

    #[test]
    fn rejects_duplicate_security_headers() {
        assert!(
            parse_headers("GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nHost: localhost\r\n\r\n").is_err()
        );
        assert!(parse_headers(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1\r\nX-SQLite-Fleet-Token: one\r\nX-SQLite-Fleet-Token: two\r\n\r\n"
        )
        .is_err());
    }

    #[test]
    fn rejects_duplicate_framing_headers() {
        assert!(parse_headers(
            "POST /api/check HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n"
        )
        .is_err());
        assert!(parse_headers(
            "POST /api/check HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: gzip\r\n\r\n"
        )
        .is_err());
        assert!(parse_headers(
            "POST /api/sql HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{}"
        )
        .is_err());
    }

    #[test]
    fn rejects_malformed_header_lines() {
        assert!(parse_headers("GET / HTTP/1.1\r\nHost 127.0.0.1\r\n\r\n").is_err());
        assert!(parse_headers("GET / HTTP/1.1\r\n Host: 127.0.0.1\r\n\r\n").is_err());
        assert!(parse_headers("GET / HTTP/1.1\r\n: value\r\n\r\n").is_err());
        assert!(parse_headers("GET / HTTP/1.1\r\nBad Header: value\r\n\r\n").is_err());
        assert!(parse_headers("GET / HTTP/1.1\r\nHost : 127.0.0.1\r\n\r\n").is_err());
        assert!(parse_headers("GET / HTTP/1.1\r\nHost: 127.0.0.1\u{1f}\r\n\r\n").is_err());
    }

    #[test]
    fn parses_query_parameters_strictly() {
        let query = parse_query("dry_run=true&database=tenant%27one&literal_plus=a+b").unwrap();

        assert_eq!(query.get("dry_run").map(String::as_str), Some("true"));
        assert_eq!(
            query.get("database").map(String::as_str),
            Some("tenant'one")
        );
        assert_eq!(query.get("literal_plus").map(String::as_str), Some("a+b"));
    }

    #[test]
    fn rejects_malformed_or_duplicate_query_parameters() {
        assert!(parse_query("database=%").is_err());
        assert!(parse_query("database=%ff").is_err());
        assert!(parse_query("database=one&database=two").is_err());
        assert!(parse_query("=value").is_err());
        assert!(parse_query("&dry_run=true").is_err());
        assert!(parse_query("dry_run=true&").is_err());
        assert!(parse_query("dry_run=true&&database=tenant").is_err());
        assert!(parse_query("database=tenant%0aone").is_err());
        assert!(parse_query("dry_run%0d=true").is_err());
    }

    #[test]
    fn parses_required_bool_query_parameter() {
        let true_query = parse_query("dry_run=true").unwrap();
        let false_query = parse_query("dry_run=false").unwrap();

        assert!(required_bool_query(&true_query, "dry_run").unwrap());
        assert!(!required_bool_query(&false_query, "dry_run").unwrap());
    }

    #[test]
    fn rejects_missing_or_invalid_bool_query_parameter() {
        let missing_query = parse_query("database=tenant").unwrap();
        let invalid_query = parse_query("dry_run=1").unwrap();

        assert!(required_bool_query(&missing_query, "dry_run").is_err());
        assert!(required_bool_query(&invalid_query, "dry_run").is_err());
    }

    #[test]
    fn validates_optional_nonempty_query_parameter() {
        let query = parse_query("database=tenant").unwrap();
        let empty_query = parse_query("database=").unwrap();

        assert_eq!(
            optional_nonempty_query(&query, "database").unwrap(),
            Some("tenant")
        );
        assert_eq!(optional_nonempty_query(&query, "missing").unwrap(), None);
        assert!(optional_nonempty_query(&empty_query, "database").is_err());
    }

    #[test]
    fn rejects_unknown_query_parameters() {
        let query = parse_query("dry_run=true&database=tenant").unwrap();
        let unknown_query = parse_query("dry_run=true&databse=tenant").unwrap();

        assert!(validate_query_keys(&query, &["dry_run", "database"]).is_ok());
        assert!(validate_query_keys(&unknown_query, &["dry_run", "database"]).is_err());
    }

    #[test]
    fn validates_no_query_for_plain_api_endpoints() {
        assert!(validate_no_query("").is_ok());
        assert!(validate_no_query("unexpected=true").is_err());
    }

    #[test]
    fn detects_api_namespace_paths() {
        assert!(is_api_path("/api"));
        assert!(is_api_path("/api/"));
        assert!(is_api_path("/api/state"));
        assert!(!is_api_path("/"));
        assert!(!is_api_path("/api%2fstate"));
        assert!(!is_api_path("/api%2Fstate"));
        assert!(!is_api_path("/API%2Fstate"));
        assert!(!is_api_path("/apix"));
        assert!(!is_api_path("/apiary"));
    }

    #[test]
    fn not_found_response_uses_api_envelope_shape() {
        let response = send_test_http_request(
            "GET /api/missing HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
        );

        assert!(response.starts_with("HTTP/1.1 404 Not Found"), "{response}");
        assert_response_security_headers(&response);
        assert!(
            response.ends_with(r#"{"ok":false,"data":null,"error":"not found"}"#),
            "{response}"
        );
    }

    #[test]
    fn api_request_requires_valid_token() {
        let response = send_test_http_request(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: wrong\r\n\r\n",
        );

        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert_response_security_headers(&response);
        assert!(response.contains(r#""ok":false"#), "{response}");
        assert!(response.contains("GUI API token が不正です"), "{response}");
    }

    #[test]
    fn api_namespace_root_requires_valid_token() {
        let forbidden =
            send_test_http_request("GET /api HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(
            forbidden.starts_with("HTTP/1.1 403 Forbidden"),
            "{forbidden}"
        );
        assert!(
            forbidden.contains("GUI API token が不正です"),
            "{forbidden}"
        );

        let not_found = send_test_http_request(
            "GET /api HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
        );
        assert!(
            not_found.starts_with("HTTP/1.1 404 Not Found"),
            "{not_found}"
        );
    }

    #[test]
    fn api_namespace_child_requires_valid_token_before_not_found() {
        let forbidden =
            send_test_http_request("GET /api/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(
            forbidden.starts_with("HTTP/1.1 403 Forbidden"),
            "{forbidden}"
        );
        assert!(
            forbidden.contains("GUI API token が不正です"),
            "{forbidden}"
        );

        let not_found = send_test_http_request(
            "GET /api/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
        );
        assert!(
            not_found.starts_with("HTTP/1.1 404 Not Found"),
            "{not_found}"
        );
        assert_response_security_headers(&not_found);
    }

    #[test]
    fn percent_encoded_path_is_rejected_before_api_routing() {
        let forbidden =
            send_test_http_request("GET /api%2Fstate HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(
            forbidden.starts_with("HTTP/1.1 400 Bad Request"),
            "{forbidden}"
        );
        assert!(
            forbidden.contains("request target のpathにpercent encodingは指定できません"),
            "{forbidden}"
        );

        let rejected =
            send_test_http_request("GET /api%5Cstate HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(
            rejected.starts_with("HTTP/1.1 400 Bad Request"),
            "{rejected}"
        );
        assert_response_security_headers(&rejected);

        let encoded_api_name =
            send_test_http_request("GET /api%73tate HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        assert!(
            encoded_api_name.starts_with("HTTP/1.1 400 Bad Request"),
            "{encoded_api_name}"
        );
        assert!(
            encoded_api_name.contains("request target のpathにpercent encodingは指定できません"),
            "{encoded_api_name}"
        );
    }

    #[test]
    fn api_like_paths_do_not_require_api_token() {
        for path in ["/apix", "/apiary"] {
            let response = send_test_http_request(&format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\n\r\n"
            ));

            assert!(response.starts_with("HTTP/1.1 404 Not Found"), "{response}");
            assert_response_security_headers(&response);
        }
    }

    #[test]
    fn page_request_does_not_require_api_token() {
        let response = send_test_http_request("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert_response_security_headers(&response);
        assert!(response.contains("text/html; charset=utf-8"), "{response}");
        assert!(response.contains("sqlite-fleet"), "{response}");
    }

    #[test]
    fn request_rejects_non_loopback_host_header_over_http() {
        let response = send_test_http_request("GET / HTTP/1.1\r\nHost: example.com:{port}\r\n\r\n");

        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert_response_security_headers(&response);
        assert!(response.contains(r#""ok":false"#), "{response}");
        assert!(
            response.contains("Host header はループバックホストのみ許可されます"),
            "{response}"
        );
    }

    fn send_test_http_request(request_template: &str) -> String {
        send_test_http_request_with_config(request_template, Config::default())
    }

    fn send_test_http_request_with_config(request_template: &str, config: Config) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let state = ServerState {
            config: Mutex::new(config),
            config_path: std::env::temp_dir().join("sqlite-fleet-test.toml"),
            csrf_token: "token".to_string(),
            script_nonce: "nonce".to_string(),
            bind_ip: addr.ip(),
            port: addr.port(),
        };

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, &state).unwrap();
        });

        let mut client = TcpStream::connect(addr).unwrap();
        let request = request_template.replace("{port}", &addr.port().to_string());
        client.write_all(request.as_bytes()).unwrap();
        client.flush().unwrap();
        client.shutdown(std::net::Shutdown::Write).unwrap();

        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        server.join().unwrap();
        response
    }

    fn test_server_state(config: Config, config_path: PathBuf) -> ServerState {
        ServerState {
            config: Mutex::new(config),
            config_path,
            csrf_token: "token".to_string(),
            script_nonce: "nonce".to_string(),
            bind_ip: Ipv4Addr::LOCALHOST.into(),
            port: 0,
        }
    }

    #[test]
    fn gui_post_apis_enforce_allow_flags_server_side() {
        let mut config = Config::default();
        config.gui.allow_check = false;
        let response = send_test_http_request_with_config(
            "POST /api/check HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI check は設定で無効化されています"));

        let mut config = Config::default();
        config.gui.allow_migrate = false;
        let response = send_test_http_request_with_config(
            "POST /api/migrate?dry_run=true HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI migrate は設定で無効化されています"));

        let mut config = Config::default();
        config.gui.allow_sql_apply = false;
        let response = send_test_http_request_with_config(
            "POST /api/sql?dry_run=false&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\nContent-Length: 19\r\n\r\n{\"sql\":\"SELECT 1;\"}",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI SQL apply は設定で無効化されています"));

        let mut config = Config::default();
        config.gui.allow_backup = false;
        let response = send_test_http_request_with_config(
            "POST /api/backup HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI backup は設定で無効化されています"));

        let mut config = Config::default();
        config.gui.allow_migration_edit = false;
        let body = r#"{"allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/gui-permissions HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI permission edit は設定で無効化されています"));
    }

    #[test]
    fn api_save_gui_permissions_persists_allow_flags() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        let state = test_server_state(Config::default(), config_path.clone());

        api_save_gui_permissions(
            &state,
            br#"{"allow_check":false,"allow_migrate":true,"allow_backup":false,"allow_restore":true,"allow_sql_apply":false,"allow_migration_edit":true}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert!(!config.gui.allow_check);
        assert!(config.gui.allow_migrate);
        assert!(!config.gui.allow_backup);
        assert!(config.gui.allow_restore);
        assert!(!config.gui.allow_sql_apply);
        assert!(config.gui.allow_migration_edit);
        let saved = std::fs::read_to_string(config_path).unwrap();
        assert!(saved.contains("allow_check = false"), "{saved}");
        assert!(saved.contains("allow_sql_apply = false"), "{saved}");
    }

    #[test]
    fn api_db_groups_resolve_relative_path_selectors_for_preview() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let database_path = data_dir.join("tenant.db");
        Connection::open(&database_path).unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config
            .db_groups
            .insert("canary".to_string(), vec!["data/tenant.db".to_string()]);
        let databases = vec![sqlite_fleet::Database {
            id: "tenant".to_string(),
            path: database_path,
            exists: true,
            readable: true,
        }];

        let groups = api_db_groups(&config, &databases);

        assert_eq!(groups.len(), 2);
        let group = groups.iter().find(|group| group.name == "canary").unwrap();
        assert_eq!(group.selectors, vec!["data/tenant.db"]);
        assert_eq!(group.database_ids, vec!["tenant"]);
        let all = groups.iter().find(|group| group.name == "all").unwrap();
        assert_eq!(all.database_ids, vec!["tenant"]);
    }

    #[test]
    fn api_db_groups_preserve_selector_order_for_limit_preview() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let db1_path = data_dir.join("db1.db");
        let db2_path = data_dir.join("db2.db");
        Connection::open(&db1_path).unwrap();
        Connection::open(&db2_path).unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config.db_groups.insert(
            "canary".to_string(),
            vec!["db2".to_string(), "db1".to_string()],
        );
        let databases = vec![
            sqlite_fleet::Database {
                id: "db1".to_string(),
                path: db1_path,
                exists: true,
                readable: true,
            },
            sqlite_fleet::Database {
                id: "db2".to_string(),
                path: db2_path,
                exists: true,
                readable: true,
            },
        ];

        let groups = api_db_groups(&config, &databases);

        let group = groups.iter().find(|group| group.name == "canary").unwrap();
        assert_eq!(group.database_ids, vec!["db2", "db1"]);
    }

    #[test]
    fn api_save_migration_group_preserves_existing_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config.migration_groups.insert(
            "legacy".to_string(),
            MigrationGroupConfig {
                dir: Some("legacy_migrations".to_string()),
                migrations: vec!["001_initial.sql".to_string()],
            },
        );
        let config_path = dir.path().join("sqlite-fleet.toml");
        let state = test_server_state(config, config_path.clone());

        api_save_migration_group(
            &state,
            br#"{"name":"legacy","versions":["001_initial.sql","002_more.sql"]}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        let group = config.migration_groups.get("legacy").unwrap();
        assert_eq!(group.dir.as_deref(), Some("legacy_migrations"));
        assert_eq!(group.migrations, vec!["001_initial.sql", "002_more.sql"]);
        let saved = std::fs::read_to_string(config_path).unwrap();
        assert!(saved.contains("dir = \"legacy_migrations\""), "{saved}");
    }

    #[test]
    fn api_save_new_migration_group_branches_from_implicit_main_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        std::fs::write(
            migration_dir.join("001_initial.sql"),
            "CREATE TABLE initial(id INTEGER);",
        )
        .unwrap();
        std::fs::write(
            migration_dir.join("002_more.sql"),
            "CREATE TABLE more(id INTEGER);",
        )
        .unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_save_migration_group(
            &state,
            br#"{"name":"premium","versions":[]}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert_eq!(
            config
                .migration_groups
                .get("main")
                .unwrap()
                .migrations,
            vec!["001_initial.sql", "002_more.sql"]
        );
        assert_eq!(
            config
                .migration_groups
                .get("premium")
                .unwrap()
                .migrations,
            vec!["001_initial.sql", "002_more.sql"]
        );
        let migrations = load_migrations(&config).unwrap();
        let versions = migrations
            .iter()
            .filter(|migration| migration.group == "main")
            .map(|migration| migration.version.as_str())
            .collect::<Vec<_>>();
        assert_eq!(versions, vec!["001", "002"]);
    }

    #[test]
    fn api_save_existing_migration_group_allows_empty_membership() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        std::fs::write(
            migration_dir.join("001_initial.sql"),
            "CREATE TABLE initial(id INTEGER);",
        )
        .unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config.migration_groups.insert(
            "premium".to_string(),
            MigrationGroupConfig::versions(vec!["001".to_string()]),
        );
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_save_migration_group(
            &state,
            br#"{"name":"premium","versions":[]}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert!(config
            .migration_groups
            .get("premium")
            .unwrap()
            .migrations
            .is_empty());
    }

    #[test]
    fn api_save_empty_migration_group_allows_missing_migrations_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_save_migration_group(
            &state,
            br#"{"name":"premium","versions":[]}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert!(config
            .migration_groups
            .get("main")
            .unwrap()
            .migrations
            .is_empty());
        assert!(config
            .migration_groups
            .get("premium")
            .unwrap()
            .migrations
            .is_empty());
        let migrations = load_migrations(&config).unwrap();
        assert!(migrations.is_empty());
    }

    #[test]
    fn api_create_migration_file_for_dir_group_writes_into_group_dir() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("legacy_migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        std::fs::write(
            migration_dir.join("001_initial.sql"),
            "CREATE TABLE initial(id INTEGER);",
        )
        .unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config.migration_groups.insert(
            "legacy".to_string(),
            MigrationGroupConfig {
                dir: Some("legacy_migrations".to_string()),
                migrations: Vec::new(),
            },
        );
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_create_migration_file(
            &state,
            br#"{"version":"005","name":"legacy_item","group":"legacy","sql":"CREATE TABLE legacy_item(id INTEGER);"}"#.to_vec(),
        )
        .unwrap();

        assert!(dir
            .path()
            .join("legacy_migrations")
            .join("005_legacy_item.sql")
            .exists());
        assert!(!dir
            .path()
            .join("migrations")
            .join("005_legacy_item.sql")
            .exists());
        let config = state.config.lock().unwrap();
        let group = config.migration_groups.get("legacy").unwrap();
        assert_eq!(group.dir.as_deref(), Some("legacy_migrations"));
        assert!(group.migrations.is_empty());
        let migrations = load_migrations(&config).unwrap();
        let versions = migrations
            .iter()
            .map(|migration| migration.version.as_str())
            .collect::<Vec<_>>();
        assert_eq!(versions, vec!["001", "005"]);
    }

    #[test]
    fn api_create_migration_file_preserves_implicit_main_group() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        std::fs::write(
            migration_dir.join("001_initial.sql"),
            "CREATE TABLE initial(id INTEGER);",
        )
        .unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_create_migration_file(
            &state,
            br#"{"version":"005","name":"main_item","group":"main","sql":"CREATE TABLE main_item(id INTEGER);"}"#.to_vec(),
        )
        .unwrap();

        assert!(migration_dir.join("005_main_item.sql").exists());
        let config = state.config.lock().unwrap();
        assert!(config.migration_groups.is_empty());
        let migrations = load_migrations(&config).unwrap();
        let versions = migrations
            .iter()
            .map(|migration| migration.version.as_str())
            .collect::<Vec<_>>();
        assert_eq!(versions, vec!["001", "005"]);
    }

    #[test]
    fn api_create_migration_file_preserves_suffix_version_filename() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_create_migration_file(
            &state,
            br#"{"version":"005","name":"suffix_item","filename":"suffix_item_005.sql","group":"main","sql":"CREATE TABLE suffix_item(id INTEGER);"}"#.to_vec(),
        )
        .unwrap();

        assert!(migration_dir.join("suffix_item_005.sql").exists());
        assert!(!migration_dir.join("005_suffix_item.sql").exists());
        let config = state.config.lock().unwrap();
        let migrations = load_migrations(&config).unwrap();
        assert_eq!(migrations[0].version, "005");
        assert_eq!(migrations[0].name, "suffix_item");
    }

    #[test]
    fn api_create_migration_file_adds_filename_to_explicit_group() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        std::fs::write(
            migration_dir.join("001_initial.sql"),
            "CREATE TABLE initial(id INTEGER);",
        )
        .unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config.migration_groups.insert(
            "premium".to_string(),
            MigrationGroupConfig::versions(vec!["001_initial.sql".to_string()]),
        );
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_create_migration_file(
            &state,
            br#"{"version":"001","name":"second","group":"premium","sql":"CREATE TABLE second(id INTEGER);"}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert_eq!(
            config
                .migration_groups
                .get("premium")
                .unwrap()
                .migrations,
            vec!["001_initial.sql", "001_second.sql"]
        );
        let migrations = load_migrations(&config).unwrap();
        assert_eq!(migrations.len(), 2);
    }

    #[test]
    fn api_update_migration_file_allows_unapplied_migration() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        let migration_path = migration_dir.join("001_initial.sql");
        std::fs::write(&migration_path, "CREATE TABLE initial(id INTEGER);").unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));
        let body = serde_json::json!({
            "path": migration_path,
            "version": "001",
            "group": "main",
            "sql": "CREATE TABLE initial(id INTEGER PRIMARY KEY);"
        });

        api_update_migration_file(&state, serde_json::to_vec(&body).unwrap()).unwrap();

        assert_eq!(
            std::fs::read_to_string(&migration_path).unwrap(),
            "CREATE TABLE initial(id INTEGER PRIMARY KEY);"
        );
    }

    #[test]
    fn api_update_migration_file_allows_parent_path_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("project with spaces");
        let migration_dir = project_dir.join("migrations");
        std::fs::create_dir_all(&migration_dir).unwrap();
        let migration_path = migration_dir.join("001_initial.sql");
        std::fs::write(&migration_path, "CREATE TABLE initial(id INTEGER);").unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(&project_dir).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, project_dir.join("sqlite-fleet.toml"));
        let body = serde_json::json!({
            "path": migration_path,
            "version": "001",
            "group": "main",
            "sql": "CREATE TABLE initial(id INTEGER PRIMARY KEY);"
        });

        api_update_migration_file(&state, serde_json::to_vec(&body).unwrap()).unwrap();

        assert_eq!(
            std::fs::read_to_string(&migration_path).unwrap(),
            "CREATE TABLE initial(id INTEGER PRIMARY KEY);"
        );
    }

    #[test]
    fn api_update_migration_file_rejects_applied_migration() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&migration_dir).unwrap();
        std::fs::create_dir(&data_dir).unwrap();
        let migration_path = migration_dir.join("001_initial.sql");
        std::fs::write(&migration_path, "CREATE TABLE initial(id INTEGER);").unwrap();
        let db_path = data_dir.join("tenant.db");
        let conn = Connection::open(&db_path).unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };
        sqlite_fleet::ensure_migrations_table(&conn, config.migrations_table()).unwrap();
        conn.execute(
            &format!(
                "INSERT INTO {} (filename, version, name, checksum, applied_at, execution_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                config.migrations_table()
            ),
            ("001_initial.sql", "001", "initial", "old", 1_i64, 1_i64),
        )
        .unwrap();
        config.validate().unwrap();
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));
        let body = serde_json::json!({
            "path": migration_path,
            "version": "001",
            "group": "main",
            "sql": "CREATE TABLE initial(id INTEGER PRIMARY KEY);"
        });

        let error = api_update_migration_file(&state, serde_json::to_vec(&body).unwrap())
            .err()
            .unwrap();

        assert!(error.to_string().contains("適用済みのため編集できません"));
        assert_eq!(
            std::fs::read_to_string(&migration_path).unwrap(),
            "CREATE TABLE initial(id INTEGER);"
        );
    }

    #[test]
    fn api_create_migration_file_requires_group_when_groups_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config.migration_groups.insert(
            "core".to_string(),
            MigrationGroupConfig::versions(vec!["001".to_string()]),
        );
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        let result = api_create_migration_file(
            &state,
            br#"{"version":"005","name":"untracked","sql":"CREATE TABLE untracked(id INTEGER);"}"#
                .to_vec(),
        );
        assert!(result.is_err());
        let error = result.err().unwrap();

        assert!(error
            .to_string()
            .contains("migration group の指定が必要です"));
        assert!(!migration_dir.join("005_untracked.sql").exists());
        let config = state.config.lock().unwrap();
        assert!(!config
            .migration_groups
            .get("core")
            .unwrap()
            .migrations
            .iter()
            .any(|version| version == "005"));
    }

    #[test]
    fn api_save_database_migration_group_allows_empty_assignment() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_save_database_migration_group(
            &state,
            br#"{"selector":"tenant","groups":[]}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert_eq!(
            config.database_migration_groups.get("tenant"),
            Some(&Vec::<String>::new())
        );
        let database = sqlite_fleet::Database {
            id: "tenant".to_string(),
            path: dir.path().join("tenant.db"),
            exists: true,
            readable: true,
        };
        assert!(config.migration_groups_for_database(&database).is_empty());
    }

    #[test]
    fn api_database_migration_assignments_preserve_matching_selector() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        let database_path = data_dir.join("tenant.db");
        Connection::open(&database_path).unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            ..Config::default()
        };
        config
            .database_migration_groups
            .insert("data/tenant.db".to_string(), vec!["main".to_string()]);
        let databases = discover_databases(&config).unwrap();

        let assignments = api_database_migration_assignments(&config, &databases);

        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].database_id, "tenant");
        assert_eq!(assignments[0].selector, "data/tenant.db");
        assert_eq!(assignments[0].migration_groups, vec!["main"]);
    }

    #[test]
    fn api_create_migration_file_removes_file_when_migration_validation_fails() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        std::fs::write(
            migration_dir.join("001_initial.sql"),
            "CREATE TABLE initial(id INTEGER);",
        )
        .unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        let result = api_create_migration_file(
            &state,
            br#"{"version":"002","name":"bad","group":"main","sql":"VACUUM;"}"#.to_vec(),
        );
        assert!(result.is_err());

        assert!(!migration_dir.join("002_bad.sql").exists());
        let config = state.config.lock().unwrap();
        assert!(config.migration_groups.is_empty());
        let migrations = load_migrations(&config).unwrap();
        let versions = migrations
            .iter()
            .map(|migration| migration.version.as_str())
            .collect::<Vec<_>>();
        assert_eq!(versions, vec!["001"]);
    }

    #[test]
    fn api_create_migration_file_accepts_absolute_migrations_dir_inside_base() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        let mut config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        config.migrations.dir = migration_dir.to_string_lossy().into_owned();
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        api_create_migration_file(
            &state,
            br#"{"version":"005","name":"absolute_dir","group":"main","sql":"CREATE TABLE absolute_dir(id INTEGER);"}"#.to_vec(),
        )
        .unwrap();

        assert!(migration_dir.join("005_absolute_dir.sql").exists());
        let config = state.config.lock().unwrap();
        let migrations = load_migrations(&config).unwrap();
        let versions = migrations
            .iter()
            .map(|migration| migration.version.as_str())
            .collect::<Vec<_>>();
        assert_eq!(versions, vec!["005"]);
    }

    #[test]
    fn gui_sql_apply_writes_success_and_failure_audit_events() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        Connection::open(data_dir.join("tenant.db")).unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            audit: sqlite_fleet::AuditConfig {
                path: Some("audit.jsonl".to_string()),
            },
            ..Config::default()
        };

        let body = r#"{"sql":"CREATE TABLE audit_items(id INTEGER PRIMARY KEY);"}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/sql?dry_run=false&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            config.clone(),
        );
        assert!(response.contains(r#""ok":true"#), "{response}");

        let body = r#"{"sql":"CREATE TABLE broken("}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/sql?dry_run=false&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            config,
        );
        assert!(response.contains(r#""ok":false"#), "{response}");

        let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert_eq!(audit.lines().count(), 2);
        assert!(audit.contains(r#""operation":"gui.sql_apply""#), "{audit}");
        assert!(audit.contains(r#""success":true"#), "{audit}");
        assert!(audit.contains(r#""success":false"#), "{audit}");
    }

    #[test]
    fn gui_migrate_writes_audit_event() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("migrations");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::create_dir(&migrations_dir).unwrap();
        Connection::open(data_dir.join("tenant.db")).unwrap();
        std::fs::write(
            migrations_dir.join("001_create_audit_items.sql"),
            "CREATE TABLE audit_items(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            migrations: sqlite_fleet::MigrationsConfig {
                dir: "migrations".to_string(),
                ..sqlite_fleet::MigrationsConfig::default()
            },
            audit: sqlite_fleet::AuditConfig {
                path: Some("audit.jsonl".to_string()),
            },
            ..Config::default()
        };

        let response = send_test_http_request_with_config(
            "POST /api/migrate?dry_run=false HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
            config,
        );

        assert!(response.contains(r#""ok":true"#), "{response}");
        let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(audit.contains(r#""operation":"gui.migrate""#), "{audit}");
    }

    #[test]
    fn gui_backup_creates_backup_and_writes_audit_event() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        Connection::open(data_dir.join("tenant.db"))
            .unwrap()
            .execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY);")
            .unwrap();
        let config = Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            backup: sqlite_fleet::BackupConfig {
                dir: "backups".to_string(),
                keep_last: 10,
                before_migrate: false,
            },
            audit: sqlite_fleet::AuditConfig {
                path: Some("audit.jsonl".to_string()),
            },
            ..Config::default()
        };

        let response = send_test_http_request_with_config(
            "POST /api/backup?database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
            config,
        );

        assert!(response.contains(r#""ok":true"#), "{response}");
        assert!(response.contains(r#""backed_up":1"#), "{response}");
        let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(audit.contains(r#""operation":"gui.backup""#), "{audit}");
        assert!(dir.path().join("backups").exists());
    }
