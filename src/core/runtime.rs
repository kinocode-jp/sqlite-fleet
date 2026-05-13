impl From<&Migration> for MigrationSummary {
    fn from(migration: &Migration) -> Self {
        Self {
            group: migration.group.clone(),
            filename: migration.filename.clone(),
            version: migration.version.clone(),
            name: migration.name.clone(),
            checksum: migration.checksum.clone(),
        }
    }
}

impl From<&AppliedMigration> for MigrationSummary {
    fn from(migration: &AppliedMigration) -> Self {
        Self {
            group: "unknown".to_string(),
            filename: migration.filename.clone(),
            version: migration.version.clone(),
            name: migration.name.clone(),
            checksum: migration.checksum.clone(),
        }
    }
}

pub(crate) fn open_existing_readonly(path: &Path) -> Result<Connection> {
    ensure_existing_database_file(path)?;
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(Into::into)
}

fn open_existing_readwrite(path: &Path) -> Result<Connection> {
    ensure_existing_database_file(path)?;
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(Into::into)
}

fn ensure_existing_database_file(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("DBファイルが存在しません: {}", path.display());
    }
    let metadata = fs::metadata(path)
        .with_context(|| format!("DBメタデータを取得できません: {}", path.display()))?;
    if !metadata.is_file() {
        bail!(
            "DBパスは通常ファイルである必要があります: {}",
            path.display()
        );
    }
    Ok(())
}

fn refresh_database_state(database: &Database) -> Database {
    let exists = database.path.exists();
    let readable = exists
        && fs::metadata(&database.path).is_ok_and(|metadata| metadata.is_file())
        && fs::File::open(&database.path).is_ok();
    Database {
        id: database.id.clone(),
        path: database.path.clone(),
        exists,
        readable,
    }
}

fn refresh_database_state_if_path_allowed(config: &Config, database: &Database) -> Database {
    if config
        .validate_resolved_path_within_base("DBパス", &database.path)
        .is_ok()
    {
        refresh_database_state(database)
    } else {
        database.clone()
    }
}

fn ensure_databases_found(databases: &[Database]) -> Result<()> {
    if databases.is_empty() {
        bail!("対象DBが見つかりません");
    }
    Ok(())
}

fn validate_runtime_config(config: &Config) -> Result<()> {
    if config.execution.parallel == 0 {
        bail!("execution.parallel は1以上が必要です");
    }
    if config.backup.dir.trim().is_empty() {
        bail!("backup.dir は空にできません");
    }
    config.validate_path_within_base("backup.dir", &config.backup.dir)?;
    config.validate_database_migration_groups()?;
    if let Some(path) = config.audit.path.as_deref() {
        if path.trim().is_empty() {
            bail!("audit.path は空にできません");
        }
        config.validate_path_within_base("audit.path", path)?;
    }
    validate_identifier(config.migrations_table())?;
    Ok(())
}

fn select_databases(config: &Config, selection: &DatabaseSelection) -> Result<Vec<Database>> {
    let mut databases = discover_databases(config)?;
    if let Some(group) = selection.group.as_deref() {
        if group != crate::ALL_DB_GROUP {
            let selectors = config
                .effective_db_groups()
                .get(group)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("指定されたDB groupが見つかりません: {group}"))?;
            databases = select_group_databases(config, &databases, &selectors)?;
            if databases.is_empty() {
                bail!("DB group に一致するDBが見つかりません: {group}");
            }
        }
    }
    if let Some(selector) = selection.database.as_deref() {
        databases.retain(|database| database_matches_selector(config, database, selector));
        if databases.is_empty() {
            bail!("指定されたDBが見つかりません: {selector}");
        }
    }
    if let Some(limit) = selection.limit {
        if limit == 0 {
            bail!("limit は1以上が必要です");
        }
        databases.truncate(limit);
    }
    Ok(databases)
}

fn select_group_databases(
    config: &Config,
    databases: &[Database],
    selectors: &[String],
) -> Result<Vec<Database>> {
    let mut selected = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    for selector in selectors {
        let mut matched = false;
        for database in databases {
            if !database_matches_selector(config, database, selector) {
                continue;
            }
            matched = true;
            let normalized_path = normalize_path_for_comparison(&database.path);
            if seen_ids.insert(database.id.clone()) && seen_paths.insert(normalized_path) {
                selected.push(database.clone());
            }
        }
        if !matched {
            bail!("指定されたDB group selectorが見つかりません: {selector}");
        }
    }
    Ok(selected)
}

fn find_database_by_selector(config: &Config, selector: &str) -> Result<Database> {
    select_databases(
        config,
        &DatabaseSelection {
            database: Some(selector.to_string()),
            ..DatabaseSelection::default()
        },
    )?
    .into_iter()
    .next()
    .ok_or_else(|| anyhow::anyhow!("指定されたDBが見つかりません: {selector}"))
}

fn database_matches_selector(config: &Config, database: &Database, selector: &str) -> bool {
    if database.id == selector {
        return true;
    }
    let selector_path = config.resolve_path(selector);
    if config
        .validate_resolved_path_within_base("DBパス", &selector_path)
        .is_err()
    {
        return false;
    }
    database.path.to_string_lossy() == selector
        || normalize_path_for_comparison(&selector_path)
            == normalize_path_for_comparison(&database.path)
}

fn backup_directory(config: &Config) -> Result<PathBuf> {
    config.validate_path_within_base("backup.dir", &config.backup.dir)?;
    let path = config.resolve_path(&config.backup.dir);
    fs::create_dir_all(&path)
        .with_context(|| format!("backup ディレクトリを作成できません: {}", path.display()))?;
    Ok(path)
}

fn prune_old_backups(
    config: &Config,
    database_dir: &Path,
    protected_paths: &[PathBuf],
) -> Result<()> {
    if config.backup.keep_last == 0 {
        return Ok(());
    }
    let mut entries = fs::read_dir(database_dir)
        .with_context(|| {
            format!(
                "backup ディレクトリを読めません: {}",
                database_dir.display()
            )
        })?
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "db")
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    if entries.len() <= config.backup.keep_last {
        return Ok(());
    }
    let mut removable_count = entries.len() - config.backup.keep_last;
    for entry in entries {
        if removable_count == 0 {
            break;
        }
        if is_protected_backup_path(&entry.path(), protected_paths) {
            continue;
        }
        let _ = fs::remove_file(entry.path());
        removable_count -= 1;
    }
    Ok(())
}

fn is_protected_backup_path(path: &Path, protected_paths: &[PathBuf]) -> bool {
    protected_paths.iter().any(|protected_path| {
        normalize_path_for_comparison(protected_path) == normalize_path_for_comparison(path)
    })
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() || sanitized.chars().all(|ch| ch == '.') {
        "database".to_string()
    } else {
        sanitized
    }
}

fn backup_path_component(database_id: &str) -> String {
    let digest = Sha256::digest(database_id.as_bytes());
    format!(
        "{}-{}",
        sanitize_path_component(database_id),
        hex_encode(&digest[..8])
    )
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn remove_sqlite_database_files(path: &Path) {
    let _ = fs::remove_file(path);
    for suffix in ["db-wal", "db-shm", "db-journal"] {
        let _ = fs::remove_file(path.with_extension(suffix));
    }
}

pub struct DatabaseOperationLock {
    path: PathBuf,
    token: String,
}

impl Drop for DatabaseOperationLock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path).is_ok_and(|content| content.contains(&self.token)) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn acquire_database_operation_lock(
    config: &Config,
    database_path: &Path,
    operation: &str,
) -> Result<DatabaseOperationLock> {
    config.validate_resolved_path_within_base("DBパス", database_path)?;
    ensure_existing_database_file(database_path)?;
    let lock_path = database_operation_lock_path(database_path)?;
    let timeout = Duration::from_millis(config.execution.lock_timeout_ms);
    let started = Instant::now();
    let token = format!("{}-{}", std::process::id(), unix_timestamp_nanos());

    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                writeln!(file, "pid={}", std::process::id())?;
                writeln!(file, "operation={operation}")?;
                writeln!(file, "database={}", database_path.display())?;
                writeln!(file, "token={token}")?;
                file.sync_all()?;
                return Ok(DatabaseOperationLock {
                    path: lock_path,
                    token,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                if started.elapsed() >= timeout {
                    bail!(
                        "DBは別のsqlite-fleet操作中です: {} lock={}",
                        database_path.display(),
                        lock_path.display()
                    );
                }
                let remaining = timeout.saturating_sub(started.elapsed());
                thread::sleep(remaining.min(Duration::from_millis(50)));
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("DB操作ロックを作成できません: {}", lock_path.display())
                });
            }
        }
    }
}

fn database_operation_lock_path(database_path: &Path) -> Result<PathBuf> {
    let parent = database_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("DBパスの親ディレクトリが不正です"))?;
    let file_name = database_path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("DBファイル名が不正です: {}", database_path.display()))?;
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".sqlite-fleet.lock");
    Ok(parent.join(lock_name))
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn unix_timestamp_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn wal_or_shm_size(path: &Path, extension: &str) -> Option<u64> {
    let mut sidecar_path = path.as_os_str().to_os_string();
    sidecar_path.push(format!("-{extension}"));
    fs::metadata(PathBuf::from(sidecar_path))
        .ok()
        .map(|metadata| metadata.len())
}

fn wal_or_shm_size_if_path_allowed(config: &Config, path: &Path, extension: &str) -> Option<u64> {
    if config
        .validate_resolved_path_within_base("DBパス", path)
        .is_ok()
    {
        wal_or_shm_size(path, extension)
    } else {
        None
    }
}
