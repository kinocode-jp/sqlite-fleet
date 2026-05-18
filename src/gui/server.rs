struct HttpRequest {
    head: String,
    initial_body: Vec<u8>,
}

struct ServerState {
    config: Mutex<Config>,
    config_path: PathBuf,
    csrf_token: String,
    setup_token: String,
    script_nonce: String,
    bind_ip: IpAddr,
    port: u16,
    allow_remote_host: bool,
}

pub struct GuiAccessOptions {
    pub ssh_user: Option<String>,
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub local_port: Option<u16>,
}

impl GuiAccessOptions {
    pub fn validate(&self) -> Result<()> {
        if let Some(user) = &self.ssh_user {
            validate_ssh_hint_token("--ssh-user", user, |c| {
                c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
            })?;
        }
        if let Some(host) = &self.ssh_host {
            validate_ssh_hint_token("--ssh-host", host, |c| {
                c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']')
            })?;
        }
        if self.ssh_port == Some(0) {
            bail!("--ssh-port は1以上が必要です");
        }
        if self.local_port == Some(0) {
            bail!("--local-port は1以上が必要です");
        }
        Ok(())
    }
}

pub fn serve(
    config: Config,
    config_path: PathBuf,
    host: &str,
    port: u16,
    allow_remote: bool,
    access_options: GuiAccessOptions,
) -> Result<()> {
    access_options.validate()?;
    validate_gui_host(host, allow_remote)?;
    let listener = TcpListener::bind((host, port))
        .with_context(|| format!("GUIサーバを起動できません: {host}:{port}"))?;
    let addr = listener.local_addr()?;
    let allow_remote_host = allow_remote && !addr.ip().is_loopback();
    let state = ServerState {
        config: Mutex::new(config),
        config_path,
        csrf_token: generate_csrf_token()?,
        setup_token: generate_csrf_token()?,
        script_nonce: generate_csrf_token()?,
        bind_ip: addr.ip(),
        port: addr.port(),
        allow_remote_host,
    };
    print_gui_access_help(&addr, &access_options);
    println!("停止するには Ctrl+C を押してください");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_connection(stream, &state) {
                    eprintln!("GUIリクエスト処理に失敗しました: {error}");
                }
            }
            Err(error) => eprintln!("GUI接続を受け付けられません: {error}"),
        }
    }
    Ok(())
}

fn validate_ssh_hint_token<F>(name: &str, value: &str, allowed: F) -> Result<()>
where
    F: Fn(char) -> bool,
{
    if value.is_empty() {
        bail!("{name} は空にできません");
    }
    if !value.chars().all(allowed) {
        bail!("{name} に使用できない文字が含まれています");
    }
    Ok(())
}

fn print_gui_access_help(addr: &std::net::SocketAddr, options: &GuiAccessOptions) {
    let listen_url = format!("http://{}", addr);
    println!("GUI listening on {listen_url}");

    if addr.ip().is_loopback() {
        let local_port = options.local_port.unwrap_or(addr.port());
        let local_url = format!("http://127.0.0.1:{local_port}");
        println!();
        println!("If this is running on a remote server, run this on your local machine:");
        println!();
        println!(
            "  {}",
            build_ssh_tunnel_command(addr.port(), local_port, options)
        );
        println!();
        println!("Then open:");
        println!();
        println!("  {local_url}");
    } else {
        println!();
        println!("WARNING: GUI is bound to a non-loopback address.");
        println!("This can expose database administration controls to the network.");
        println!("Use this only behind trusted network controls and GUI user authentication.");
    }
}

fn build_ssh_tunnel_command(remote_port: u16, local_port: u16, options: &GuiAccessOptions) -> String {
    let destination = match (&options.ssh_user, &options.ssh_host) {
        (Some(user), Some(host)) => format!("{user}@{host}"),
        (Some(user), None) => format!("{user}@<server>"),
        (None, Some(host)) => format!("<user>@{host}"),
        (None, None) => "<user>@<server>".to_string(),
    };
    let port = options
        .ssh_port
        .map(|port| format!(" -p {port}"))
        .unwrap_or_default();
    format!(
        "ssh{port} -N -L 127.0.0.1:{local_port}:127.0.0.1:{remote_port} {destination}"
    )
}

fn handle_connection(mut stream: TcpStream, state: &ServerState) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let request = match read_http_request(&mut stream) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(()),
        Err(error) => return write_json_error(&mut stream, 400, error),
    };
    if let Err(error) = validate_crlf_lines(&request.head) {
        return write_json_error(&mut stream, 400, error);
    }
    let (method, target) = match request.head.lines().next().map(parse_request_line) {
        Some(Ok(parsed)) => parsed,
        Some(Err(error)) => return write_json_error(&mut stream, 400, error),
        None => {
            return write_json_error(&mut stream, 400, anyhow::anyhow!("request line が必要です"))
        }
    };
    let (path, query) = split_target(target);
    let headers = match parse_headers(&request.head) {
        Ok(headers) => headers,
        Err(error) => return write_json_error(&mut stream, 400, error),
    };
    if let Err(error) =
        validate_host_header(&headers, state.bind_ip, state.port, state.allow_remote_host)
    {
        return write_json_error(&mut stream, 403, error);
    }
    if is_api_path(path) {
        if let Err(error) = validate_api_token(&headers, &state.csrf_token) {
            return write_json_error(&mut stream, 403, error);
        }
    }
    let mut request_body = None;
    if method == "POST" && requires_json_body(path) {
        if let Err(error) = validate_json_content_type(&headers) {
            return write_json_error(&mut stream, 400, error);
        }
        match read_request_body(&mut stream, &headers, request.initial_body) {
            Ok(body) => request_body = Some(body),
            Err(error) => return write_json_error(&mut stream, 400, error),
        }
    } else if let Err(error) = validate_no_request_body(&headers, !request.initial_body.is_empty())
    {
        return write_json_error(&mut stream, 400, error);
    }
    let config = match state.config.lock() {
        Ok(config) => config.clone(),
        Err(_) => {
            return write_json_error(
                &mut stream,
                500,
                anyhow::anyhow!("GUI設定状態が壊れています"),
            )
        }
    };
    let permissions = if is_api_path(path) {
        match config.effective_gui_permissions(
            headers
                .get("x-sqlite-fleet-user-token")
                .map(String::as_str),
        ) {
            Ok(permissions) => permissions,
            Err(error) => return write_json_error(&mut stream, 403, error),
        }
    } else {
        config.gui.clone()
    };
    if is_api_path(path) && config.gui_users.is_empty() && !initial_setup_api_allowed(method, path)
    {
        return write_json_error(
            &mut stream,
            403,
            anyhow::anyhow!("GUI user を作成するまで、この操作は利用できません"),
        );
    }

    match (method, path) {
        ("GET", "/") => {
            let body = INDEX_HTML
                .replace("__CSRF_TOKEN__", &state.csrf_token)
                .replace("__SETUP_TOKEN__", &state.setup_token)
                .replace("__SCRIPT_NONCE__", &state.script_nonce);
            write_response(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                &body,
                Some(&state.script_nonce),
                Some(&state.script_nonce),
            )
        }
        ("GET", "/api/state") => {
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json(&mut stream, 200, &api_state(&config, &permissions))
        }
        ("GET", "/api/discover") => {
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(&mut stream, discover_databases(&config))
        }
        ("GET", "/api/plan") => {
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json(&mut stream, 200, &api_plan(&config))
        }
        ("GET", "/api/admin/path-entries") => {
            if !permissions.allow_config_edit && !permissions.allow_migration_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI config edit は設定で無効化されています"),
                );
            }
            let query = match parse_query(query) {
                Ok(query) => query,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            if let Err(error) = validate_query_keys(&query, &["dir"]) {
                return write_json_error(&mut stream, 400, error);
            }
            let dir = match optional_nonempty_query(&query, "dir") {
                Ok(dir) => dir,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            write_json_result(&mut stream, api_path_entries(&config, dir))
        }
        ("GET", "/api/schema") => {
            let query = match parse_query(query) {
                Ok(query) => query,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            if let Err(error) = validate_query_keys(&query, &["database"]) {
                return write_json_error(&mut stream, 400, error);
            }
            let database = match optional_nonempty_query(&query, "database") {
                Ok(Some(database)) => database,
                Ok(None) => {
                    return write_json_error(
                        &mut stream,
                        400,
                        anyhow::anyhow!("query parameter database が必要です"),
                    )
                }
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            write_json_result(&mut stream, api_schema(&config, database))
        }
        ("POST", "/api/check") => {
            if !permissions.allow_check {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI check は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json(&mut stream, 200, &api_check(&config))
        }
        ("POST", "/api/backup") => {
            if !permissions.allow_backup {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI backup は設定で無効化されています"),
                );
            }
            let query = match parse_query(query) {
                Ok(query) => query,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            if let Err(error) = validate_query_keys(&query, &["database", "group", "limit"]) {
                return write_json_error(&mut stream, 400, error);
            }
            let database = match optional_nonempty_query(&query, "database") {
                Ok(database) => database,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let group = match optional_nonempty_query(&query, "group") {
                Ok(group) => group,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let limit = match optional_usize_query(&query, "limit") {
                Ok(limit) => limit,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let result = backup(
                &config,
                DatabaseSelection {
                    database: database.map(str::to_string),
                    group: group.map(str::to_string),
                    limit,
                },
            )
            .and_then(|report| {
                write_audit_event(&config, "gui.backup", &report)?;
                Ok(report)
            });
            write_json_result(&mut stream, result)
        }
        ("POST", "/api/sql") => {
            let query = match parse_query(query) {
                Ok(query) => query,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let dry_run = match required_bool_query(&query, "dry_run") {
                Ok(dry_run) => dry_run,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            if !dry_run && !permissions.allow_sql_apply {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI SQL適用は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_query_keys(&query, &["dry_run", "database"]) {
                return write_json_error(&mut stream, 400, error);
            }
            let database = match optional_nonempty_query(&query, "database") {
                Ok(Some(database)) => database,
                Ok(None) => {
                    return write_json_error(
                        &mut stream,
                        400,
                        anyhow::anyhow!("query parameter database が必要です"),
                    )
                }
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let body = request_body.unwrap_or_default();
            let result = api_sql(&config, database, dry_run, &body);
            let result = if dry_run {
                result
            } else {
                match result {
                    Ok(sql_result) => write_audit_event(
                        &config,
                        "gui.sql_apply",
                        &serde_json::json!({
                            "success": true,
                            "result": sql_result,
                        }),
                    )
                    .map(|()| sql_result),
                    Err(error) => {
                        let message = error.to_string();
                        match write_audit_event(
                            &config,
                            "gui.sql_apply",
                            &serde_json::json!({
                                "success": false,
                                "database": database,
                                "error": message,
                            }),
                        ) {
                            Ok(()) => Err(error),
                            Err(audit_error) => Err(audit_error),
                        }
                    }
                }
            };
            write_json_result(&mut stream, result)
        }
        ("POST", "/api/admin/gui-permissions") => {
            let body = request_body.unwrap_or_default();
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            let request = match parse_gui_permission_request(&body) {
                Ok(request) => request,
                Err(error) => return write_json_result::<AdminResult>(&mut stream, Err(error)),
            };
            let initial_user_create = config.gui_users.is_empty() && request.gui_users.is_some();
            if config.gui_users.is_empty() {
                if initial_user_create {
                    if let Err(error) = validate_setup_token(&headers, &state.setup_token) {
                        return write_json_error(&mut stream, 403, error);
                    }
                } else {
                    return write_json_error(
                        &mut stream,
                        403,
                        anyhow::anyhow!("GUI user を作成するまで、この操作は利用できません"),
                    );
                }
            } else if !permissions.allow_gui_permission_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI permission edit は設定で無効化されています"),
                );
            }
            write_json_result(&mut stream, api_save_gui_permissions_request(state, request))
        }
        ("POST", "/api/admin/settings") => {
            if !permissions.allow_config_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI settings edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_save_settings(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/admin/discovery-preview") => {
            if !permissions.allow_config_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI settings edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_preview_discovery(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/admin/baseline") => {
            if !permissions.allow_migrate {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migrate は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            let result =
                api_baseline_migrations(state, request_body.unwrap_or_default()).and_then(
                    |report| {
                        let report_write_error = write_report_json(&config, &report).err();
                        if let Some(error) = report_write_error {
                            Err(error)
                        } else {
                            write_audit_event(&config, "gui.baseline", &report)?;
                            Ok(report)
                        }
                    },
                );
            write_json_result(&mut stream, result)
        }
        ("POST", "/api/admin/migration-group") => {
            if !permissions.allow_migration_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migration edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_save_migration_group(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/admin/db-group") => {
            if !permissions.allow_migration_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migration edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_save_db_group(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/admin/database-migration-group") => {
            if !permissions.allow_migration_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migration edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_save_database_migration_group(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/admin/migration-file") => {
            if !permissions.allow_migration_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migration edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_create_migration_file(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/admin/migration-file/update") => {
            if !permissions.allow_migration_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migration edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_update_migration_file(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/admin/database-file") => {
            if !permissions.allow_migration_edit {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migration edit は設定で無効化されています"),
                );
            }
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(
                &mut stream,
                api_create_database_file(state, request_body.unwrap_or_default()),
            )
        }
        ("POST", "/api/migrate") => {
            let query = match parse_query(query) {
                Ok(query) => query,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let dry_run = match required_bool_query(&query, "dry_run") {
                Ok(dry_run) => dry_run,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            if !permissions.allow_migrate {
                return write_json_error(
                    &mut stream,
                    403,
                    anyhow::anyhow!("GUI migrate は設定で無効化されています"),
                );
            }
            if let Err(error) =
                validate_query_keys(&query, &["dry_run", "database", "group", "limit"])
            {
                return write_json_error(&mut stream, 400, error);
            }
            let database = match optional_nonempty_query(&query, "database") {
                Ok(database) => database,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let group = match optional_nonempty_query(&query, "group") {
                Ok(group) => group,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let limit = match optional_usize_query(&query, "limit") {
                Ok(limit) => limit,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let result = migrate_with_options(
                &config,
                MigrateOptions {
                    dry_run,
                    selection: DatabaseSelection {
                        database: database.map(str::to_string),
                        group: group.map(str::to_string),
                        limit,
                    },
                    backup_before_migrate: None,
                },
            )
            .and_then(|report| {
                let report_write_error = write_report_json(&config, &report).err();
                if let Some(error) = report_write_error {
                    Err(error)
                } else {
                    write_audit_event(&config, "gui.migrate", &report)?;
                    Ok(report)
                }
            });
            write_json_result(&mut stream, result)
        }
        _ => write_json_error(&mut stream, 404, anyhow::anyhow!("not found")),
    }
}

fn initial_setup_api_allowed(method: &str, path: &str) -> bool {
    matches!(
        (method, path),
        ("GET", "/api/state") | ("POST", "/api/admin/gui-permissions")
    )
}
