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
        let response = send_test_http_request_with_config(
            "GET /api/missing HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config_with_gui_user_permissions(sqlite_fleet::GuiConfig::default()),
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

        let not_found = send_test_http_request_with_config(
            "GET /api HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config_with_gui_user_permissions(sqlite_fleet::GuiConfig::default()),
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

        let not_found = send_test_http_request_with_config(
            "GET /api/ HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config_with_gui_user_permissions(sqlite_fleet::GuiConfig::default()),
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
            setup_token: "setup-token".to_string(),
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
            setup_token: "setup-token".to_string(),
            script_nonce: "nonce".to_string(),
            bind_ip: Ipv4Addr::LOCALHOST.into(),
            port: 0,
        }
    }

    fn config_with_gui_user_permissions(permissions: sqlite_fleet::GuiConfig) -> Config {
        with_gui_user_permissions(Config::default(), permissions)
    }

    fn with_gui_user_permissions(
        mut config: Config,
        permissions: sqlite_fleet::GuiConfig,
    ) -> Config {
        config.gui_users = HashMap::from([(
            "admin".to_string(),
            sqlite_fleet::GuiUserConfig {
                token: "user-token".to_string(),
                permissions,
            },
        )]);
        config
    }

    #[test]
    fn gui_post_apis_enforce_allow_flags_server_side() {
        let config = config_with_gui_user_permissions(sqlite_fleet::GuiConfig {
            allow_check: false,
            ..sqlite_fleet::GuiConfig::default()
        });
        let response = send_test_http_request_with_config(
            "POST /api/check HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI check は設定で無効化されています"));

        let config = config_with_gui_user_permissions(sqlite_fleet::GuiConfig {
            allow_migrate: false,
            ..sqlite_fleet::GuiConfig::default()
        });
        let response = send_test_http_request_with_config(
            "POST /api/migrate?dry_run=true HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI migrate は設定で無効化されています"));

        let config = config_with_gui_user_permissions(sqlite_fleet::GuiConfig {
            allow_sql_apply: false,
            ..sqlite_fleet::GuiConfig::default()
        });
        let response = send_test_http_request_with_config(
            "POST /api/sql?dry_run=false&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\nContent-Type: application/json\r\nContent-Length: 19\r\n\r\n{\"sql\":\"SELECT 1;\"}",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI SQL適用は設定で無効化されています"));

        let config = config_with_gui_user_permissions(sqlite_fleet::GuiConfig {
            allow_backup: false,
            ..sqlite_fleet::GuiConfig::default()
        });
        let response = send_test_http_request_with_config(
            "POST /api/backup HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI backup は設定で無効化されています"));

        let config = config_with_gui_user_permissions(sqlite_fleet::GuiConfig {
            allow_migration_edit: true,
            allow_gui_permission_edit: false,
            ..sqlite_fleet::GuiConfig::default()
        });
        let body = r#"{"allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/gui-permissions HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI permission edit は設定で無効化されています"));

        let config = config_with_gui_user_permissions(sqlite_fleet::GuiConfig {
            allow_migration_edit: false,
            allow_config_edit: false,
            ..sqlite_fleet::GuiConfig::default()
        });
        let response = send_test_http_request_with_config(
            "GET /api/admin/path-entries HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI config edit は設定で無効化されています"));

        let config = config_with_gui_user_permissions(sqlite_fleet::GuiConfig {
            allow_migration_edit: true,
            allow_config_edit: false,
            ..sqlite_fleet::GuiConfig::default()
        });
        let response = send_test_http_request_with_config(
            "POST /api/admin/settings HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
            config,
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI settings edit は設定で無効化されています"));
    }

    #[test]
    fn gui_user_tokens_select_effective_permissions() {
        let mut config = Config::default();
        config.gui_users = HashMap::from([
            (
                "viewer".to_string(),
                sqlite_fleet::GuiUserConfig {
                    token: "viewer-token".to_string(),
                    permissions: sqlite_fleet::GuiConfig {
                        allow_check: true,
                        ..sqlite_fleet::GuiConfig::default()
                    },
                },
            ),
            (
                "operator".to_string(),
                sqlite_fleet::GuiUserConfig {
                    token: "operator-token".to_string(),
                    permissions: sqlite_fleet::GuiConfig {
                        allow_check: true,
                        allow_migrate: true,
                        allow_gui_permission_edit: true,
                        ..sqlite_fleet::GuiConfig::default()
                    },
                },
            ),
        ]);

        let response = send_test_http_request_with_config(
            "POST /api/migrate?dry_run=true HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
            config.clone(),
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI user token が必要です"), "{response}");

        let response = send_test_http_request_with_config(
            "POST /api/migrate?dry_run=true HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: viewer-token\r\n\r\n",
            config.clone(),
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI migrate は設定で無効化されています"));

        let response = send_test_http_request_with_config(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: operator-token\r\n\r\n",
            config.clone(),
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(r#""allow_migrate":true"#), "{response}");
        assert!(
            response.contains(r#""allow_gui_permission_edit":true"#),
            "{response}"
        );

        let body = r#"{"allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true,"gui_users":[{"name":"operator","token":"operator-token","allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":false,"allow_sql_apply":false,"allow_migration_edit":false,"allow_gui_permission_edit":true,"allow_config_edit":false},{"name":"viewer","token":"viewer-token","allow_check":true,"allow_migrate":false,"allow_backup":false,"allow_restore":false,"allow_sql_apply":false,"allow_migration_edit":false,"allow_gui_permission_edit":false,"allow_config_edit":false}]}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/gui-permissions HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: operator-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            config,
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }

    #[test]
    fn api_save_gui_permissions_persists_allow_flags() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        let state = test_server_state(Config::default(), config_path.clone());

        api_save_gui_permissions(
            &state,
            br#"{"allow_check":false,"allow_migrate":true,"allow_backup":false,"allow_restore":true,"allow_sql_apply":false,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert!(!config.gui.allow_check);
        assert!(config.gui.allow_migrate);
        assert!(!config.gui.allow_backup);
        assert!(config.gui.allow_restore);
        assert!(!config.gui.allow_sql_apply);
        assert!(config.gui.allow_migration_edit);
        assert!(config.gui.allow_gui_permission_edit);
        assert!(config.gui.allow_config_edit);
        let saved = std::fs::read_to_string(config_path).unwrap();
        assert!(saved.contains("allow_check = false"), "{saved}");
        assert!(saved.contains("allow_sql_apply = false"), "{saved}");
        assert!(saved.contains("allow_gui_permission_edit = true"), "{saved}");
        assert!(saved.contains("allow_config_edit = true"), "{saved}");
    }

    #[test]
    fn api_state_omits_gui_users_in_plain_permission_mode() {
        let state = api_state(&Config::default(), &sqlite_fleet::GuiConfig::default());
        let json = serde_json::to_string(&state).unwrap();

        assert!(!json.contains("gui_users"), "{json}");
        assert!(json.contains(r#""gui_user_setup_available":true"#), "{json}");
    }

    #[test]
    fn http_save_gui_permissions_without_users_requires_initial_user_setup() {
        let body = r#"{"allow_check":false,"allow_migrate":true,"allow_backup":false,"allow_restore":true,"allow_sql_apply":false,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/gui-permissions HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            Config {
                gui: sqlite_fleet::GuiConfig {
                    allow_gui_permission_edit: true,
                    ..sqlite_fleet::GuiConfig::default()
                },
                ..Config::default()
            },
        );

        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(
            response.contains("GUI user を作成するまで、この操作は利用できません"),
            "{response}"
        );
    }

    #[test]
    fn initial_setup_blocks_non_setup_apis() {
        let response = send_test_http_request(
            "GET /api/discover HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
        );
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(
            response.contains("GUI user を作成するまで、この操作は利用できません"),
            "{response}"
        );

        let response = send_test_http_request(
            "GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\n\r\n",
        );
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(r#""gui_user_setup_available":true"#), "{response}");
        assert!(response.contains(r#""database_count":0"#), "{response}");
    }

    #[test]
    fn api_save_gui_permissions_updates_user_permission_mode() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        let state = test_server_state(
            Config {
                gui_users: HashMap::from([(
                    "admin".to_string(),
                    sqlite_fleet::GuiUserConfig {
                        token: "admin-token".to_string(),
                        permissions: sqlite_fleet::GuiConfig {
                            allow_gui_permission_edit: true,
                            ..sqlite_fleet::GuiConfig::default()
                        },
                    },
                )]),
                ..Config::default()
            },
            config_path,
        );

        api_save_gui_permissions(
            &state,
            br#"{"allow_check":false,"allow_migrate":true,"allow_backup":false,"allow_restore":true,"allow_sql_apply":false,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true,"gui_users":[{"name":"admin","token":"admin-token","allow_check":true,"allow_migrate":true,"allow_backup":false,"allow_restore":false,"allow_sql_apply":false,"allow_migration_edit":false,"allow_gui_permission_edit":true,"allow_config_edit":false},{"name":"viewer","token":"viewer-token","allow_check":true,"allow_migrate":false,"allow_backup":false,"allow_restore":false,"allow_sql_apply":false,"allow_migration_edit":false,"allow_gui_permission_edit":false,"allow_config_edit":false}]}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert_eq!(config.gui_users.len(), 2);
        assert!(config.gui_users["admin"].permissions.allow_migrate);
        assert!(!config.gui_users["viewer"].permissions.allow_migrate);
    }

    #[test]
    fn api_save_gui_permissions_rejects_empty_user_list() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        let state = test_server_state(
            Config {
                gui_users: HashMap::from([(
                    "admin".to_string(),
                    sqlite_fleet::GuiUserConfig {
                        token: "admin-token".to_string(),
                        permissions: sqlite_fleet::GuiConfig {
                            allow_gui_permission_edit: true,
                            ..sqlite_fleet::GuiConfig::default()
                        },
                    },
                )]),
                ..Config::default()
            },
            config_path,
        );

        let error = match api_save_gui_permissions(
            &state,
            br#"{"allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true,"gui_users":[]}"#.to_vec(),
        ) {
            Ok(_) => panic!("empty GUI user list must be rejected"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("GUI user は1人以上必要です"));
        assert_eq!(state.config.lock().unwrap().gui_users.len(), 1);
    }

    #[test]
    fn gui_allows_initial_user_creation_with_setup_token() {
        let body = r#"{"allow_check":true,"allow_migrate":false,"allow_backup":false,"allow_restore":false,"allow_sql_apply":false,"allow_migration_edit":false,"allow_gui_permission_edit":false,"allow_config_edit":false,"gui_users":[{"name":"owner","token":"owner-token","allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true}]}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/gui-permissions HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-Setup-Token: setup-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            Config::default(),
        );

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }

    #[test]
    fn gui_rejects_initial_user_creation_without_setup_token() {
        let body = r#"{"allow_check":true,"allow_migrate":false,"allow_backup":false,"allow_restore":false,"allow_sql_apply":false,"allow_migration_edit":false,"allow_gui_permission_edit":false,"allow_config_edit":false,"gui_users":[{"name":"owner","token":"owner-token","allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true}]}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/gui-permissions HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            Config::default(),
        );

        assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
        assert!(response.contains("GUI initial user setup token が不正です"));
    }

    #[test]
    fn gui_rejects_null_gui_users_during_initial_setup() {
        let body = r#"{"allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true,"gui_users":null}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/gui-permissions HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-Setup-Token: setup-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            Config::default(),
        );

        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains(r#""ok":false"#), "{response}");
        assert!(response.contains("gui_users は配列で指定してください"), "{response}");
    }

    #[test]
    fn api_save_settings_persists_discovery_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("db/migrate");
        let backups_dir = dir.path().join("safe/backups");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&migrations_dir).unwrap();
        std::fs::create_dir_all(&backups_dir).unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        let state = test_server_state(
            Config {
                base_dir: std::fs::canonicalize(dir.path()).unwrap(),
                ..Config::default()
            },
            config_path.clone(),
        );

        api_save_settings(
            &state,
            br#"{"project_name":"Existing App","discovery":"glob","databases_path_glob":"data/*.db","databases_source":null,"databases_query":null,"databases_id_column":null,"databases_path_column":null,"databases_path_template":null,"migrations_dir":"db/migrate","migrations_table":"schema_migrations","report_format":"json","report_path":"reports/status.json","backup_dir":"safe/backups","backup_before_migrate":true,"backup_keep_last":0,"audit_path":"audit.jsonl","parallel":2,"lock_timeout_ms":0,"continue_on_error":true}"#.to_vec(),
        )
        .unwrap();

        let config = state.config.lock().unwrap();
        assert_eq!(config.project.name.as_deref(), Some("Existing App"));
        assert_eq!(config.databases.path_glob.as_deref(), Some("data/*.db"));
        assert_eq!(config.migrations.dir, "db/migrate");
        assert_eq!(config.migrations.table, "schema_migrations");
        assert_eq!(config.backup.dir, "safe/backups");
        assert!(config.backup.before_migrate);
        assert_eq!(config.backup.keep_last, 0);
        assert_eq!(config.execution.parallel, 2);
        assert_eq!(config.execution.lock_timeout_ms, 0);
        let saved = std::fs::read_to_string(config_path).unwrap();
        assert!(saved.contains("name = \"Existing App\""), "{saved}");
        assert!(saved.contains("dir = \"db/migrate\""), "{saved}");
    }

    #[test]
    fn api_save_settings_persists_allowed_roots() {
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("migrations")).unwrap();
        std::fs::create_dir_all(dir.path().join("backups")).unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        let state = test_server_state(
            Config {
                base_dir: std::fs::canonicalize(dir.path()).unwrap(),
                ..Config::default()
            },
            config_path.clone(),
        );
        let body = format!(
            r#"{{"project_name":null,"allowed_roots":[".","{}"],"discovery":"glob","databases_path_glob":"data/*.db","databases_source":null,"databases_query":null,"databases_id_column":null,"databases_path_column":null,"databases_path_template":null,"migrations_dir":"migrations","migrations_table":"_sqlite_fleet_migrations","report_format":"json","report_path":null,"backup_dir":"backups","backup_before_migrate":false,"backup_keep_last":10,"audit_path":null,"parallel":1,"lock_timeout_ms":0,"continue_on_error":false}}"#,
            external.path().display()
        );

        api_save_settings(&state, body.into_bytes()).unwrap();

        let config = state.config.lock().unwrap();
        assert_eq!(config.security.allowed_roots.len(), 2);
        assert_eq!(
            config.security.allowed_roots[1],
            external.path().display().to_string()
        );
        let saved = std::fs::read_to_string(config_path).unwrap();
        assert!(saved.contains("[security]"), "{saved}");
        assert!(saved.contains("allowed_roots"), "{saved}");
    }

    #[test]
    fn api_preview_discovery_uses_unsaved_allowed_roots() {
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        Connection::open(external.path().join("tenant.db")).unwrap();
        let state = test_server_state(
            Config {
                base_dir: std::fs::canonicalize(dir.path()).unwrap(),
                ..Config::default()
            },
            dir.path().join("sqlite-fleet.toml"),
        );
        let body = format!(
            r#"{{"project_name":null,"allowed_roots":[".","{}"],"discovery":"glob","databases_path_glob":"{}","databases_source":null,"databases_query":null,"databases_id_column":null,"databases_path_column":null,"databases_path_template":null,"migrations_dir":"migrations","migrations_table":"_sqlite_fleet_migrations","report_format":"json","report_path":null,"backup_dir":"backups","backup_before_migrate":false,"backup_keep_last":10,"audit_path":null,"parallel":1,"lock_timeout_ms":0,"continue_on_error":false}}"#,
            external.path().display(),
            external.path().join("*.db").display()
        );

        let preview = api_preview_discovery(&state, body.into_bytes()).unwrap();

        assert_eq!(preview.count, 1);
        assert!(preview.errors.is_empty(), "{:?}", preview.errors);
        assert_eq!(preview.databases[0].id, "tenant");
        assert!(preview.databases[0].allowed_root.is_some());
    }

    #[test]
    fn allowed_root_data_warns_for_broad_roots() {
        let config = Config {
            security: sqlite_fleet::SecurityConfig {
                allowed_roots: vec!["/".to_string()],
            },
            ..Config::default()
        };

        let roots = allowed_root_data(&config);

        assert!(roots[0]
            .warnings
            .iter()
            .any(|warning| warning.contains("broad")));
    }

    #[test]
    fn api_path_entries_lists_base_relative_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data/nested")).unwrap();
        std::fs::write(dir.path().join("data/tenant.db"), "").unwrap();
        std::fs::write(dir.path().join(".hidden"), "").unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };

        let root = api_path_entries(&config, None).unwrap();
        assert_eq!(root.current, "");
        assert!(root.entries.iter().any(|entry| entry.path == "data" && entry.kind == "dir"));
        assert!(!root.entries.iter().any(|entry| entry.name == ".hidden"));

        let data = api_path_entries(&config, Some("data")).unwrap();
        assert_eq!(data.current, "data");
        assert_eq!(data.parent.as_deref(), Some(""));
        assert!(data
            .entries
            .iter()
            .any(|entry| entry.path == "data/tenant.db"
                && entry.kind == "file"
                && entry.modified_at_ms.is_some()));
    }

    #[test]
    fn api_path_entries_rejects_outside_or_glob_paths() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };

        assert!(api_path_entries(&config, Some("../")).is_err());
        assert!(api_path_entries(&config, Some("data/**/*.db")).is_err());
    }

    #[test]
    fn api_baseline_migrations_marks_pending_without_running_sql() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("migrations");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&migrations_dir).unwrap();
        Connection::open(data_dir.join("tenant.db")).unwrap();
        std::fs::write(
            migrations_dir.join("001_create_items.sql"),
            "CREATE TABLE items(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        let state = test_server_state(
            Config {
                base_dir: std::fs::canonicalize(dir.path()).unwrap(),
                databases: sqlite_fleet::DatabasesConfig {
                    discovery: "glob".to_string(),
                    path_glob: Some("data/*.db".to_string()),
                    ..Default::default()
                },
                ..Config::default()
            },
            dir.path().join("sqlite-fleet.toml"),
        );

        let report = api_baseline_migrations(&state, br#"{"databases":["tenant"]}"#.to_vec()).unwrap();
        assert_eq!(report.failed_databases, 0);
        assert_eq!(report.applied_databases, 1);

        let conn = Connection::open(data_dir.join("tenant.db")).unwrap();
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='items'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_exists, 0);
        let history_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _sqlite_fleet_migrations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(history_count, 1);
    }

    #[test]
    fn baseline_rechecks_pending_after_acquiring_operation_lock() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("migrations");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&migrations_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path).unwrap();
        std::fs::write(
            migrations_dir.join("001_create_items.sql"),
            "CREATE TABLE items(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..Default::default()
            },
            ..Config::default()
        };
        let databases = discover_databases(&config).unwrap();
        let migrations = load_migrations(&config).unwrap();
        let stale_plan = sqlite_fleet::build_plan(&config, &databases, &migrations)
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(stale_plan.pending.len(), 1);
        let conn = Connection::open(&db_path).unwrap();
        ensure_migrations_table(&conn, "_sqlite_fleet_migrations").unwrap();
        conn.execute(
            "INSERT INTO _sqlite_fleet_migrations (filename, version, name, checksum, applied_at, execution_ms) VALUES (?1, ?2, ?3, ?4, 1, 0)",
            rusqlite::params![
                migrations[0].filename,
                migrations[0].version,
                migrations[0].name,
                migrations[0].checksum,
            ],
        )
        .unwrap();

        let result = baseline_database(&config, stale_plan, &migrations);

        assert!(result.success, "{:?}", result.error);
        assert!(result.applied.is_empty());
        assert!(result.pending.is_empty());
        let history_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _sqlite_fleet_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(history_count, 1);
    }

    #[test]
    fn api_baseline_migrations_respects_existing_database_operation_lock() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("migrations");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&migrations_dir).unwrap();
        let db_path = data_dir.join("tenant.db");
        Connection::open(&db_path).unwrap();
        std::fs::write(
            migrations_dir.join("001_create_items.sql"),
            "CREATE TABLE items(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..Default::default()
            },
            execution: sqlite_fleet::ExecutionConfig {
                lock_timeout_ms: 1,
                ..Default::default()
            },
            ..Config::default()
        };
        let _lock = sqlite_fleet::acquire_database_operation_lock(&config, &db_path, "test")
            .expect("test lock should be acquired");
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        let report =
            api_baseline_migrations(&state, br#"{"databases":["tenant"]}"#.to_vec()).unwrap();

        assert_eq!(report.database_count, 1);
        assert_eq!(report.processed_databases, 1);
        assert_eq!(report.failed_databases, 1);
        assert!(report.databases[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("別のsqlite-fleet操作中")));
        let conn = Connection::open(&db_path).unwrap();
        let history_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlite_fleet_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(history_exists, 0);
    }

    #[test]
    fn api_baseline_migrations_upgrades_legacy_history_before_inserting_pending() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("migrations");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&migrations_dir).unwrap();
        std::fs::write(
            migrations_dir.join("001_create_items.sql"),
            "CREATE TABLE items(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        std::fs::write(
            migrations_dir.join("002_add_items_name.sql"),
            "ALTER TABLE items ADD COLUMN name TEXT;",
        )
        .unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..Default::default()
            },
            ..Config::default()
        };
        let migrations = load_migrations(&config).unwrap();
        let first = migrations
            .iter()
            .find(|migration| migration.filename == "001_create_items.sql")
            .unwrap();
        let db_path = data_dir.join("tenant.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY);")
            .unwrap();
        conn.execute(
            "CREATE TABLE _sqlite_fleet_migrations (
                version TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                checksum TEXT NOT NULL,
                applied_at INTEGER NOT NULL,
                execution_ms INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO _sqlite_fleet_migrations (version, name, checksum, applied_at, execution_ms)
             VALUES (?1, ?2, ?3, 1, 1)",
            rusqlite::params![first.version, first.name, first.checksum],
        )
        .unwrap();
        drop(conn);
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        let report =
            api_baseline_migrations(&state, br#"{"databases":["tenant"]}"#.to_vec()).unwrap();

        assert_eq!(report.failed_databases, 0);
        assert_eq!(report.applied_databases, 1);
        let conn = Connection::open(&db_path).unwrap();
        let applied =
            sqlite_fleet::read_applied_migrations(&conn, "_sqlite_fleet_migrations").unwrap();
        let filenames = applied
            .iter()
            .map(|migration| migration.filename.as_str())
            .collect::<Vec<_>>();
        assert_eq!(filenames, ["001_create_items.sql", "002_add_items_name.sql"]);
    }

    #[test]
    fn api_baseline_migrations_stops_on_first_failure_when_continue_on_error_is_false() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("migrations");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&migrations_dir).unwrap();
        std::fs::write(
            migrations_dir.join("001_create_items.sql"),
            "CREATE TABLE items(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        let first = Connection::open(data_dir.join("a.db")).unwrap();
        ensure_migrations_table(&first, "_sqlite_fleet_migrations").unwrap();
        first
            .execute(
                "INSERT INTO _sqlite_fleet_migrations (filename, version, name, checksum, applied_at, execution_ms) VALUES (?1, ?2, ?3, ?4, 1, 0)",
                rusqlite::params!["001_create_items.sql", "001", "create_items", "wrong-checksum"],
            )
            .unwrap();
        Connection::open(data_dir.join("b.db")).unwrap();
        let state = test_server_state(
            Config {
                base_dir: std::fs::canonicalize(dir.path()).unwrap(),
                databases: sqlite_fleet::DatabasesConfig {
                    discovery: "glob".to_string(),
                    path_glob: Some("data/*.db".to_string()),
                    ..Default::default()
                },
                execution: sqlite_fleet::ExecutionConfig {
                    continue_on_error: false,
                    ..Default::default()
                },
                ..Config::default()
            },
            dir.path().join("sqlite-fleet.toml"),
        );

        let report =
            api_baseline_migrations(&state, br#"{"databases":["a","b"]}"#.to_vec()).unwrap();

        assert_eq!(report.database_count, 2);
        assert_eq!(report.processed_databases, 1);
        assert_eq!(report.failed_databases, 1);
        assert_eq!(report.applied_databases, 0);
        assert_eq!(report.databases[0].database.id, "a");
        let second = Connection::open(data_dir.join("b.db")).unwrap();
        let history_exists: i64 = second
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlite_fleet_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(history_exists, 0);
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
            .contains("マイグレーショングループの指定が必要です"));
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

    #[cfg(unix)]
    #[test]
    fn api_create_migration_file_rejects_final_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let migration_dir = dir.path().join("migrations");
        std::fs::create_dir(&migration_dir).unwrap();
        let outside = dir.path().join("outside.sql");
        std::os::unix::fs::symlink(&outside, migration_dir.join("005_symlinked.sql")).unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        let result = api_create_migration_file(
            &state,
            br#"{"version":"005","name":"symlinked","group":"main","sql":"CREATE TABLE symlinked(id INTEGER);"}"#.to_vec(),
        );

        assert!(result.is_err());
        assert!(!outside.exists());
    }

    #[cfg(unix)]
    #[test]
    fn api_create_database_file_rejects_final_symlink() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("data")).unwrap();
        let outside = dir.path().join("outside.db");
        std::os::unix::fs::symlink(&outside, dir.path().join("data/new.db")).unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, dir.path().join("sqlite-fleet.toml"));

        let result =
            api_create_database_file(&state, br#"{"path":"data/new.db","db_group":null}"#.to_vec());

        assert!(result.is_err());
        assert!(!outside.exists());
    }

    #[cfg(unix)]
    #[test]
    fn persist_config_does_not_follow_fixed_tmp_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("sqlite-fleet.toml");
        let fixed_tmp = dir.path().join("sqlite-fleet.toml.tmp");
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, "original").unwrap();
        std::os::unix::fs::symlink(&outside, &fixed_tmp).unwrap();
        let config = Config {
            base_dir: std::fs::canonicalize(dir.path()).unwrap(),
            ..Config::default()
        };
        let state = test_server_state(config, config_path);

        api_save_gui_permissions(
            &state,
            br#"{"allow_check":true,"allow_migrate":true,"allow_backup":true,"allow_restore":true,"allow_sql_apply":true,"allow_migration_edit":true,"allow_gui_permission_edit":true,"allow_config_edit":true}"#.to_vec(),
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(outside).unwrap(), "original");
        assert!(std::fs::symlink_metadata(fixed_tmp)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn gui_sql_apply_writes_success_and_failure_audit_events() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir(&data_dir).unwrap();
        Connection::open(data_dir.join("tenant.db")).unwrap();
        let config = with_gui_user_permissions(Config {
            base_dir: dir.path().to_path_buf(),
            databases: sqlite_fleet::DatabasesConfig {
                discovery: "glob".to_string(),
                path_glob: Some("data/*.db".to_string()),
                ..sqlite_fleet::DatabasesConfig::default()
            },
            audit: sqlite_fleet::AuditConfig {
                path: Some("audit.jsonl".to_string()),
            },
            gui: sqlite_fleet::GuiConfig {
                allow_sql_apply: true,
                ..sqlite_fleet::GuiConfig::default()
            },
            ..Config::default()
        }, sqlite_fleet::GuiConfig {
            allow_sql_apply: true,
            ..sqlite_fleet::GuiConfig::default()
        });

        let body = r#"{"sql":"CREATE TABLE audit_items(id INTEGER PRIMARY KEY);"}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/sql?dry_run=false&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            config.clone(),
        );
        assert!(response.contains(r#""ok":true"#), "{response}");

        let body = r#"{"sql":"CREATE TABLE broken("}"#;
        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/sql?dry_run=false&database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
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
        let config = with_gui_user_permissions(Config {
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
            gui: sqlite_fleet::GuiConfig {
                allow_migrate: true,
                ..sqlite_fleet::GuiConfig::default()
            },
            ..Config::default()
        }, sqlite_fleet::GuiConfig {
            allow_migrate: true,
            ..sqlite_fleet::GuiConfig::default()
        });

        let response = send_test_http_request_with_config(
            "POST /api/migrate?dry_run=false HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config,
        );

        assert!(response.contains(r#""ok":true"#), "{response}");
        let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(audit.contains(r#""operation":"gui.migrate""#), "{audit}");
    }

    #[test]
    fn gui_baseline_writes_report_and_audit_event() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let migrations_dir = dir.path().join("migrations");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::create_dir(&migrations_dir).unwrap();
        Connection::open(data_dir.join("tenant.db")).unwrap();
        std::fs::write(
            migrations_dir.join("001_create_baseline_items.sql"),
            "CREATE TABLE baseline_items(id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        let config = with_gui_user_permissions(Config {
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
            report: sqlite_fleet::ReportConfig {
                format: "json".to_string(),
                path: Some("baseline-report.json".to_string()),
            },
            audit: sqlite_fleet::AuditConfig {
                path: Some("audit.jsonl".to_string()),
            },
            gui: sqlite_fleet::GuiConfig {
                allow_migrate: true,
                ..sqlite_fleet::GuiConfig::default()
            },
            ..Config::default()
        }, sqlite_fleet::GuiConfig {
            allow_migrate: true,
            ..sqlite_fleet::GuiConfig::default()
        });
        let body = r#"{"databases":["tenant"]}"#;

        let response = send_test_http_request_with_config(
            &format!(
                "POST /api/admin/baseline HTTP/1.1\r\nHost: 127.0.0.1:{{port}}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            ),
            config,
        );

        assert!(response.contains(r#""ok":true"#), "{response}");
        let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(audit.contains(r#""operation":"gui.baseline""#), "{audit}");
        let report = std::fs::read_to_string(dir.path().join("baseline-report.json")).unwrap();
        assert!(report.contains(r#""applied_databases": 1"#), "{report}");
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
        let config = with_gui_user_permissions(Config {
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
            gui: sqlite_fleet::GuiConfig {
                allow_backup: true,
                ..sqlite_fleet::GuiConfig::default()
            },
            ..Config::default()
        }, sqlite_fleet::GuiConfig {
            allow_backup: true,
            ..sqlite_fleet::GuiConfig::default()
        });

        let response = send_test_http_request_with_config(
            "POST /api/backup?database=tenant HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SQLite-Fleet-Token: token\r\nX-SQLite-Fleet-User-Token: user-token\r\n\r\n",
            config,
        );

        assert!(response.contains(r#""ok":true"#), "{response}");
        assert!(response.contains(r#""backed_up":1"#), "{response}");
        let audit = std::fs::read_to_string(dir.path().join("audit.jsonl")).unwrap();
        assert!(audit.contains(r#""operation":"gui.backup""#), "{audit}");
        assert!(dir.path().join("backups").exists());
    }
