use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sqlite_fleet::{
    check, discover_databases, load_migrations, migrate, status_report, write_report_json, Config,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_SQL_BYTES: usize = 2 * 1024 * 1024;
const MAX_HTTP_BODY_BYTES: usize = MAX_SQL_BYTES + 4 * 1024;

struct HttpRequest {
    head: String,
    initial_body: Vec<u8>,
}

struct ServerState {
    config: Config,
    csrf_token: String,
    script_nonce: String,
    bind_ip: IpAddr,
    port: u16,
}

pub fn serve(config: Config, host: &str, port: u16) -> Result<()> {
    validate_gui_host(host)?;
    let listener = TcpListener::bind((host, port))
        .with_context(|| format!("GUIサーバを起動できません: {host}:{port}"))?;
    let addr = listener.local_addr()?;
    let state = ServerState {
        config,
        csrf_token: generate_csrf_token()?,
        script_nonce: generate_csrf_token()?,
        bind_ip: addr.ip(),
        port: addr.port(),
    };
    println!("GUI: http://{addr}/");
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
    if let Err(error) = validate_host_header(&headers, state.bind_ip, state.port) {
        return write_json_error(&mut stream, 403, error);
    }
    if is_api_path(path) {
        if let Err(error) = validate_api_token(&headers, &state.csrf_token) {
            return write_json_error(&mut stream, 403, error);
        }
    }
    let mut request_body = None;
    if matches!((method, path), ("POST", "/api/sql")) {
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

    match (method, path) {
        ("GET", "/") => {
            let body = INDEX_HTML
                .replace("__CSRF_TOKEN__", &state.csrf_token)
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
            write_json(&mut stream, 200, &api_state(&state.config))
        }
        ("GET", "/api/discover") => {
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json_result(&mut stream, discover_databases(&state.config))
        }
        ("GET", "/api/plan") => {
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json(&mut stream, 200, &api_plan(&state.config))
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
            write_json_result(&mut stream, api_schema(&state.config, database))
        }
        ("POST", "/api/check") => {
            if let Err(error) = validate_no_query(query) {
                return write_json_error(&mut stream, 400, error);
            }
            write_json(&mut stream, 200, &api_check(&state.config))
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
            write_json_result(
                &mut stream,
                api_sql(&state.config, database, dry_run, &body),
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
            if let Err(error) = validate_query_keys(&query, &["dry_run", "database"]) {
                return write_json_error(&mut stream, 400, error);
            }
            let database = match optional_nonempty_query(&query, "database") {
                Ok(database) => database,
                Err(error) => return write_json_error(&mut stream, 400, error),
            };
            let result = migrate(&state.config, dry_run, database).and_then(|report| {
                let report_write_error = write_report_json(&state.config, &report).err();
                if let Some(error) = report_write_error {
                    Err(error)
                } else {
                    Ok(report)
                }
            });
            write_json_result(&mut stream, result)
        }
        _ => write_json_error(&mut stream, 404, anyhow::anyhow!("not found")),
    }
}

fn parse_request_line(line: &str) -> Result<(&str, &str)> {
    if line.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("request line に制御文字は指定できません");
    }
    let mut parts = line.split(' ');
    let Some(method) = parts.next() else {
        bail!("request line にmethodが必要です");
    };
    let Some(target) = parts.next() else {
        bail!("request line にtargetが必要です");
    };
    let Some(version) = parts.next() else {
        bail!("request line にHTTP versionが必要です");
    };
    if parts.next().is_some() {
        bail!("request line の要素が多すぎます");
    }
    if !matches!(method, "GET" | "POST") {
        bail!("HTTP method はGETまたはPOSTだけ許可されます");
    }
    if !target.starts_with('/') {
        bail!("request target はabsolute pathである必要があります");
    }
    if target.starts_with("//") {
        bail!("request target はorigin-form pathである必要があります");
    }
    if target.contains('#') {
        bail!("request target にfragmentは指定できません");
    }
    if target_path_contains_percent_encoding(target) {
        bail!("request target のpathにpercent encodingは指定できません");
    }
    if target.contains('\\') {
        bail!("request target にambiguous path separatorは指定できません");
    }
    if target.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("request target に制御文字は指定できません");
    }
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        bail!("HTTP version はHTTP/1.0またはHTTP/1.1だけ許可されます");
    }
    Ok((method, target))
}

fn read_http_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let size = stream.read(&mut buffer)?;
        if size == 0 {
            if request.is_empty() {
                return Ok(None);
            }
            bail!("HTTP request header が完了していません");
        }
        request.extend_from_slice(&buffer[..size]);
        if let Some(header_end) = http_header_end(&request) {
            if header_end > MAX_HTTP_HEADER_BYTES {
                bail!("HTTP request header が大きすぎます");
            }
            let head = String::from_utf8(request[..header_end].to_vec())
                .context("HTTP request header はUTF-8である必要があります")
                .map(Some)?;
            return Ok(head.map(|head| HttpRequest {
                head,
                initial_body: request[header_end..].to_vec(),
            }));
        }
        if request.len() > MAX_HTTP_HEADER_BYTES {
            bail!("HTTP request header が大きすぎます");
        }
    }
}

fn http_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn validate_crlf_lines(request: &str) -> Result<()> {
    let bytes = request.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        match *byte {
            b'\n' if index == 0 || bytes[index - 1] != b'\r' => {
                bail!("HTTP header の行区切りはCRLFである必要があります");
            }
            b'\r' if bytes.get(index + 1) != Some(&b'\n') => {
                bail!("HTTP header の行区切りはCRLFである必要があります");
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ApiEnvelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

fn api_state(config: &Config) -> ApiEnvelope<StateData> {
    match (
        status_report(config),
        discover_databases(config),
        load_migrations(config),
    ) {
        (Ok(status), Ok(databases), Ok(migrations)) => ApiEnvelope {
            ok: true,
            data: Some(StateData {
                project: config.project.name.clone(),
                status,
                databases,
                migrations,
            }),
            error: None,
        },
        (status, databases, migrations) => ApiEnvelope {
            ok: false,
            data: None,
            error: Some(
                status
                    .err()
                    .or_else(|| databases.err())
                    .or_else(|| migrations.err())
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "状態を取得できません".to_string()),
            ),
        },
    }
}

fn api_plan(config: &Config) -> ApiEnvelope<Vec<sqlite_fleet::DatabasePlan>> {
    let result = discover_databases(config).and_then(|databases| {
        let migrations = load_migrations(config)?;
        Ok(sqlite_fleet::build_plan(config, &databases, &migrations))
    });
    match result {
        Ok(plan) => ApiEnvelope {
            ok: true,
            data: Some(plan),
            error: None,
        },
        Err(error) => ApiEnvelope {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        },
    }
}

fn api_check(config: &Config) -> ApiEnvelope<sqlite_fleet::CheckReport> {
    match check(config) {
        Ok(report) => ApiEnvelope {
            ok: true,
            data: Some(report),
            error: None,
        },
        Err(error) => ApiEnvelope {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        },
    }
}

fn api_schema(config: &Config, database_id: &str) -> Result<SchemaData> {
    let database = find_database(config, database_id)?;
    let conn = open_gui_database(config, &database, true)?;
    let mut tables = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT type, name
         FROM pragma_table_list
         WHERE schema = 'main'
           AND type IN ('table', 'view', 'virtual')
           AND name NOT GLOB 'sqlite_*'
         ORDER BY type, name",
    )?;
    let relations = stmt
        .query_map([], |row| {
            Ok(SchemaRelation {
                object_type: row.get(0)?,
                name: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);

    for relation in relations {
        let pragma = format!("PRAGMA table_xinfo({})", quote_sqlite_ident(&relation.name));
        let mut column_stmt = conn.prepare(&pragma)?;
        let columns = column_stmt
            .query_map([], |row| {
                Ok(ColumnInfo {
                    cid: row.get(0)?,
                    name: row.get(1)?,
                    column_type: row.get(2)?,
                    not_null: row.get::<_, i64>(3)? != 0,
                    default_value: row.get(4)?,
                    primary_key: row.get::<_, i64>(5)? != 0,
                    hidden: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        tables.push(TableInfo {
            object_type: relation.object_type,
            name: relation.name,
            columns,
        });
    }

    let mut object_stmt = conn.prepare(
        "SELECT type, name, tbl_name, sql
         FROM sqlite_schema
         WHERE type IN ('index', 'view', 'trigger')
           AND name NOT GLOB 'sqlite_*'
         ORDER BY type, name",
    )?;
    let objects = object_stmt
        .query_map([], |row| {
            Ok(SchemaObject {
                object_type: row.get(0)?,
                name: row.get(1)?,
                table_name: row.get(2)?,
                sql: row.get(3)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(SchemaData {
        database,
        tables,
        objects,
    })
}

fn api_sql(config: &Config, database_id: &str, dry_run: bool, body: &[u8]) -> Result<SqlResult> {
    let request: SqlRequest =
        serde_json::from_slice(body).context("SQL request body のJSONが不正です")?;
    let sql = request.sql.trim();
    if sql.is_empty() {
        bail!("SQL は空にできません");
    }
    if utf8_byte_len(&request.sql) > MAX_SQL_BYTES {
        bail!("SQL が大きすぎます");
    }
    if request.sql.contains('\0') {
        bail!("SQL にNUL文字は指定できません");
    }
    if let Some(pragma) = sql_unsafe_pragma(sql) {
        bail!("危険PRAGMAはGUIでは実行できません: PRAGMA {pragma}");
    }
    if dry_run && sql_contains_statement_keyword(sql, &["ATTACH", "DETACH"]) {
        bail!("ATTACH/DETACH を含むSQLはdry-runできません。外部DBへ影響する可能性があるため、内容を確認してから適用してください");
    }
    if dry_run && sql_contains_vacuum_into(sql) {
        bail!("VACUUM INTO を含むSQLはdry-runできません。外部ファイルを作成する可能性があるため、内容を確認してから適用してください");
    }

    let database = find_database(config, database_id)?;
    let changed = if dry_run {
        let copy = create_dry_run_database_copy(config, &database)?;
        execute_sql_on_dry_run_copy(copy, sql, config.execution.lock_timeout_ms)?
    } else {
        let conn = open_gui_database(config, &database, false)?;
        execute_sql_apply(&conn, sql)?
    };
    Ok(SqlResult {
        database: database.id,
        dry_run,
        changed,
        message: if dry_run {
            "dry-run OK".to_string()
        } else {
            "SQL applied".to_string()
        },
    })
}

fn execute_sql_apply(conn: &Connection, sql: &str) -> Result<u64> {
    if let Some(keyword) = sql_transaction_control_statement(sql) {
        bail!("GUI SQL apply はatomic transactionで実行するため、transaction制御文は使用できません: {keyword}");
    }
    if sql_contains_statement_keyword(sql, &["ATTACH", "DETACH"]) {
        bail!("ATTACH/DETACH を含むSQLはGUI applyできません。外部DBへ影響するため、sqlite3等で明示的に実行してください");
    }
    if sql_contains_journal_mode_pragma(sql) {
        if sql_statement_command_count(sql) == 1 {
            let before = conn.total_changes();
            conn.execute_batch(sql)?;
            return Ok(conn.total_changes().saturating_sub(before));
        }
        bail!("PRAGMA journal_mode はatomic applyできません。単独SQLとして実行してください");
    }
    if sql_contains_statement_keyword(sql, &["VACUUM"]) {
        if sql_statement_command_count(sql) == 1 {
            let before = conn.total_changes();
            conn.execute_batch(sql)?;
            return Ok(conn.total_changes().saturating_sub(before));
        }
        bail!("VACUUMを含むSQLはatomic applyできません。VACUUMだけを単独で実行してください");
    }

    execute_sql_apply_atomically(conn, sql)
}

fn execute_sql_apply_atomically(conn: &Connection, sql: &str) -> Result<u64> {
    let before = conn.total_changes();
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    match conn.execute_batch(sql) {
        Ok(()) => {
            if let Err(error) = conn.execute_batch("COMMIT;") {
                let _ = conn.execute_batch("ROLLBACK;");
                return Err(anyhow::Error::new(error))
                    .context("GUI SQL apply transaction をcommitできません");
            }
            Ok(conn.total_changes().saturating_sub(before))
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch("ROLLBACK;") {
                return Err(anyhow::Error::new(error)).context(format!(
                    "GUI SQL apply に失敗し、rollbackにも失敗しました: {rollback_error}"
                ));
            }
            Err(anyhow::Error::new(error)).context("GUI SQL apply に失敗したためrollbackしました")
        }
    }
}

struct DryRunDatabaseCopy {
    path: PathBuf,
}

impl DryRunDatabaseCopy {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DryRunDatabaseCopy {
    fn drop(&mut self) {
        remove_sqlite_database_files(&self.path);
    }
}

fn remove_sqlite_database_files(path: &Path) {
    let Some(file_name) = path.file_name() else {
        let _ = std::fs::remove_file(path);
        return;
    };
    for candidate in std::iter::once(path.to_path_buf()).chain(
        ["-wal", "-shm", "-journal"]
            .into_iter()
            .map(|suffix| path.with_file_name(append_os_suffix(file_name, suffix))),
    ) {
        let _ = std::fs::remove_file(candidate);
    }
}

fn append_os_suffix(value: &std::ffi::OsStr, suffix: &str) -> OsString {
    let mut value = value.to_os_string();
    value.push(suffix);
    value
}

fn execute_sql_on_dry_run_copy(
    copy: DryRunDatabaseCopy,
    sql: &str,
    lock_timeout_ms: u64,
) -> Result<u64> {
    let conn = Connection::open_with_flags(copy.path(), OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("dry-run用DBコピーを開けません: {}", copy.path().display()))?;
    configure_gui_connection(&conn, lock_timeout_ms)?;
    let before = conn.total_changes();
    conn.execute_batch(sql)
        .map(|_| conn.total_changes().saturating_sub(before))
        .map_err(anyhow::Error::from)
}

fn create_dry_run_database_copy(
    config: &Config,
    database: &sqlite_fleet::Database,
) -> Result<DryRunDatabaseCopy> {
    let source = open_gui_database(config, database, true)?;
    let token = generate_csrf_token()?;
    let destination = std::env::temp_dir().join(format!("sqlite-fleet-dry-run-{token}.db"));
    let sql = format!(
        "VACUUM main INTO {}",
        quote_sql_string(&destination.display().to_string())
    );
    if let Err(error) = source.execute_batch(&sql) {
        remove_sqlite_database_files(&destination);
        return Err(error).with_context(|| {
            format!(
                "dry-run用DBコピーを作成できません: {}",
                destination.display()
            )
        });
    }
    Ok(DryRunDatabaseCopy::new(destination))
}

fn find_database(config: &Config, database_id: &str) -> Result<sqlite_fleet::Database> {
    if database_id.trim().is_empty() || database_id.trim() != database_id {
        bail!("DB ID が不正です: {database_id}");
    }
    let databases = discover_databases(config)?;
    databases
        .into_iter()
        .find(|database| database.id == database_id)
        .ok_or_else(|| anyhow::anyhow!("指定されたDBが見つかりません: {database_id}"))
}

fn open_gui_database(
    config: &Config,
    database: &sqlite_fleet::Database,
    readonly: bool,
) -> Result<Connection> {
    if !database.path.exists() {
        bail!("DBファイルが存在しません: {}", database.path.display());
    }
    let base_dir = std::fs::canonicalize(&config.base_dir)
        .with_context(|| format!("base_dir を解決できません: {}", config.base_dir.display()))?;
    let database_path = std::fs::canonicalize(&database.path)
        .with_context(|| format!("DBパスを解決できません: {}", database.path.display()))?;
    if !database_path.starts_with(&base_dir) {
        bail!(
            "DBパスはbase_dir配下である必要があります: {}",
            database.path.display()
        );
    }
    let metadata = std::fs::metadata(&database.path)
        .with_context(|| format!("DBメタデータを取得できません: {}", database.path.display()))?;
    if !metadata.is_file() {
        bail!(
            "DBパスは通常ファイルである必要があります: {}",
            database.path.display()
        );
    }
    let flags = if readonly {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    };
    let conn = Connection::open_with_flags(&database.path, flags)
        .with_context(|| format!("DBを開けません: {}", database.path.display()))?;
    configure_gui_connection(&conn, config.execution.lock_timeout_ms)?;
    Ok(conn)
}

fn configure_gui_connection(conn: &Connection, lock_timeout_ms: u64) -> Result<()> {
    conn.busy_timeout(Duration::from_millis(lock_timeout_ms))?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

fn quote_sqlite_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn sql_contains_statement_keyword(sql: &str, keywords: &[&str]) -> bool {
    let mut explain_prefix = ExplainPrefix::None;
    for token in sql_token_details(sql, false) {
        if is_statement_command_token(&mut explain_prefix, &token)
            && keywords
                .iter()
                .any(|keyword| token.text.eq_ignore_ascii_case(keyword))
        {
            return true;
        }
    }
    false
}

fn sql_transaction_control_statement(sql: &str) -> Option<String> {
    let mut explain_prefix = ExplainPrefix::None;
    let tokens = sql_token_details(sql, false);
    let mut pending_create_statement = false;
    let mut in_create_trigger = false;
    for (index, token) in tokens.iter().enumerate() {
        if pending_create_statement && token.statement_start {
            pending_create_statement = false;
        }
        if pending_create_statement
            && token.kind == SqlTokenKind::Bare
            && token.text.eq_ignore_ascii_case("TRIGGER")
        {
            pending_create_statement = false;
            in_create_trigger = true;
        }
        if !is_statement_command_token(&mut explain_prefix, token) {
            continue;
        }
        let keyword = token.text.to_ascii_uppercase();
        if keyword == "CREATE" {
            pending_create_statement = true;
        }
        if keyword == "END" && in_create_trigger {
            in_create_trigger = false;
            continue;
        }
        if keyword == "END"
            && tokens.get(index + 1).is_some_and(|next| {
                next.kind == SqlTokenKind::Bare && next.text.eq_ignore_ascii_case("TRANSACTION")
            })
        {
            return Some("END TRANSACTION".to_string());
        }
        if matches!(
            keyword.as_str(),
            "BEGIN" | "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" | "END"
        ) {
            return Some(keyword);
        }
    }
    None
}

fn sql_statement_command_count(sql: &str) -> usize {
    let mut explain_prefix = ExplainPrefix::None;
    let mut count = 0;
    for token in sql_token_details(sql, false) {
        if is_statement_command_token(&mut explain_prefix, &token) {
            count += 1;
        }
    }
    count
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExplainPrefix {
    None,
    AfterExplain,
    AfterExplainQuery,
    AfterExplainQueryPlan,
}

fn is_statement_command_token(explain_prefix: &mut ExplainPrefix, token: &SqlToken) -> bool {
    if token.statement_start {
        *explain_prefix = if token.text.eq_ignore_ascii_case("EXPLAIN") {
            ExplainPrefix::AfterExplain
        } else {
            ExplainPrefix::None
        };
        return *explain_prefix == ExplainPrefix::None;
    }

    match *explain_prefix {
        ExplainPrefix::None => false,
        ExplainPrefix::AfterExplain => {
            if token.text.eq_ignore_ascii_case("QUERY") {
                *explain_prefix = ExplainPrefix::AfterExplainQuery;
                false
            } else {
                *explain_prefix = ExplainPrefix::None;
                true
            }
        }
        ExplainPrefix::AfterExplainQuery => {
            *explain_prefix = if token.text.eq_ignore_ascii_case("PLAN") {
                ExplainPrefix::AfterExplainQueryPlan
            } else {
                ExplainPrefix::None
            };
            false
        }
        ExplainPrefix::AfterExplainQueryPlan => {
            *explain_prefix = ExplainPrefix::None;
            true
        }
    }
}

fn sql_contains_vacuum_into(sql: &str) -> bool {
    let mut explain_prefix = ExplainPrefix::None;
    let mut vacuum_statement = false;
    for token in sql_token_details(sql, false) {
        if token.statement_start {
            vacuum_statement = false;
        }
        if vacuum_statement && token.text.eq_ignore_ascii_case("INTO") {
            return true;
        }
        if is_statement_command_token(&mut explain_prefix, &token) {
            vacuum_statement = token.text.eq_ignore_ascii_case("VACUUM");
        }
    }
    false
}

fn sql_contains_journal_mode_pragma(sql: &str) -> bool {
    let tokens = sql_token_details(sql, true);
    tokens.iter().enumerate().any(|(index, token)| {
        token.statement_start
            && token.kind == SqlTokenKind::Bare
            && token.text.eq_ignore_ascii_case("PRAGMA")
            && pragma_name_matches(sql, &tokens, index, "journal_mode")
    })
}

fn sql_unsafe_pragma(sql: &str) -> Option<&'static str> {
    let tokens = sql_token_details(sql, true);
    tokens.iter().enumerate().find_map(|(index, token)| {
        if !token.statement_start
            || token.kind != SqlTokenKind::Bare
            || !token.text.eq_ignore_ascii_case("PRAGMA")
        {
            return None;
        }

        if pragma_value(sql, &tokens, index, "foreign_keys")
            .is_some_and(|token| is_pragma_boolean_value(token, false))
        {
            return Some("foreign_keys");
        }
        if pragma_value(sql, &tokens, index, "ignore_check_constraints")
            .is_some_and(|token| is_pragma_boolean_value(token, true))
        {
            return Some("ignore_check_constraints");
        }
        if pragma_name_matches(sql, &tokens, index, "writable_schema") {
            return Some("writable_schema");
        }
        if pragma_value(sql, &tokens, index, "journal_mode").is_some_and(is_journal_mode_off_value)
        {
            return Some("journal_mode=OFF");
        }
        None
    })
}

fn pragma_name_matches(sql: &str, tokens: &[SqlToken], pragma_index: usize, name: &str) -> bool {
    tokens
        .get(pragma_index + 1)
        .is_some_and(|token| token.text.eq_ignore_ascii_case(name))
        || tokens
            .get(pragma_index + 1)
            .zip(tokens.get(pragma_index + 2))
            .is_some_and(|(schema, pragma_name)| {
                pragma_name.text.eq_ignore_ascii_case(name)
                    && has_schema_qualifier_separator(&sql[schema.end..pragma_name.start])
            })
}

fn pragma_value<'a>(
    sql: &str,
    tokens: &'a [SqlToken],
    pragma_index: usize,
    name: &str,
) -> Option<&'a str> {
    if tokens
        .get(pragma_index + 1)
        .is_some_and(|token| token.text.eq_ignore_ascii_case(name))
    {
        return tokens
            .get(pragma_index + 2)
            .filter(|value| {
                has_pragma_value_separator(&sql[tokens[pragma_index + 1].end..value.start])
            })
            .map(|value| value.text.as_str());
    }
    if tokens
        .get(pragma_index + 1)
        .zip(tokens.get(pragma_index + 2))
        .is_some_and(|(schema, pragma_name)| {
            pragma_name.text.eq_ignore_ascii_case(name)
                && has_schema_qualifier_separator(&sql[schema.end..pragma_name.start])
        })
    {
        return tokens
            .get(pragma_index + 3)
            .filter(|value| {
                has_pragma_value_separator(&sql[tokens[pragma_index + 2].end..value.start])
            })
            .map(|value| value.text.as_str());
    }
    None
}

fn has_pragma_value_separator(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let index = skip_sql_spacing_and_comments(bytes, 0);
    matches!(bytes.get(index), Some(b'=' | b'('))
        && skip_sql_spacing_and_comments(bytes, index + 1) == bytes.len()
}

fn has_schema_qualifier_separator(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let mut index = skip_sql_spacing_and_comments(bytes, 0);
    if bytes.get(index) != Some(&b'.') {
        return false;
    }
    index += 1;
    skip_sql_spacing_and_comments(bytes, index) == bytes.len()
}

fn skip_sql_spacing_and_comments(bytes: &[u8], mut index: usize) -> usize {
    loop {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if bytes.get(index) == Some(&b'-') && bytes.get(index + 1) == Some(&b'-') {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes.get(index) == Some(&b'/') && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        return index;
    }
}

fn is_pragma_boolean_value(token: &str, expected: bool) -> bool {
    let token = token.trim();
    if token.is_empty() {
        return false;
    }
    let numeric_value = token.parse::<i64>().ok();
    let is_false = numeric_value == Some(0)
        || matches!(token.to_ascii_lowercase().as_str(), "off" | "false" | "no");
    let is_true = numeric_value.is_some_and(|value| value > 0)
        || matches!(token.to_ascii_lowercase().as_str(), "on" | "true" | "yes");
    if expected {
        is_true
    } else {
        is_false
    }
}

fn is_journal_mode_off_value(token: &str) -> bool {
    let token = token.trim();
    let numeric_value = token.parse::<i64>().ok();
    numeric_value == Some(0) || token.eq_ignore_ascii_case("off")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SqlTokenKind {
    Bare,
    Quoted,
}

#[derive(Debug)]
struct SqlToken {
    text: String,
    kind: SqlTokenKind,
    statement_start: bool,
    start: usize,
    end: usize,
}

impl SqlToken {
    fn bare(text: impl Into<String>, statement_start: bool, start: usize, end: usize) -> Self {
        Self {
            text: text.into(),
            kind: SqlTokenKind::Bare,
            statement_start,
            start,
            end,
        }
    }

    fn quoted(text: String, statement_start: bool, start: usize, end: usize) -> Self {
        Self {
            text,
            kind: SqlTokenKind::Quoted,
            statement_start,
            start,
            end,
        }
    }
}

fn sql_token_details(sql: &str, include_quoted_literals: bool) -> Vec<SqlToken> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut keywords = Vec::new();
    let mut statement_start = true;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' if include_quoted_literals => {
                let start = index;
                let (token, next_index) = read_quoted_sql_token(bytes, index, bytes[index]);
                keywords.push(SqlToken::quoted(token, statement_start, start, next_index));
                statement_start = false;
                index = next_index;
            }
            b'\'' => {
                index = skip_quoted_sql(bytes, index, b'\'');
                statement_start = false;
            }
            b'"' => {
                index = skip_quoted_sql(bytes, index, b'"');
                statement_start = false;
            }
            b'`' => {
                index = skip_quoted_sql(bytes, index, b'`');
                statement_start = false;
            }
            b'[' if include_quoted_literals => {
                let start = index;
                let (token, next_index) = read_bracket_sql_token(bytes, index);
                keywords.push(SqlToken::quoted(token, statement_start, start, next_index));
                statement_start = false;
                index = next_index;
            }
            b'[' => {
                index = skip_bracket_quoted_ident(bytes, index);
                statement_start = false;
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
            }
            b';' => {
                statement_start = true;
                index += 1;
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = index;
                index += 1;
                while index < bytes.len()
                    && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
                {
                    index += 1;
                }
                let token = &sql[start..index];
                keywords.push(SqlToken::bare(token, statement_start, start, index));
                statement_start = false;
            }
            byte if byte.is_ascii_digit() => {
                let start = index;
                index += 1;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                let token = &sql[start..index];
                keywords.push(SqlToken::bare(token, statement_start, start, index));
                statement_start = false;
            }
            b'+' | b'-' if bytes.get(index + 1).is_some_and(u8::is_ascii_digit) => {
                let start = index;
                index += 2;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                let token = &sql[start..index];
                keywords.push(SqlToken::bare(token, statement_start, start, index));
                statement_start = false;
            }
            _ => index += 1,
        }
    }
    keywords
}

fn read_quoted_sql_token(bytes: &[u8], mut index: usize, quote: u8) -> (String, usize) {
    index += 1;
    let mut token = Vec::new();
    while index < bytes.len() {
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                token.push(quote);
                index += 2;
            } else {
                return (String::from_utf8_lossy(&token).into_owned(), index + 1);
            }
        } else {
            token.push(bytes[index]);
            index += 1;
        }
    }
    (String::from_utf8_lossy(&token).into_owned(), index)
}

fn read_bracket_sql_token(bytes: &[u8], mut index: usize) -> (String, usize) {
    index += 1;
    let mut token = Vec::new();
    while index < bytes.len() {
        if bytes[index] == b']' {
            if bytes.get(index + 1) == Some(&b']') {
                token.push(b']');
                index += 2;
            } else {
                return (String::from_utf8_lossy(&token).into_owned(), index + 1);
            }
        } else {
            token.push(bytes[index]);
            index += 1;
        }
    }
    (String::from_utf8_lossy(&token).into_owned(), index)
}

fn skip_quoted_sql(bytes: &[u8], mut index: usize, quote: u8) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    index
}

fn skip_bracket_quoted_ident(bytes: &[u8], mut index: usize) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b']' {
            if bytes.get(index + 1) == Some(&b']') {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    index
}

#[derive(Serialize)]
struct StateData {
    project: Option<String>,
    status: sqlite_fleet::StatusReport,
    databases: Vec<sqlite_fleet::Database>,
    migrations: Vec<sqlite_fleet::Migration>,
}

#[derive(Serialize)]
struct SchemaData {
    database: sqlite_fleet::Database,
    tables: Vec<TableInfo>,
    objects: Vec<SchemaObject>,
}

#[derive(Serialize)]
struct TableInfo {
    #[serde(rename = "type")]
    object_type: String,
    name: String,
    columns: Vec<ColumnInfo>,
}

struct SchemaRelation {
    object_type: String,
    name: String,
}

#[derive(Serialize)]
struct SchemaObject {
    #[serde(rename = "type")]
    object_type: String,
    name: String,
    table_name: String,
    sql: Option<String>,
}

#[derive(Serialize)]
struct ColumnInfo {
    cid: i64,
    name: String,
    #[serde(rename = "type")]
    column_type: String,
    not_null: bool,
    default_value: Option<String>,
    primary_key: bool,
    hidden: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SqlRequest {
    sql: String,
}

#[derive(Serialize)]
struct SqlResult {
    database: String,
    dry_run: bool,
    changed: u64,
    message: String,
}

fn write_json_result<T: Serialize>(stream: &mut TcpStream, result: Result<T>) -> Result<()> {
    match result {
        Ok(data) => write_json(
            stream,
            200,
            &ApiEnvelope {
                ok: true,
                data: Some(data),
                error: None,
            },
        ),
        Err(error) => write_json(
            stream,
            200,
            &ApiEnvelope::<()> {
                ok: false,
                data: None,
                error: Some(error.to_string()),
            },
        ),
    }
}

fn utf8_byte_len(value: &str) -> usize {
    value.len()
}

fn write_json<T: Serialize>(stream: &mut TcpStream, status: u16, value: &T) -> Result<()> {
    let body = serde_json::to_string(value)?;
    write_response(
        stream,
        status,
        "application/json; charset=utf-8",
        &body,
        None,
        None,
    )
}

fn write_json_error(stream: &mut TcpStream, status: u16, error: anyhow::Error) -> Result<()> {
    write_json(
        stream,
        status,
        &ApiEnvelope::<()> {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        },
    )
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
    script_nonce: Option<&str>,
    style_nonce: Option<&str>,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "OK",
    };
    let script_src = script_nonce
        .map(|nonce| format!("'nonce-{nonce}'"))
        .unwrap_or_else(|| "'none'".to_string());
    let style_src = style_nonce
        .map(|nonce| format!("'nonce-{nonce}'"))
        .unwrap_or_else(|| "'none'".to_string());
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src {script_src}; style-src {style_src}; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    Ok(())
}

fn split_target(target: &str) -> (&str, &str) {
    target.split_once('?').unwrap_or((target, ""))
}

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

fn target_path_contains_percent_encoding(target: &str) -> bool {
    split_target(target).0.contains('%')
}

fn parse_headers(request: &str) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();
    for line in request.lines().skip(1).take_while(|line| !line.is_empty()) {
        if line.starts_with(' ') || line.starts_with('\t') {
            bail!("HTTP header の折り返し形式は許可されません");
        }
        let Some((key, value)) = line.split_once(':') else {
            bail!("HTTP header の形式が不正です");
        };
        if key.trim() != key {
            bail!("HTTP header 名の前後に空白は使用できません");
        }
        let key = key.to_ascii_lowercase();
        if key.is_empty() {
            bail!("HTTP header 名が空です");
        }
        if !is_valid_header_name(&key) {
            bail!("HTTP header 名が不正です: {key}");
        }
        let value = value.trim();
        if value.bytes().any(|byte| byte.is_ascii_control()) {
            bail!("HTTP header 値に制御文字は指定できません: {key}");
        }
        if matches!(
            key.as_str(),
            "host"
                | "x-sqlite-fleet-token"
                | "content-length"
                | "content-type"
                | "transfer-encoding"
        ) && headers.contains_key(&key)
        {
            bail!("重複したHTTP header は許可されません: {key}");
        }
        headers.insert(key, value.to_string());
    }
    Ok(headers)
}

fn is_valid_header_name(name: &str) -> bool {
    name.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            )
    })
}

fn validate_api_token(headers: &HashMap<String, String>, expected: &str) -> Result<()> {
    match headers.get("x-sqlite-fleet-token") {
        Some(actual) if constant_time_eq(actual, expected) => Ok(()),
        _ => bail!("GUI API token が不正です。画面を更新して再実行してください"),
    }
}

fn constant_time_eq(actual: &str, expected: &str) -> bool {
    let actual = actual.as_bytes();
    let expected = expected.as_bytes();
    let mut diff = actual.len() ^ expected.len();
    for (index, expected_byte) in expected.iter().enumerate() {
        let actual_byte = actual.get(index).copied().unwrap_or(0);
        diff |= usize::from(actual_byte ^ expected_byte);
    }
    diff == 0
}

fn validate_no_request_body(
    headers: &HashMap<String, String>,
    has_initial_body: bool,
) -> Result<()> {
    if headers.contains_key("transfer-encoding") {
        bail!("GUI API request body は使用できません");
    }
    if has_initial_body {
        bail!("GUI API request body は使用できません");
    }
    match headers.get("content-length").map(String::as_str) {
        Some("0") | None => Ok(()),
        Some(_) => bail!("GUI API request body は使用できません"),
    }
}

fn validate_json_content_type(headers: &HashMap<String, String>) -> Result<()> {
    let Some(content_type) = headers.get("content-type") else {
        bail!("Content-Type は application/json が必要です");
    };
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if media_type == "application/json" {
        Ok(())
    } else {
        bail!("Content-Type は application/json が必要です");
    }
}

fn read_request_body(
    stream: &mut TcpStream,
    headers: &HashMap<String, String>,
    mut body: Vec<u8>,
) -> Result<Vec<u8>> {
    if headers.contains_key("transfer-encoding") {
        bail!("Transfer-Encoding は使用できません");
    }
    let Some(length) = headers.get("content-length") else {
        bail!("Content-Length が必要です");
    };
    let length = parse_content_length(length)?;
    if length > MAX_HTTP_BODY_BYTES {
        bail!("HTTP request body が大きすぎます");
    }
    if body.len() > length {
        bail!("HTTP request body がContent-Lengthを超えています");
    }
    while body.len() < length {
        let remaining = length - body.len();
        let mut buffer = vec![0_u8; remaining.min(8192)];
        let size = stream.read(&mut buffer)?;
        if size == 0 {
            bail!("HTTP request body が完了していません");
        }
        body.extend_from_slice(&buffer[..size]);
    }
    Ok(body)
}

fn parse_content_length(value: &str) -> Result<usize> {
    if value.is_empty() {
        bail!("Content-Length が空です");
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("Content-Length はASCII数字だけ指定できます: {value}");
    }
    if value.len() > 1 && value.starts_with('0') {
        bail!("Content-Length に先頭ゼロは使用できません: {value}");
    }
    value
        .parse()
        .with_context(|| format!("Content-Length が不正です: {value}"))
}

fn validate_host_header(
    headers: &HashMap<String, String>,
    bind_ip: IpAddr,
    port: u16,
) -> Result<()> {
    let Some(host) = headers.get("host") else {
        bail!("Host header が必要です");
    };
    let (hostname, header_port) = parse_host_header(host)?;
    match header_port {
        Some(header_port) if header_port == port => {}
        Some(_) => bail!("Host header のportが不正です"),
        None if port == 80 => {}
        None => bail!("Host header にはGUI serverのportが必要です"),
    }
    if is_localhost_alias(&hostname) && is_default_loopback(bind_ip) {
        return Ok(());
    }
    match hostname.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() && ip == bind_ip => Ok(()),
        Ok(ip) if ip.is_loopback() => {
            bail!("Host header のIPがGUI serverのbind addressと一致しません")
        }
        _ => bail!("Host header はループバックホストのみ許可されます"),
    }
}

fn is_localhost_alias(hostname: &str) -> bool {
    matches!(hostname, "localhost" | "localhost.")
}

fn is_default_loopback(ip: IpAddr) -> bool {
    matches!(
        ip,
        IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST
    ) || matches!(
        ip,
        IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST
    )
}

fn parse_host_header(host: &str) -> Result<(String, Option<u16>)> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        bail!("Host header が空です");
    }
    if let Some(rest) = host.strip_prefix('[') {
        let Some((hostname, suffix)) = rest.split_once(']') else {
            bail!("Host header のIPv6形式が不正です");
        };
        if hostname.is_empty() {
            bail!("Host header のhostnameが空です");
        }
        if hostname.parse::<Ipv6Addr>().is_err() {
            bail!("Host header のIPv6 literalが不正です");
        }
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(parse_host_port(port)?)
        } else {
            bail!("Host header のIPv6形式が不正です");
        };
        return Ok((hostname.to_string(), port));
    }

    if host.matches(':').count() == 1 {
        let Some((hostname, port)) = host.rsplit_once(':') else {
            bail!("Host header の形式が不正です");
        };
        if hostname.is_empty() {
            bail!("Host header のhostnameが空です");
        }
        Ok((hostname.to_string(), Some(parse_host_port(port)?)))
    } else {
        if host.contains(':') {
            bail!("Host header のIPv6 literalには角括弧が必要です");
        }
        if host.is_empty() {
            bail!("Host header のhostnameが空です");
        }
        Ok((host.to_string(), None))
    }
}

fn parse_host_port(port: &str) -> Result<u16> {
    if port.is_empty() {
        bail!("Host header のportが空です");
    }
    if !port.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("Host header のportはASCII数字だけ指定できます: {port}");
    }
    if port.len() > 1 && port.starts_with('0') {
        bail!("Host header のportに先頭ゼロは使用できません: {port}");
    }
    port.parse()
        .with_context(|| format!("Host header のportが不正です: {port}"))
}

fn parse_query(query: &str) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    if query.is_empty() {
        return Ok(values);
    }
    for part in query.split('&') {
        if part.is_empty() {
            bail!("query parameter が空です");
        }
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        if key.is_empty() {
            bail!("query parameter 名が空です");
        }
        if key.bytes().any(|byte| byte.is_ascii_control())
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            bail!("query parameter に制御文字は指定できません");
        }
        if values.insert(key.clone(), value).is_some() {
            bail!("重複したquery parameter は許可されません: {key}");
        }
    }
    Ok(values)
}

fn required_bool_query(query: &HashMap<String, String>, name: &str) -> Result<bool> {
    match query.get(name).map(String::as_str) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(value) => bail!("query parameter {name} はtrueまたはfalseが必要です: {value}"),
        None => bail!("query parameter {name} が必要です"),
    }
}

fn optional_nonempty_query<'a>(
    query: &'a HashMap<String, String>,
    name: &str,
) -> Result<Option<&'a str>> {
    match query.get(name).map(String::as_str) {
        Some("") => bail!("query parameter {name} は空にできません"),
        Some(value) => Ok(Some(value)),
        None => Ok(None),
    }
}

fn validate_query_keys(query: &HashMap<String, String>, allowed: &[&str]) -> Result<()> {
    for key in query.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!("未知のquery parameterです: {key}");
        }
    }
    Ok(())
}

fn validate_no_query(query: &str) -> Result<()> {
    if query.is_empty() {
        Ok(())
    } else {
        bail!("このAPI endpointにquery parameterは指定できません");
    }
}

fn percent_decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .context("query parameter のpercent encodingが不正です")?;
                decoded.push(
                    u8::from_str_radix(hex, 16)
                        .context("query parameter のpercent encodingが不正です")?,
                );
                i += 3;
            }
            b'%' => bail!("query parameter のpercent encodingが不正です"),
            byte => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).context("query parameter はUTF-8である必要があります")
}

fn validate_gui_host(host: &str) -> Result<()> {
    let addrs = (host, 0)
        .to_socket_addrs()
        .with_context(|| format!("GUI host を解決できません: {host}"))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        bail!("GUI host を解決できません: {host}");
    }
    if addrs.iter().all(|addr| addr.ip().is_loopback()) {
        Ok(())
    } else {
        bail!("GUI host はループバックアドレスのみ指定できます: {host}");
    }
}

fn generate_csrf_token() -> Result<String> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).context("GUI API token を生成できません")?;
    Ok(hex_encode(random))
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="ja">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>sqlite-fleet GUI</title>
  <style nonce="__SCRIPT_NONCE__">
    :root { color-scheme: light; --bg:#f6f7f9; --panel:#ffffff; --text:#1f2933; --muted:#607080; --line:#d9e0e7; --accent:#1769aa; --danger:#b42318; --ok:#16794c; --warn:#9a5b00; }
    * { box-sizing: border-box; }
    body { margin:0; background:var(--bg); color:var(--text); font:14px/1.45 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    .layout { display:grid; grid-template-columns:280px minmax(0, 1fr); min-height:100vh; }
    .sidebar { position:sticky; top:0; height:100vh; overflow:auto; border-right:1px solid var(--line); background:var(--panel); padding:20px; }
    .content { min-width:0; padding:24px; }
    .brand { margin-bottom:18px; }
    h1 { margin:0; font-size:20px; font-weight:650; }
    .toolbar { display:grid; gap:8px; margin:16px 0; }
    .actions { display:flex; flex-wrap:wrap; gap:8px; }
    .sidebar .actions, .sidebar .actions button { width:100%; }
    button { border:1px solid var(--line); background:#fff; color:var(--text); border-radius:6px; min-height:34px; padding:0 12px; font:inherit; cursor:pointer; }
    button.primary { background:var(--accent); border-color:var(--accent); color:#fff; }
    button.danger { background:var(--danger); border-color:var(--danger); color:#fff; }
    button:disabled { opacity:.55; cursor:wait; }
    input, select, textarea { width:100%; border:1px solid var(--line); border-radius:6px; background:#fff; color:var(--text); font:inherit; padding:8px 10px; }
    textarea { min-height:260px; resize:vertical; font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .form-grid { display:grid; grid-template-columns:repeat(2, minmax(0, 1fr)); gap:10px; padding:16px; }
    .form-grid .wide { grid-column:1 / -1; }
    .stack { display:grid; gap:10px; }
    .schema-list { display:grid; gap:10px; padding:16px; }
    .schema-table { border:1px solid var(--line); border-radius:8px; overflow:hidden; }
    .schema-table h3 { margin:0; padding:10px 12px; font-size:14px; border-bottom:1px solid var(--line); background:#fafbfc; }
    .schema-sql { margin:0; max-height:180px; overflow:auto; white-space:pre-wrap; word-break:break-word; }
    .summary { display:grid; grid-template-columns:1fr; gap:10px; margin-top:16px; }
    .metric, .panel { background:var(--panel); border:1px solid var(--line); border-radius:8px; }
    .metric { padding:12px; }
    .metric strong { display:block; font-size:22px; line-height:1.1; margin-top:4px; }
    .label { color:var(--muted); font-size:12px; }
    .panel { margin-bottom:16px; overflow:hidden; }
    .panel h2 { font-size:16px; margin:0; padding:14px 16px; border-bottom:1px solid var(--line); }
    table { width:100%; border-collapse:collapse; }
    th, td { padding:10px 12px; text-align:left; border-bottom:1px solid var(--line); vertical-align:top; }
    th { font-size:12px; color:var(--muted); background:#fafbfc; font-weight:600; }
    tr:last-child td { border-bottom:0; }
    code { font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size:12px; overflow-wrap:anywhere; }
    .pill { display:inline-block; border-radius:999px; padding:2px 8px; font-size:12px; border:1px solid var(--line); }
    .ok { color:var(--ok); }
    .warn { color:var(--warn); }
    .bad { color:var(--danger); }
    .muted { color:var(--muted); }
    .message { margin:0; padding:10px 12px; border:1px solid var(--line); border-radius:8px; background:#fff; }
    .message.error { border-color:#f2b8b5; color:var(--danger); }
    @media (max-width: 820px) { .layout { display:block; } .sidebar { position:static; height:auto; border-right:0; border-bottom:1px solid var(--line); } .content { padding:16px; } .summary { grid-template-columns:repeat(2, 1fr); } th:nth-child(2), td:nth-child(2) { display:none; } }
  </style>
</head>
<body>
  <div class="layout">
    <aside class="sidebar">
      <div class="brand">
        <h1 id="title">sqlite-fleet</h1>
        <div class="label">SQLite fleet database manager</div>
      </div>
      <p id="message" class="message muted">読み込み中...</p>
      <div class="toolbar">
        <div class="actions">
          <button id="refresh">更新</button>
          <button id="check">Check</button>
          <button id="dryRun" class="primary">Dry run</button>
          <button id="migrateAll" class="danger">全DBへ適用</button>
        </div>
      </div>
      <section class="summary" id="summary"></section>
    </aside>
    <main class="content">
      <section class="panel">
        <h2>Databases</h2>
        <table>
          <thead><tr><th>ID</th><th>Path</th><th>Status</th><th>Applied</th><th>Pending</th><th>Actions</th></tr></thead>
          <tbody id="databases"></tbody>
        </table>
      </section>
      <section class="panel">
        <h2>SQL Console</h2>
        <div class="form-grid">
          <label>Target DB<select id="sqlDatabase"></select></label>
          <label>SQL file<input id="sqlFile" type="file" accept=".sql,text/sql,text/plain"></label>
          <label>SQL template<select id="sqlTemplate"></select></label>
          <label>Output file name<input id="sqlFileName" value="sqlite-fleet-change.sql"></label>
          <label class="wide">SQL<textarea id="sqlInput" spellcheck="false" placeholder="CREATE TABLE example(id INTEGER PRIMARY KEY);"></textarea></label>
          <div class="actions wide">
            <button id="insertTemplate">テンプレート挿入</button>
            <button id="downloadSql">SQLファイル保存</button>
            <button id="sqlDryRun" class="primary">SQL dry-run</button>
            <button id="sqlApply" class="danger">SQLを適用</button>
            <button id="loadSchema">Schemaを再読み込み</button>
          </div>
        </div>
      </section>
      <section class="panel">
        <h2>Schema Editor</h2>
        <div class="form-grid">
          <label>New table<input id="newTableName" placeholder="new_table"></label>
          <label>Columns<input id="newTableColumns" placeholder="id INTEGER PRIMARY KEY, name TEXT"></label>
          <div class="actions wide"><button id="generateCreateTable">CREATE TABLE SQL生成</button></div>

          <label>Table<input id="alterTableName" placeholder="target_table"></label>
          <label>New column<input id="newColumnName" placeholder="new_column"></label>
          <label class="wide">Column definition<input id="newColumnDefinition" placeholder="TEXT NOT NULL DEFAULT ''"></label>
          <div class="actions wide"><button id="generateAddColumn">ADD COLUMN SQL生成</button></div>

          <label>Rename table from<input id="renameTableFrom" placeholder="old_table"></label>
          <label>Rename table to<input id="renameTableTo" placeholder="new_table"></label>
          <div class="actions wide"><button id="generateRenameTable">RENAME TABLE SQL生成</button></div>

          <label>Rename column table<input id="renameColumnTable" placeholder="target_table"></label>
          <label>Rename column from<input id="renameColumnFrom" placeholder="old_column"></label>
          <label class="wide">Rename column to<input id="renameColumnTo" placeholder="new_column"></label>
          <div class="actions wide"><button id="generateRenameColumn">RENAME COLUMN SQL生成</button></div>
        </div>
        <div id="schema" class="schema-list"></div>
      </section>
      <section class="panel">
        <h2>Migrations</h2>
        <table>
          <thead><tr><th>Version</th><th>Name</th><th>Checksum</th></tr></thead>
          <tbody id="migrations"></tbody>
        </table>
      </section>
    </main>
  </div>
  <script nonce="__SCRIPT_NONCE__">
    const $ = (id) => document.getElementById(id);
    const apiToken = '__CSRF_TOKEN__';
    const maxSqlBytes = 2 * 1024 * 1024;
    const maxSqlRequestBytes = maxSqlBytes + 4 * 1024;
    const sqlTemplates = {
      create_table: ['CREATE TABLE', 'CREATE TABLE "example" (\n  "id" INTEGER PRIMARY KEY,\n  "name" TEXT NOT NULL,\n  "created_at" TEXT DEFAULT CURRENT_TIMESTAMP\n);'],
      create_table_strict: ['CREATE TABLE STRICT', 'CREATE TABLE "example_strict" (\n  "id" INTEGER PRIMARY KEY,\n  "name" TEXT NOT NULL,\n  "amount" REAL\n) STRICT;'],
      create_temp_table: ['CREATE TEMP TABLE', 'CREATE TEMP TABLE "temp_example" (\n  "id" INTEGER PRIMARY KEY,\n  "value" TEXT\n);'],
      create_without_rowid: ['CREATE TABLE WITHOUT ROWID', 'CREATE TABLE "example_without_rowid" (\n  "id" TEXT PRIMARY KEY,\n  "name" TEXT NOT NULL\n) WITHOUT ROWID;'],
      create_table_generated: ['CREATE TABLE generated columns', 'CREATE TABLE "example_generated" (\n  "value" INTEGER NOT NULL,\n  "value_text" TEXT GENERATED ALWAYS AS (printf(\'%d\', "value")) VIRTUAL,\n  CHECK ("value" >= 0)\n);'],
      create_table_stored_generated: ['CREATE TABLE stored generated', 'CREATE TABLE "example_stored_generated" (\n  "price" REAL NOT NULL CHECK ("price" >= 0),\n  "quantity" INTEGER NOT NULL CHECK ("quantity" >= 0),\n  "total" REAL GENERATED ALWAYS AS ("price" * "quantity") STORED\n);'],
      create_table_foreign_key: ['CREATE TABLE foreign key', 'CREATE TABLE "parent" (\n  "id" INTEGER PRIMARY KEY\n);\nCREATE TABLE "child" (\n  "id" INTEGER PRIMARY KEY,\n  "parent_id" INTEGER NOT NULL,\n  FOREIGN KEY ("parent_id") REFERENCES "parent"("id") ON DELETE CASCADE\n);'],
      create_table_constraints: ['CREATE TABLE constraints', 'CREATE TABLE "example_constraints" (\n  "id" INTEGER PRIMARY KEY,\n  "email" TEXT NOT NULL COLLATE NOCASE,\n  "status" TEXT NOT NULL DEFAULT \'active\' CHECK ("status" IN (\'active\', \'disabled\')),\n  UNIQUE ("email")\n);'],
      create_table_as: ['CREATE TABLE AS SELECT', 'CREATE TABLE "example_copy" AS\nSELECT * FROM "example";'],
      alter_add_column: ['ALTER TABLE ADD COLUMN', 'ALTER TABLE "example" ADD COLUMN "notes" TEXT;'],
      alter_rename_table: ['ALTER TABLE RENAME TO', 'ALTER TABLE "example" RENAME TO "example_new";'],
      alter_rename_column: ['ALTER TABLE RENAME COLUMN', 'ALTER TABLE "example" RENAME COLUMN "old_name" TO "new_name";'],
      alter_drop_column: ['ALTER TABLE DROP COLUMN', 'ALTER TABLE "example" DROP COLUMN "unused_column";'],
      create_index: ['CREATE INDEX', 'CREATE INDEX "idx_example_name" ON "example" ("name");'],
      create_unique_index: ['CREATE UNIQUE INDEX', 'CREATE UNIQUE INDEX "idx_example_name_unique" ON "example" ("name");'],
      create_partial_index: ['CREATE INDEX partial', 'CREATE INDEX "idx_example_active_name" ON "example" ("name")\nWHERE "deleted_at" IS NULL;'],
      create_expression_index: ['CREATE INDEX expression', 'CREATE INDEX "idx_example_lower_name" ON "example" (lower("name"));'],
      drop_index: ['DROP INDEX', 'DROP INDEX IF EXISTS "idx_example_name";'],
      create_view: ['CREATE VIEW', 'CREATE VIEW "example_view" AS\nSELECT "id", "name"\nFROM "example";'],
      create_temp_view: ['CREATE TEMP VIEW', 'CREATE TEMP VIEW "temp_example_view" AS\nSELECT "id", "name"\nFROM "example";'],
      drop_view: ['DROP VIEW', 'DROP VIEW IF EXISTS "example_view";'],
      create_trigger: ['CREATE TRIGGER', 'CREATE TRIGGER "trg_example_updated"\nAFTER UPDATE ON "example"\nFOR EACH ROW\nBEGIN\n  UPDATE "example" SET "created_at" = CURRENT_TIMESTAMP WHERE "id" = NEW."id";\nEND;'],
      create_instead_of_trigger: ['CREATE TRIGGER INSTEAD OF', 'CREATE TRIGGER "trg_example_view_insert"\nINSTEAD OF INSERT ON "example_view"\nFOR EACH ROW\nBEGIN\n  INSERT INTO "example" ("name") VALUES (NEW."name");\nEND;'],
      create_temp_trigger: ['CREATE TEMP TRIGGER', 'CREATE TEMP TRIGGER "temp_trg_example_insert"\nBEFORE INSERT ON "example"\nFOR EACH ROW\nWHEN NEW."name" IS NULL\nBEGIN\n  SELECT RAISE(IGNORE);\nEND;'],
      drop_trigger: ['DROP TRIGGER', 'DROP TRIGGER IF EXISTS "trg_example_updated";'],
      virtual_fts5: ['CREATE VIRTUAL TABLE FTS5', 'CREATE VIRTUAL TABLE "example_fts" USING fts5("title", "body");'],
      virtual_rtree: ['CREATE VIRTUAL TABLE RTree', 'CREATE VIRTUAL TABLE "example_rtree" USING rtree(\n  "id",\n  "min_x", "max_x",\n  "min_y", "max_y"\n);'],
      insert_rows: ['INSERT', 'INSERT INTO "example" ("name") VALUES\n  (\'alpha\'),\n  (\'beta\');'],
      insert_select: ['INSERT SELECT', 'INSERT INTO "example_archive" ("id", "name")\nSELECT "id", "name"\nFROM "example"\nWHERE "deleted_at" IS NOT NULL;'],
      insert_returning: ['INSERT RETURNING', 'INSERT INTO "example" ("name") VALUES (\'alpha\')\nRETURNING *;'],
      insert_or_replace: ['INSERT OR REPLACE', 'INSERT OR REPLACE INTO "example" ("id", "name") VALUES (1, \'alpha\');'],
      update_rows: ['UPDATE', 'UPDATE "example"\nSET "name" = \'updated\'\nWHERE "id" = 1;'],
      update_returning: ['UPDATE RETURNING', 'UPDATE "example"\nSET "name" = \'updated\'\nWHERE "id" = 1\nRETURNING *;'],
      delete_rows: ['DELETE', 'DELETE FROM "example"\nWHERE "id" = 1;'],
      delete_returning: ['DELETE RETURNING', 'DELETE FROM "example"\nWHERE "id" = 1\nRETURNING *;'],
      upsert_rows: ['UPSERT', 'INSERT INTO "example" ("id", "name") VALUES (1, \'alpha\')\nON CONFLICT("id") DO UPDATE SET "name" = excluded."name";'],
      with_query: ['WITH / CTE', 'WITH recent AS (\n  SELECT * FROM "example" ORDER BY "id" DESC LIMIT 10\n)\nSELECT * FROM recent;'],
      recursive_cte: ['WITH RECURSIVE', 'WITH RECURSIVE nums(n) AS (\n  SELECT 1\n  UNION ALL\n  SELECT n + 1 FROM nums WHERE n < 10\n)\nSELECT n FROM nums;'],
      transaction: ['Transaction (reference)', '-- GUI applyは自動的にatomic transactionで実行されるため、BEGIN/COMMITは使用できません。\n-- 明示transactionが必要な場合はsqlite3等で実行してください。\nBEGIN IMMEDIATE;\n-- SQL statements here\nCOMMIT;'],
      savepoint: ['SAVEPOINT (reference)', '-- GUI applyはSAVEPOINT/RELEASEを使用できません。\n-- 明示savepointが必要な場合はsqlite3等で実行してください。\nSAVEPOINT "edit_batch";\n-- SQL statements here\nRELEASE "edit_batch";'],
      explain_query_plan: ['EXPLAIN QUERY PLAN', 'EXPLAIN QUERY PLAN\nSELECT * FROM "example" WHERE "name" = \'alpha\';'],
      pragma: ['PRAGMA', 'PRAGMA user_version;\nPRAGMA foreign_key_check;\nPRAGMA integrity_check;\nPRAGMA quick_check;\nPRAGMA optimize;\nPRAGMA wal_checkpoint(PASSIVE);'],
      pragma_journal_mode: ['PRAGMA journal_mode (single apply only)', '-- PRAGMA journal_modeはatomic transactionでは実行できないため、単独SQLとしてだけ適用できます。\n-- OFFは危険なためGUI SQLでは拒否されます。\nPRAGMA journal_mode = WAL;'],
      vacuum: ['VACUUM', 'VACUUM;'],
      vacuum_into: ['VACUUM INTO (single apply only)', '-- VACUUM INTOは外部ファイルを作成するため、SQL dry-runでは拒否されます。\n-- atomic transactionでは実行できないため、単独SQLとしてだけ適用できます。\n-- 出力先パスを確認し、DBコピーを作成する意図がある場合だけ適用してください。\nVACUUM INTO \'backup.db\';'],
      analyze: ['ANALYZE', 'ANALYZE;'],
      reindex: ['REINDEX', 'REINDEX;'],
      attach_database: ['ATTACH DATABASE (external tool)', '-- ATTACH/DETACHは外部DBへ影響するため、GUI SQLではdry-run/適用とも拒否されます。\n-- sqlite3等でパスと内容を確認してから明示的に実行してください。\nATTACH DATABASE \'other.db\' AS other;\n-- SELECT * FROM other.sqlite_schema;\nDETACH DATABASE other;'],
      drop_table: ['DROP TABLE', 'DROP TABLE IF EXISTS "example";'],
      custom: ['Custom SQL', '-- 任意のSQLite SQLをここに書いてください。\n-- 基本的にはdry-runで確認してから適用してください。\n-- GUI applyは自動的にatomic transactionで実行されます。\n-- ATTACH/DETACHはGUI SQLでは拒否され、VACUUM/VACUUM INTO/PRAGMA journal_modeは単独SQLとしてだけ適用できます。\n'],
    };
    let state = null;
    let schemaState = null;
    let activeRequests = 0;

    function setBusy(value) {
      activeRequests = value ? activeRequests + 1 : Math.max(0, activeRequests - 1);
      syncControlState();
    }
    function syncControlState() {
      const disabled = activeRequests > 0;
      document.querySelectorAll('button, input, select, textarea').forEach((control) => control.disabled = disabled);
      if (!disabled && state && state.status && state.status.plans.length === 0) {
        $('sqlDatabase').disabled = true;
      }
    }
    function message(text, isError = false) {
      const el = $('message');
      el.textContent = text;
      el.className = `message ${isError ? 'error' : 'muted'}`;
    }
    async function api(path, options = {}) {
      setBusy(true);
      try {
        const { headers: optionHeaders = {}, ...fetchOptions } = options;
        const headers = Object.assign({}, optionHeaders);
        headers['X-SQLite-Fleet-Token'] = apiToken;
        const response = await fetch(path, Object.assign({}, fetchOptions, { headers }));
        const contentType = response.headers.get('content-type') || '';
        const mediaType = contentType.split(';', 1)[0].trim().toLowerCase();
        if (mediaType !== 'application/json') {
          throw new Error(`GUI API returned HTTP ${response.status}`);
        }
        const text = await response.text();
        let payload;
        try {
          payload = JSON.parse(text);
        } catch (_error) {
          throw new Error('GUI API response is not valid JSON');
        }
        if (!payload || typeof payload.ok !== 'boolean') {
          throw new Error('GUI API response envelope is invalid');
        }
        if (!payload.ok) throw new Error(payload.error || 'request failed');
        return payload.data;
      } finally {
        setBusy(false);
      }
    }
    async function load(options = {}) {
      const showLoadedMessage = options.showLoadedMessage !== false;
      try {
        state = await api('/api/state');
        render();
        if (showLoadedMessage) message('最新状態を表示しています');
      } catch (error) {
        message(error.message, true);
      }
    }
    function render() {
      const project = state.project || 'sqlite-fleet';
      $('title').textContent = project;
      const s = state.status;
      renderDatabaseSelectors(s.plans);
      $('summary').innerHTML = [
        ['DB数', s.database_count],
        ['最新', s.latest_migration ? `${s.latest_migration.version}_${s.latest_migration.name}` : 'なし'],
        ['最新適用済み', s.up_to_date],
        ['未適用あり', s.pending],
        ['失敗', s.failed],
        ['不整合', s.corrupt],
      ].map(([label, value]) => `<div class="metric"><span class="label">${escapeHtml(label)}</span><strong>${escapeHtml(String(value))}</strong></div>`).join('');

      $('databases').innerHTML = s.plans.map((plan) => {
        const pending = plan.pending.length;
        const status = plan.error ? `<span class="pill bad">error</span>` :
          plan.checksum_errors.length || plan.unknown_applied.length ? `<span class="pill bad">corrupt</span>` :
          pending ? `<span class="pill warn">pending</span>` : `<span class="pill ok">ok</span>`;
        const details = plan.error || [plan.checksum_errors.length && 'checksum', plan.unknown_applied.length && 'unknown migration'].filter(Boolean).join(', ');
        return `<tr>
          <td><code>${escapeHtml(plan.database.id)}</code></td>
          <td><code>${escapeHtml(plan.database.path)}</code></td>
          <td>${status}${details ? `<div class="muted">${escapeHtml(details)}</div>` : ''}</td>
          <td>${escapeHtml(plan.applied_count)}</td>
          <td>${pending ? escapeHtml(plan.pending.map((m) => `${m.version}_${m.name}`).join(', ')) : '<span class="muted">なし</span>'}</td>
          <td class="actions">
            <button data-action="migrate" data-dry-run="true" data-database="${escapeHtml(plan.database.id)}">Dry run</button>
            <button class="danger" data-action="migrate" data-dry-run="false" data-database="${escapeHtml(plan.database.id)}">適用</button>
          </td>
        </tr>`;
      }).join('');

      $('migrations').innerHTML = state.migrations.map((migration) => `<tr>
        <td><code>${escapeHtml(migration.version)}</code></td>
        <td>${escapeHtml(migration.name)}</td>
        <td><code>${escapeHtml(migration.checksum)}</code></td>
      </tr>`).join('') || '<tr><td colspan="3" class="muted">migration はありません</td></tr>';
    }
    function renderDatabaseSelectors(plans) {
      const current = $('sqlDatabase').value;
      if (!plans.length) {
        $('sqlDatabase').innerHTML = '<option value="">DBなし</option>';
        $('sqlDatabase').disabled = true;
        clearSchema();
        return;
      }
      $('sqlDatabase').disabled = activeRequests > 0;
      $('sqlDatabase').innerHTML = plans.map((plan) => `<option value="${escapeHtml(plan.database.id)}">${escapeHtml(plan.database.id)}</option>`).join('');
      if (current && plans.some((plan) => plan.database.id === current)) {
        $('sqlDatabase').value = current;
      } else if (current) {
        clearSchema();
      }
      const selected = $('sqlDatabase').value;
      if (schemaState && schemaState.database && schemaState.database.id !== selected) {
        clearSchema();
      }
    }
    function renderSqlTemplates() {
      $('sqlTemplate').innerHTML = Object.entries(sqlTemplates)
        .map(([key, template]) => `<option value="${escapeHtml(key)}">${escapeHtml(template[0])}</option>`)
        .join('');
    }
    async function loadSchema(options = {}) {
      const showLoadedMessage = options.showLoadedMessage !== false;
      const database = $('sqlDatabase').value;
      if (!database) {
        message('DBを選択してください', true);
        return;
      }
      try {
        const nextSchemaState = await api(`/api/schema?database=${encodeURIComponent(database)}`);
        if ($('sqlDatabase').value !== database) return;
        if (!nextSchemaState || !nextSchemaState.database || nextSchemaState.database.id !== database) {
          clearSchema();
          throw new Error('schema response database does not match selected DB');
        }
        schemaState = nextSchemaState;
        renderSchema();
        if (showLoadedMessage) message(`schema 読み込み完了: ${database}`);
      } catch (error) {
        if ($('sqlDatabase').value === database) clearSchema();
        message(error.message, true);
      }
    }
    function renderSchema() {
      const tables = schemaState && schemaState.tables ? schemaState.tables : [];
      const objects = schemaState && schemaState.objects ? schemaState.objects : [];
      const tableHtml = tables.map((table) => `<section class="schema-table">
        <h3><code>${escapeHtml(table.type || 'table')}</code> ${escapeHtml(table.name)}</h3>
        <table>
          <thead><tr><th>Column</th><th>Type</th><th>Not null</th><th>Default</th><th>PK</th><th>Hidden</th></tr></thead>
          <tbody>${table.columns.map((column) => `<tr>
            <td><code>${escapeHtml(column.name)}</code></td>
            <td><code>${escapeHtml(column.type || '')}</code></td>
            <td>${column.not_null ? 'yes' : 'no'}</td>
            <td><code>${escapeHtml(column.default_value || '')}</code></td>
            <td>${column.primary_key ? 'yes' : 'no'}</td>
            <td>${column.hidden ? escapeHtml(hiddenColumnLabel(column.hidden)) : 'no'}</td>
          </tr>`).join('')}</tbody>
        </table>
      </section>`).join('') || '<p class="muted">テーブルはありません</p>';
      const objectHtml = objects.length ? `<section class="schema-table">
        <h3>Object Definitions</h3>
        <table>
          <thead><tr><th>Type</th><th>Name</th><th>Table</th><th>SQL</th></tr></thead>
          <tbody>${objects.map((object) => `<tr>
            <td><code>${escapeHtml(object.type)}</code></td>
            <td><code>${escapeHtml(object.name)}</code></td>
            <td><code>${escapeHtml(object.table_name || '')}</code></td>
            <td><pre class="schema-sql"><code>${escapeHtml(object.sql || '')}</code></pre></td>
          </tr>`).join('')}</tbody>
        </table>
      </section>` : '';
      $('schema').innerHTML = `${tableHtml}${objectHtml}`;
    }
    function clearSchema() {
      schemaState = null;
      $('schema').innerHTML = '<p class="muted">DBを選択してSchemaを読み込んでください</p>';
    }
    function hiddenColumnLabel(hidden) {
      if (hidden === 1) return 'hidden';
      if (hidden === 2) return 'generated virtual';
      if (hidden === 3) return 'generated stored';
      return String(hidden);
    }
    async function runSql(dryRun) {
      const database = $('sqlDatabase').value;
      const sql = $('sqlInput').value;
      if (!database) {
        message('DBを選択してください', true);
        return;
      }
      if (!sql.trim()) {
        message('SQLを入力してください', true);
        return;
      }
      if (sql.includes('\u0000')) {
        message('SQLにNUL文字は指定できません', true);
        return;
      }
      if (new TextEncoder().encode(sql).length > maxSqlBytes) {
        message('SQLが大きすぎます。2MiB以下にしてください', true);
        return;
      }
      const sqlBody = JSON.stringify({ sql });
      if (new TextEncoder().encode(sqlBody).length > maxSqlRequestBytes) {
        message('SQLリクエストが大きすぎます。引用符や改行を減らすかSQLを分割してください', true);
        return;
      }
      if (!dryRun && !confirm('選択DBへSQLを適用します。続行しますか？')) return;
      try {
        const result = await api(`/api/sql?dry_run=${dryRun}&database=${encodeURIComponent(database)}`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: sqlBody,
        });
        message(`${dryRun ? 'SQL dry-run' : 'SQL apply'} 完了: changed=${result.changed}`);
        await refreshAfterSqlRun(database, dryRun);
      } catch (error) {
        message(error.message, true);
      }
    }
    async function refreshAfterSqlRun(database, dryRun) {
      if (dryRun) return;
      await load({ showLoadedMessage: false });
      if ($('sqlDatabase').value === database) {
        await loadSchema({ showLoadedMessage: false });
      }
    }
    function quoteIdent(name) {
      return `"${String(name).replace(/"/g, '""')}"`;
    }
    function setSql(sql) {
      $('sqlInput').value = sql;
      $('sqlInput').focus();
    }
    function insertSelectedTemplate() {
      const template = sqlTemplates[$('sqlTemplate').value];
      if (!template) {
        message('SQLテンプレートを選択してください', true);
        return;
      }
      setSql(template[1]);
      message(`${template[0]} テンプレートを挿入しました。内容を確認・編集してください`);
    }
    function downloadSqlFile() {
      const sql = $('sqlInput').value;
      if (!sql.trim()) {
        message('保存するSQLを入力してください', true);
        return;
      }
      const filename = sanitizeSqlFileName($('sqlFileName').value);
      const blob = new Blob([sql], { type: 'text/sql;charset=utf-8' });
      const link = document.createElement('a');
      const url = URL.createObjectURL(blob);
      link.href = url;
      link.download = filename;
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
      message(`SQLファイルを保存しました: ${filename}`);
    }
    function sanitizeSqlFileName(value) {
      const fallback = 'sqlite-fleet-change.sql';
      const sanitized = String(value || '')
        .trim()
        .replace(/[\\/:*?"<>|]/g, '_')
        .replace(/^\.+$/, '')
        .replace(/^_+$/, '');
      const name = sanitized || fallback;
      return name.toLowerCase().endsWith('.sql') ? name : `${name}.sql`;
    }
    function requireValue(id, label) {
      const value = $(id).value.trim();
      if (!value) throw new Error(`${label} を入力してください`);
      return value;
    }
    function generateSql(builder) {
      try {
        setSql(builder());
        message('SQLを生成しました。dry-run後に適用してください');
      } catch (error) {
        message(error.message, true);
      }
    }
    async function runMigrate(database, dryRun) {
      if (!dryRun && !confirm(database ? 'このDBへmigrationを適用します。続行しますか？' : '全DBへmigrationを適用します。続行しますか？')) return;
      try {
        const suffix = `${database ? `&database=${encodeURIComponent(database)}` : ''}`;
        const report = await api(`/api/migrate?dry_run=${dryRun}${suffix}`, { method: 'POST' });
        message(`${dryRun ? 'dry run' : 'migrate'} 完了: processed=${report.processed_databases}, failed=${report.failed_databases}`);
        await load({ showLoadedMessage: false });
        const selectedDatabase = $('sqlDatabase').value;
        if (!dryRun && selectedDatabase && (!database || selectedDatabase === database)) {
          await loadSchema({ showLoadedMessage: false });
        }
      } catch (error) {
        message(error.message, true);
      }
    }
    async function runCheck() {
      try {
        const report = await api('/api/check', { method: 'POST' });
        message(`check 完了: ok=${report.ok}, failed=${report.failed}`);
      } catch (error) {
        message(error.message, true);
      }
    }
    function escapeHtml(value) {
      return String(value).replace(/[&<>"']/g, (c) => ({ '&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;' }[c]));
    }
    $('refresh').addEventListener('click', load);
    $('check').addEventListener('click', runCheck);
    $('dryRun').addEventListener('click', () => runMigrate('', true));
    $('migrateAll').addEventListener('click', () => runMigrate('', false));
    $('loadSchema').addEventListener('click', loadSchema);
    $('insertTemplate').addEventListener('click', insertSelectedTemplate);
    $('downloadSql').addEventListener('click', downloadSqlFile);
    $('sqlDryRun').addEventListener('click', () => runSql(true));
    $('sqlApply').addEventListener('click', () => runSql(false));
    $('sqlDatabase').addEventListener('change', clearSchema);
    $('sqlFile').addEventListener('change', async (event) => {
      const file = event.target.files && event.target.files[0];
      if (!file) return;
      if (file.size > maxSqlBytes) {
        event.target.value = '';
        message('SQLファイルが大きすぎます。2MiB以下にしてください', true);
        return;
      }
      let sql;
      try {
        const bytes = await file.arrayBuffer();
        sql = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
      } catch (error) {
        event.target.value = '';
        message(`SQLファイルをUTF-8として読み込めません: ${error.message || error}`, true);
        return;
      }
      if (sql.includes('\u0000')) {
        event.target.value = '';
        message('SQLファイルにNUL文字は指定できません', true);
        return;
      }
      $('sqlInput').value = sql;
      $('sqlFileName').value = sanitizeSqlFileName(file.name);
      message(`SQLファイルを読み込みました: ${file.name}`);
    });
    $('generateCreateTable').addEventListener('click', () => generateSql(() => {
      const table = requireValue('newTableName', 'New table');
      const columns = requireValue('newTableColumns', 'Columns');
      return `CREATE TABLE ${quoteIdent(table)} (${columns});`;
    }));
    $('generateAddColumn').addEventListener('click', () => generateSql(() => {
      const table = requireValue('alterTableName', 'Table');
      const column = requireValue('newColumnName', 'New column');
      const definition = requireValue('newColumnDefinition', 'Column definition');
      return `ALTER TABLE ${quoteIdent(table)} ADD COLUMN ${quoteIdent(column)} ${definition};`;
    }));
    $('generateRenameTable').addEventListener('click', () => generateSql(() => {
      const from = requireValue('renameTableFrom', 'Rename table from');
      const to = requireValue('renameTableTo', 'Rename table to');
      return `ALTER TABLE ${quoteIdent(from)} RENAME TO ${quoteIdent(to)};`;
    }));
    $('generateRenameColumn').addEventListener('click', () => generateSql(() => {
      const table = requireValue('renameColumnTable', 'Rename column table');
      const from = requireValue('renameColumnFrom', 'Rename column from');
      const to = requireValue('renameColumnTo', 'Rename column to');
      return `ALTER TABLE ${quoteIdent(table)} RENAME COLUMN ${quoteIdent(from)} TO ${quoteIdent(to)};`;
    }));
    $('databases').addEventListener('click', (event) => {
      const button = event.target.closest('button[data-action="migrate"]');
      if (!button) return;
      runMigrate(button.dataset.database || '', button.dataset.dryRun === 'true');
    });
    renderSqlTemplates();
    load();
  </script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;

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
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let state = ServerState {
            config: Config::default(),
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

    #[test]
    fn html_does_not_use_inline_event_handlers() {
        assert!(!INDEX_HTML.contains("onclick="));
        assert!(INDEX_HTML.contains("data-action=\"migrate\""));
    }

    #[test]
    fn html_script_uses_nonce_placeholder() {
        assert!(INDEX_HTML.contains("<script nonce=\"__SCRIPT_NONCE__\">"));
        assert!(INDEX_HTML.contains("<style nonce=\"__SCRIPT_NONCE__\">"));
    }

    #[test]
    fn html_api_client_validates_json_envelope() {
        assert!(
            INDEX_HTML.contains("const { headers: optionHeaders = {}, ...fetchOptions } = options")
        );
        assert!(INDEX_HTML.contains("Object.assign({}, optionHeaders)"));
        assert!(INDEX_HTML.contains("Object.assign({}, fetchOptions, { headers })"));
        assert!(INDEX_HTML.contains("response.headers.get('content-type')"));
        assert!(INDEX_HTML
            .contains("const mediaType = contentType.split(';', 1)[0].trim().toLowerCase()"));
        assert!(INDEX_HTML.contains("mediaType !== 'application/json'"));
        assert!(INDEX_HTML.contains("JSON.parse(text)"));
        assert!(INDEX_HTML.contains("typeof payload.ok !== 'boolean'"));
        assert!(!INDEX_HTML.contains("response.json()"));
    }

    #[test]
    fn html_keeps_buttons_disabled_until_all_api_requests_finish() {
        assert!(INDEX_HTML.contains("let activeRequests = 0"));
        assert!(INDEX_HTML.contains(
            "activeRequests = value ? activeRequests + 1 : Math.max(0, activeRequests - 1)"
        ));
        assert!(INDEX_HTML.contains("function syncControlState()"));
        assert!(INDEX_HTML.contains("const disabled = activeRequests > 0"));
        assert!(INDEX_HTML.contains("button, input, select, textarea"));
        assert!(INDEX_HTML.contains("control.disabled = disabled"));
        assert!(!INDEX_HTML.contains("let busy = false"));
    }

    #[test]
    fn html_preserves_migrate_completion_message_after_refresh() {
        assert!(
            INDEX_HTML.contains("const showLoadedMessage = options.showLoadedMessage !== false")
        );
        assert!(INDEX_HTML.contains("await load({ showLoadedMessage: false })"));
        assert!(INDEX_HTML.contains("const selectedDatabase = $('sqlDatabase').value"));
        assert!(INDEX_HTML.contains(
            "if (!dryRun && selectedDatabase && (!database || selectedDatabase === database))"
        ));
    }

    #[test]
    fn html_preserves_sql_completion_message_after_schema_refresh() {
        assert!(INDEX_HTML.contains("async function loadSchema(options = {})"));
        assert!(INDEX_HTML.contains("await refreshAfterSqlRun(database, dryRun)"));
        assert!(INDEX_HTML.contains("async function refreshAfterSqlRun(database, dryRun)"));
        assert!(INDEX_HTML.contains("if (dryRun) return"));
        assert!(INDEX_HTML
            .contains("if (showLoadedMessage) message(`schema 読み込み完了: ${database}`)"));
        assert!(INDEX_HTML.contains("if ($('sqlDatabase').value !== database) return"));
        assert!(INDEX_HTML.contains("nextSchemaState.database.id !== database"));
        assert!(INDEX_HTML
            .contains("throw new Error('schema response database does not match selected DB')"));
        assert!(INDEX_HTML.contains("if ($('sqlDatabase').value === database) clearSchema()"));
        assert!(INDEX_HTML.contains("await loadSchema({ showLoadedMessage: false })"));
        assert!(INDEX_HTML.contains(
            "await load({ showLoadedMessage: false });\n      if ($('sqlDatabase').value === database)"
        ));
    }

    #[test]
    fn html_disables_database_selector_when_no_databases_exist() {
        assert!(INDEX_HTML.contains("if (!plans.length)"));
        assert!(INDEX_HTML.contains(r#"<option value="">DBなし</option>"#));
        assert!(INDEX_HTML.contains("$('sqlDatabase').disabled = true"));
        assert!(INDEX_HTML.contains("$('sqlDatabase').disabled = activeRequests > 0"));
        assert!(INDEX_HTML.contains("clearSchema()"));
        assert!(INDEX_HTML.contains("$('sqlDatabase').addEventListener('change', clearSchema)"));
        assert!(INDEX_HTML.contains("function clearSchema()"));
        assert!(INDEX_HTML.contains("DBを選択してSchemaを読み込んでください"));
        assert!(INDEX_HTML.contains("const selected = $('sqlDatabase').value"));
        assert!(INDEX_HTML.contains(
            "schemaState && schemaState.database && schemaState.database.id !== selected"
        ));
    }

    #[test]
    fn html_escapes_dynamic_table_values_consistently() {
        assert!(INDEX_HTML.contains("return String(value).replace"));
        assert!(INDEX_HTML.contains("<td>${escapeHtml(plan.applied_count)}</td>"));
        assert!(!INDEX_HTML.contains("<td>${plan.applied_count}</td>"));
    }

    #[test]
    fn html_escapes_dynamic_attribute_values_consistently() {
        assert!(INDEX_HTML.contains(r#"data-database="${escapeHtml(plan.database.id)}""#));
        assert!(INDEX_HTML.contains(r#"<option value="${escapeHtml(plan.database.id)}">"#));
        assert!(INDEX_HTML.contains(r#"<option value="${escapeHtml(key)}">"#));
        assert!(!INDEX_HTML.contains(r#"data-database="${plan.database.id}""#));
        assert!(!INDEX_HTML.contains(r#"<option value="${plan.database.id}">"#));
    }

    #[test]
    fn html_uses_sidebar_layout() {
        assert!(INDEX_HTML.contains(r#"<div class="layout">"#));
        assert!(INDEX_HTML.contains(r#"<aside class="sidebar">"#));
        assert!(INDEX_HTML.contains(r#"<main class="content">"#));
        assert!(INDEX_HTML.contains("grid-template-columns:280px minmax(0, 1fr)"));
    }

    #[test]
    fn html_supports_sql_templates_upload_edit_and_download() {
        assert!(INDEX_HTML.contains("const sqlTemplates = {"));
        assert!(INDEX_HTML.contains("const maxSqlBytes = 2 * 1024 * 1024"));
        assert!(INDEX_HTML.contains("create_table_strict"));
        assert!(INDEX_HTML.contains("create_table_generated"));
        assert!(INDEX_HTML.contains("create_table_stored_generated"));
        assert!(INDEX_HTML.contains("create_table_foreign_key"));
        assert!(INDEX_HTML.contains("create_table_constraints"));
        assert!(INDEX_HTML.contains("create_table_as"));
        assert!(INDEX_HTML.contains("create_partial_index"));
        assert!(INDEX_HTML.contains("create_expression_index"));
        assert!(INDEX_HTML.contains("create_temp_view"));
        assert!(INDEX_HTML.contains("create_instead_of_trigger"));
        assert!(INDEX_HTML.contains("create_temp_trigger"));
        assert!(INDEX_HTML.contains("virtual_fts5"));
        assert!(INDEX_HTML.contains("virtual_rtree"));
        assert!(INDEX_HTML.contains("insert_select"));
        assert!(INDEX_HTML.contains("insert_returning"));
        assert!(INDEX_HTML.contains("insert_or_replace"));
        assert!(INDEX_HTML.contains("update_returning"));
        assert!(INDEX_HTML.contains("delete_returning"));
        assert!(INDEX_HTML.contains("BEFORE INSERT ON"));
        assert!(INDEX_HTML.contains(r#"WHEN NEW."name" IS NULL"#));
        assert!(INDEX_HTML.contains("savepoint"));
        assert!(INDEX_HTML.contains("recursive_cte"));
        assert!(INDEX_HTML.contains("explain_query_plan"));
        assert!(INDEX_HTML.contains("PRAGMA integrity_check"));
        assert!(INDEX_HTML.contains("PRAGMA quick_check"));
        assert!(INDEX_HTML.contains("PRAGMA optimize"));
        assert!(INDEX_HTML.contains("PRAGMA wal_checkpoint(PASSIVE)"));
        assert!(INDEX_HTML.contains("PRAGMA journal_mode (single apply only)"));
        assert!(INDEX_HTML.contains("PRAGMA journal_modeはatomic transactionでは実行できないため"));
        assert!(INDEX_HTML.contains("VACUUM INTO (single apply only)"));
        assert!(INDEX_HTML.contains("VACUUM INTOは外部ファイルを作成するため"));
        assert!(INDEX_HTML.contains("ATTACH DATABASE (external tool)"));
        assert!(INDEX_HTML.contains("ATTACH/DETACHは外部DBへ影響するため"));
        assert!(INDEX_HTML.contains("基本的にはdry-runで確認してから適用してください"));
        assert!(INDEX_HTML.contains("GUI applyは自動的にatomic transactionで実行されます"));
        assert!(INDEX_HTML.contains("PRAGMA journal_modeは単独SQLとしてだけ適用できます"));
        assert!(INDEX_HTML.contains(r#"<select id="sqlTemplate">"#));
        assert!(INDEX_HTML.contains(r#"<input id="sqlFile" type="file""#));
        assert!(INDEX_HTML.contains(r#"<textarea id="sqlInput""#));
        assert!(INDEX_HTML.contains(r#"<button id="downloadSql">SQLファイル保存</button>"#));
        assert!(INDEX_HTML.contains("new Blob([sql]"));
        assert!(INDEX_HTML.contains("sql.includes('\\u0000')"));
        assert!(INDEX_HTML.contains("SQLにNUL文字は指定できません"));
        assert!(INDEX_HTML.contains("SQLファイルにNUL文字は指定できません"));
        assert!(INDEX_HTML.contains("file.size > maxSqlBytes"));
        assert!(INDEX_HTML.contains("new TextEncoder().encode(sql).length > maxSqlBytes"));
        assert!(INDEX_HTML.contains("const maxSqlRequestBytes = maxSqlBytes + 4 * 1024"));
        assert!(INDEX_HTML.contains("const sqlBody = JSON.stringify({ sql })"));
        assert!(
            INDEX_HTML.contains("new TextEncoder().encode(sqlBody).length > maxSqlRequestBytes")
        );
        assert!(INDEX_HTML.contains("body: sqlBody"));
        assert!(INDEX_HTML.contains("file.arrayBuffer()"));
        assert!(INDEX_HTML.contains("new TextDecoder('utf-8', { fatal: true })"));
        assert!(INDEX_HTML.contains("SQLファイルをUTF-8として読み込めません"));
        assert!(!INDEX_HTML.contains("file.text()"));
    }

    #[test]
    fn html_sanitizes_download_sql_file_name() {
        assert!(INDEX_HTML.contains("function sanitizeSqlFileName(value)"));
        assert!(INDEX_HTML.contains("$('sqlFileName').value = sanitizeSqlFileName(file.name)"));
        assert!(INDEX_HTML.contains("replace(/^\\.+$/, '')"));
        assert!(INDEX_HTML.contains("replace(/^_+$/, '')"));
        assert!(INDEX_HTML.contains("name.toLowerCase().endsWith('.sql')"));
        assert!(INDEX_HTML.contains("const url = URL.createObjectURL(blob)"));
        assert!(INDEX_HTML.contains("URL.revokeObjectURL(url)"));
    }

    #[test]
    fn html_displays_hidden_schema_columns() {
        assert!(INDEX_HTML.contains("escapeHtml(table.type || 'table')"));
        assert!(INDEX_HTML.contains("<th>Hidden</th>"));
        assert!(INDEX_HTML.contains("hiddenColumnLabel(column.hidden)"));
        assert!(INDEX_HTML.contains("generated virtual"));
        assert!(INDEX_HTML.contains("generated stored"));
    }

    #[test]
    fn html_displays_schema_object_definitions() {
        assert!(INDEX_HTML.contains("Object Definitions"));
        assert!(INDEX_HTML.contains("schemaState.objects"));
        assert!(INDEX_HTML.contains("object.table_name"));
        assert!(INDEX_HTML.contains("schema-sql"));
    }
}
