pub fn schema_drift(config: &Config, selection: DatabaseSelection) -> Result<SchemaDriftReport> {
    validate_runtime_config(config)?;
    let databases = select_databases(config, &selection)?;
    ensure_databases_found(&databases)?;
    let mut baseline: Option<(Database, BTreeMap<String, String>)> = None;
    let mut results = Vec::new();
    for database in &databases {
        match read_schema_signature(config, database) {
            Ok(signature) => {
                if let Some((_, baseline_signature)) = &baseline {
                    let result =
                        compare_schema_signature(database.clone(), baseline_signature, &signature);
                    results.push(result);
                } else {
                    baseline = Some((database.clone(), signature));
                    results.push(DatabaseSchemaDriftResult {
                        database: database.clone(),
                        matches_baseline: true,
                        missing_objects: Vec::new(),
                        extra_objects: Vec::new(),
                        changed_objects: Vec::new(),
                        success: true,
                        error: None,
                    });
                }
            }
            Err(error) => results.push(DatabaseSchemaDriftResult {
                database: refresh_database_state_if_path_allowed(config, database),
                matches_baseline: false,
                missing_objects: Vec::new(),
                extra_objects: Vec::new(),
                changed_objects: Vec::new(),
                success: false,
                error: Some(error.to_string()),
            }),
        }
    }
    let drifted = results
        .iter()
        .filter(|result| result.success && !result.matches_baseline)
        .count();
    let failed = results.iter().filter(|result| !result.success).count();
    Ok(SchemaDriftReport {
        database_count: results.len(),
        baseline_database: baseline.map(|(database, _)| database),
        drifted,
        failed,
        databases: results,
    })
}

fn read_schema_signature(config: &Config, database: &Database) -> Result<BTreeMap<String, String>> {
    config.validate_resolved_path_within_base("DBパス", &database.path)?;
    ensure_existing_database_file(&database.path)?;
    let conn = open_existing_readonly(&database.path)
        .with_context(|| format!("DBを開けません: {}", database.path.display()))?;
    conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let mut stmt = conn.prepare(
        "SELECT type, name, COALESCE(sql, '')
         FROM sqlite_schema
         WHERE type IN ('table', 'index', 'view', 'trigger')
           AND name NOT GLOB 'sqlite_*'
           AND name <> ?1
         ORDER BY type, name",
    )?;
    let rows = stmt.query_map([config.migrations_table()], |row| {
        let object_type: String = row.get(0)?;
        let name: String = row.get(1)?;
        let sql: String = row.get(2)?;
        Ok((format!("{object_type}:{name}"), normalize_schema_sql(&sql)))
    })?;
    rows.collect::<std::result::Result<BTreeMap<_, _>, _>>()
        .map_err(Into::into)
}

fn compare_schema_signature(
    database: Database,
    baseline: &BTreeMap<String, String>,
    actual: &BTreeMap<String, String>,
) -> DatabaseSchemaDriftResult {
    let missing_objects = baseline
        .keys()
        .filter(|key| !actual.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    let extra_objects = actual
        .keys()
        .filter(|key| !baseline.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    let changed_objects = baseline
        .iter()
        .filter(|(key, expected)| actual.get(*key).is_some_and(|value| value != *expected))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    let matches_baseline =
        missing_objects.is_empty() && extra_objects.is_empty() && changed_objects.is_empty();
    DatabaseSchemaDriftResult {
        database,
        matches_baseline,
        missing_objects,
        extra_objects,
        changed_objects,
        success: true,
        error: None,
    }
}

pub fn write_audit_event<T: Serialize>(config: &Config, operation: &str, value: &T) -> Result<()> {
    let Some(path) = config.audit.path.as_deref() else {
        return Ok(());
    };
    if path.trim().is_empty() {
        bail!("audit.path は空にできません");
    }
    let path = config.resolve_path(path);
    config.validate_resolved_path_within_base("audit.path", &path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("audit ディレクトリを作成できません: {}", parent.display()))?;
    }
    let event = serde_json::json!({
        "timestamp": unix_timestamp(),
        "operation": operation,
        "result": value,
    });
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("audit log を開けません: {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(&event)?)
        .with_context(|| format!("audit log を書き込めません: {}", path.display()))
}

pub fn check_database(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
) -> DatabaseCheckResult {
    match check_database_inner(config, database, migrations) {
        Ok(result) => result,
        Err(error) => DatabaseCheckResult {
            database: refresh_database_state_if_path_allowed(config, database),
            quick_check: None,
            integrity_check: None,
            wal_bytes: wal_or_shm_size_if_path_allowed(config, &database.path, "wal"),
            shm_bytes: wal_or_shm_size_if_path_allowed(config, &database.path, "shm"),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            success: false,
            error: Some(error.to_string()),
        },
    }
}

fn check_database_inner(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
) -> Result<DatabaseCheckResult> {
    let migrations = migrations_for_database(config, database, migrations);
    let known_versions = migrations
        .iter()
        .map(|migration| migration.version.as_str())
        .collect::<HashSet<_>>();
    validate_runtime_config(config)?;
    validate_database_id(&database.id)?;
    validate_migrations(config, &migrations)?;
    config.validate_resolved_path_within_base("DBパス", &database.path)?;
    let database = refresh_database_state(database);
    ensure_existing_database_file(&database.path)?;
    let conn = open_existing_readonly(&database.path)
        .with_context(|| format!("DBを開けません: {}", database.path.display()))?;
    conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let quick_check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .context("PRAGMA quick_check に失敗しました")?;
    let integrity_check: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .context("PRAGMA integrity_check に失敗しました")?;
    let applied = read_applied_migrations(&conn, config.migrations_table())?;
    let known: HashMap<&str, &Migration> =
        migrations.iter().map(|m| (m.version.as_str(), m)).collect();
    let checksum_errors = applied
        .iter()
        .filter_map(|applied| {
            known.get(applied.version.as_str()).and_then(|migration| {
                (applied.checksum != migration.checksum).then(|| ChecksumError {
                    version: applied.version.clone(),
                    expected: migration.checksum.clone(),
                    actual: applied.checksum.clone(),
                })
            })
        })
        .collect::<Vec<_>>();
    let unknown_applied = applied
        .iter()
        .filter(|migration| !known_versions.contains(migration.version.as_str()))
        .map(MigrationSummary::from)
        .collect::<Vec<_>>();
    let success = quick_check == "ok"
        && integrity_check == "ok"
        && checksum_errors.is_empty()
        && unknown_applied.is_empty();
    Ok(DatabaseCheckResult {
        database: database.clone(),
        quick_check: Some(quick_check),
        integrity_check: Some(integrity_check),
        wal_bytes: wal_or_shm_size(&database.path, "wal"),
        shm_bytes: wal_or_shm_size(&database.path, "shm"),
        checksum_errors,
        unknown_applied,
        success,
        error: None,
    })
}
