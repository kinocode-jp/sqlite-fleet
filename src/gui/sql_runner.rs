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

