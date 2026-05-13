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
        (Ok(status), Ok(databases), Ok(migrations)) => {
            let migration_data = match api_migrations(config, &databases, &migrations) {
                Ok(migration_data) => migration_data,
                Err(error) => {
                    return ApiEnvelope {
                        ok: false,
                        data: None,
                        error: Some(error.to_string()),
                    };
                }
            };
            ApiEnvelope {
                ok: true,
                data: Some(StateData {
                    migration_groups: api_migration_groups(config, &databases, &migrations),
                    db_groups: api_db_groups(config, &databases),
                    database_migration_rules: api_database_migration_rules(config),
                    database_migration_assignments: api_database_migration_assignments(
                        config, &databases,
                    ),
                    gui_permissions: GuiPermissionData::from_config(config),
                    settings: SettingsData::from_config(config),
                    project: config.project.name.clone(),
                    status,
                    databases,
                    migrations: migration_data,
                }),
                error: None,
            }
        }
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

fn api_migrations(
    config: &Config,
    databases: &[sqlite_fleet::Database],
    migrations: &[sqlite_fleet::Migration],
) -> Result<Vec<MigrationData>> {
    migrations
        .iter()
        .map(|migration| {
            Ok(MigrationData {
                group: migration.group.clone(),
                filename: migration.filename.clone(),
                version: migration.version.clone(),
                name: migration.name.clone(),
                checksum: migration.checksum.clone(),
                path: migration.path.clone(),
                sql: migration.sql.clone(),
                applied_databases: applied_databases_for_filename(
                    config,
                    databases,
                    migrations,
                    &migration.filename,
                    false,
                )?,
            })
        })
        .collect()
}

fn applied_databases_for_filename(
    config: &Config,
    databases: &[sqlite_fleet::Database],
    migrations: &[sqlite_fleet::Migration],
    filename: &str,
    strict: bool,
) -> Result<Vec<String>> {
    let mut applied_databases = Vec::new();
    for database in databases {
        if !database.exists {
            continue;
        }
        let applied = match open_gui_database(config, database, true)
            .and_then(|conn| {
                read_applied_migrations_with_catalog(&conn, config.migrations_table(), migrations)
            })
        {
            Ok(applied) => applied,
            Err(error) if strict => return Err(error),
            Err(_) => continue,
        };
        if applied.iter().any(|migration| migration.filename == filename) {
            applied_databases.push(database.id.clone());
        }
    }
    Ok(applied_databases)
}

fn api_migration_groups(
    config: &Config,
    databases: &[sqlite_fleet::Database],
    migrations: &[sqlite_fleet::Migration],
) -> Vec<MigrationGroupData> {
    let mut names = config
        .effective_migration_groups()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let group_migrations = migrations
                .iter()
                .filter(|migration| migration.group == name)
                .map(sqlite_fleet::MigrationSummary::from)
                .collect::<Vec<_>>();
            let group_databases = databases
                .iter()
                .filter(|database| {
                    config
                        .migration_groups_for_database(database)
                        .iter()
                        .any(|group| group == &name)
                })
                .map(|database| database.id.clone())
                .collect::<Vec<_>>();
            MigrationGroupData {
                name,
                migrations: group_migrations,
                databases: group_databases,
            }
        })
        .collect()
}

fn api_db_groups(config: &Config, databases: &[sqlite_fleet::Database]) -> Vec<DbGroupData> {
    let mut groups = config
        .effective_db_groups()
        .into_iter()
        .map(|(name, selectors)| {
            let mut database_ids = Vec::new();
            for selector in &selectors {
                for database in databases {
                    if config.database_matches_selector(database, selector)
                        && !database_ids.contains(&database.id)
                    {
                        database_ids.push(database.id.clone());
                    }
                }
            }
            DbGroupData {
                name,
                selectors,
                database_ids,
            }
        })
        .collect::<Vec<_>>();
    if !groups.iter().any(|group| group.name == ALL_DB_GROUP) {
        groups.push(DbGroupData {
            name: ALL_DB_GROUP.to_string(),
            selectors: vec!["*".to_string()],
            database_ids: databases.iter().map(|database| database.id.clone()).collect(),
        });
    }
    groups.sort_by(|left, right| left.name.cmp(&right.name));
    groups
}

fn api_database_migration_rules(config: &Config) -> Vec<DatabaseMigrationRuleData> {
    let mut rules = config
        .database_migration_groups
        .iter()
        .map(|(selector, groups)| {
            let mut migration_groups = groups.clone();
            migration_groups.sort();
            migration_groups.dedup();
            DatabaseMigrationRuleData {
                selector: selector.clone(),
                migration_groups,
            }
        })
        .collect::<Vec<_>>();
    rules.sort_by(|left, right| left.selector.cmp(&right.selector));
    rules
}

fn api_database_migration_assignments(
    config: &Config,
    databases: &[sqlite_fleet::Database],
) -> Vec<DatabaseMigrationAssignmentData> {
    let mut assignments = Vec::new();
    for database in databases {
        for (selector, groups) in &config.database_migration_groups {
            if config.database_matches_selector(database, selector) {
                let mut migration_groups = groups.clone();
                migration_groups.sort();
                migration_groups.dedup();
                assignments.push(DatabaseMigrationAssignmentData {
                    database_id: database.id.clone(),
                    selector: selector.clone(),
                    migration_groups,
                });
            }
        }
    }
    assignments.sort_by(|left, right| {
        left.database_id
            .cmp(&right.database_id)
            .then_with(|| left.selector.cmp(&right.selector))
    });
    assignments
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

fn api_save_migration_group(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: MigrationGroupRequest =
        serde_json::from_slice(&body).context("migration group request body のJSONが不正です")?;
    let name = clean_name(&request.name, "Migration group name")?;
    let mut migrations = clean_migration_id_list(request.versions)?;
    let mut config = locked_config(state)?;
    let existed = config.migration_groups.contains_key(&name);
    if config.migration_groups.is_empty() && name != MAIN_MIGRATION_GROUP {
        let main_versions = if config.resolve_path(&config.migrations.dir).exists() {
            load_migrations(&config)?
                .into_iter()
                .map(|migration| migration.filename)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        config.migration_groups.insert(
            MAIN_MIGRATION_GROUP.to_string(),
            MigrationGroupConfig::versions(main_versions),
        );
    }
    if !existed && name != MAIN_MIGRATION_GROUP && migrations.is_empty() {
        migrations = config
            .migration_groups
            .get(MAIN_MIGRATION_GROUP)
            .map(|group| group.migrations.clone())
            .unwrap_or_default();
    }
    match config.migration_groups.get_mut(&name) {
        Some(group) => group.migrations = migrations,
        None => {
            config
                .migration_groups
                .insert(name.clone(), MigrationGroupConfig::versions(migrations));
        }
    }
    persist_config(state, config)?;
    Ok(AdminResult::new(format!(
        "migration group を保存しました: {name}"
    )))
}

fn clean_migration_id_list(values: Vec<String>) -> Result<Vec<String>> {
    let mut cleaned = Vec::new();
    for value in values {
        let value = clean_migration_id(&value)?;
        if !cleaned.contains(&value) {
            cleaned.push(value);
        }
    }
    Ok(cleaned)
}

fn api_save_db_group(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: DbGroupRequest =
        serde_json::from_slice(&body).context("DB group request body のJSONが不正です")?;
    let name = clean_name(&request.name, "DB group name")?;
    let selectors = clean_list(request.selectors, "selectors")?;
    let mut config = locked_config(state)?;
    config.db_groups.insert(name.clone(), selectors);
    persist_config(state, config)?;
    Ok(AdminResult::new(format!("DB group を保存しました: {name}")))
}

fn api_save_database_migration_group(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: DatabaseMigrationGroupRequest = serde_json::from_slice(&body)
        .context("database migration group request body のJSONが不正です")?;
    let selector = clean_name(&request.selector, "DB selector")?;
    let groups = clean_list_allow_empty(request.groups, "migration groups")?;
    let mut config = locked_config(state)?;
    config
        .database_migration_groups
        .insert(selector.clone(), groups);
    persist_config(state, config)?;
    Ok(AdminResult::new(format!(
        "DBのmigration group割当を保存しました: {selector}"
    )))
}

fn api_create_migration_file(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: MigrationFileRequest =
        serde_json::from_slice(&body).context("migration file request body のJSONが不正です")?;
    let (filename, version, name) = match request.filename.as_deref() {
        Some(filename) => {
            let filename = filename.trim();
            let (version, name) = sqlite_fleet::parse_migration_file_name(filename)?;
            (filename.to_string(), version, name)
        }
        None => {
            let version = clean_version(&request.version)?;
            let name = clean_file_stem(&request.name, "migration name")?;
            (format!("{version}_{name}.sql"), version, name)
        }
    };
    let request_version = clean_version(&request.version)?;
    let request_name = clean_file_stem(&request.name, "migration name")?;
    if request_version != version || request_name != name {
        bail!("migration file name と version/name が一致しません");
    }
    let sql = request.sql.trim();
    if sql.is_empty() {
        bail!("migration SQL は空にできません");
    }
    if utf8_byte_len(sql) > MAX_SQL_BYTES {
        bail!("migration SQL が大きすぎます");
    }
    let mut config = locked_config(state)?;
    let target_group = request
        .group
        .as_deref()
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(|group| clean_name(group, "Migration group name"))
        .transpose()?;
    if !config.migration_groups.is_empty() && target_group.is_none() {
        bail!("明示的な migration_groups がある設定では migration group の指定が必要です");
    }
    let migrations_dir_value = target_group
        .as_deref()
        .and_then(|group| config.migration_groups.get(group))
        .and_then(|group| group.dir.as_deref())
        .unwrap_or(&config.migrations.dir);
    let migrations_dir = resolve_existing_or_creatable_dir(&config, migrations_dir_value)?;
    std::fs::create_dir_all(&migrations_dir).with_context(|| {
        format!(
            "migrations.dir を作成できません: {}",
            migrations_dir.display()
        )
    })?;
    let path = migrations_dir.join(&filename);
    validate_path_stays_in_base(&config, &path, "migration file")?;
    if path.exists() {
        bail!("migration file は既に存在します: {}", path.display());
    }
    std::fs::write(&path, sql)
        .with_context(|| format!("migration file を作成できません: {}", path.display()))?;
    let preserves_implicit_main =
        config.migration_groups.is_empty() && target_group.as_deref() == Some(MAIN_MIGRATION_GROUP);
    if let Some(group) = target_group.filter(|_| !preserves_implicit_main) {
        let entry = config
            .migration_groups
            .entry(group)
            .or_insert_with(|| MigrationGroupConfig::versions(Vec::new()));
        let tracks_all_dir_migrations = entry.dir.is_some() && entry.migrations.is_empty();
        if !tracks_all_dir_migrations && !entry.migrations.iter().any(|item| item == &filename) {
            entry.migrations.push(filename.clone());
        }
    }
    if let Err(error) = load_migrations(&config) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    if let Err(error) = persist_config(state, config) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(AdminResult::new(format!(
        "migration file を作成しました: {}",
        path.display()
    )))
}

fn api_update_migration_file(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: MigrationFileUpdateRequest =
        serde_json::from_slice(&body).context("migration file update request body のJSONが不正です")?;
    let request_version = clean_version(&request.version)?;
    let group = clean_name(&request.group, "Migration group name")?;
    let sql = request.sql.trim();
    if sql.is_empty() {
        bail!("migration SQL は空にできません");
    }
    if utf8_byte_len(sql) > MAX_SQL_BYTES {
        bail!("migration SQL が大きすぎます");
    }
    let config = locked_config(state)?;
    let databases = discover_databases(&config)?;
    let migrations = load_migrations(&config)?;
    let requested_path = PathBuf::from(request.path.trim());
    if requested_path.as_os_str().is_empty() {
        bail!("migration file path は空にできません");
    }
    validate_path_stays_in_base(&config, &requested_path, "migration file")?;
    let filename = requested_path
        .file_name()
        .and_then(|filename| filename.to_str())
        .ok_or_else(|| anyhow::anyhow!("migration file name が不正です"))?;
    let filename = clean_migration_id(filename)?;
    let applied_databases =
        applied_databases_for_filename(&config, &databases, &migrations, &filename, true)?;
    if !applied_databases.is_empty() {
        bail!(
            "このmigrationは既にDBへ適用済みのため編集できません: {}",
            applied_databases.join(", ")
        );
    }
    let migration = migrations
        .iter()
        .find(|migration| {
            migration.group == group
                && migration.filename == filename
                && migration.version == request_version
                && paths_equal(&migration.path, &requested_path)
        })
        .ok_or_else(|| anyhow::anyhow!("指定されたmigration fileが見つかりません"))?;
    let previous_sql = std::fs::read_to_string(&migration.path).with_context(|| {
        format!(
            "migration file の現在内容を読めません: {}",
            migration.path.display()
        )
    })?;
    std::fs::write(&migration.path, sql)
        .with_context(|| format!("migration file を更新できません: {}", migration.path.display()))?;
    if let Err(error) = load_migrations(&config) {
        let _ = std::fs::write(&migration.path, previous_sql);
        return Err(error);
    }
    Ok(AdminResult::new(format!(
        "migration file を更新しました: {}",
        migration.path.display()
    )))
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn api_create_database_file(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: DatabaseFileRequest =
        serde_json::from_slice(&body).context("database file request body のJSONが不正です")?;
    let relative_path = clean_relative_path(&request.path, "DB path")?;
    let mut config = locked_config(state)?;
    let path = config.resolve_path(&relative_path);
    validate_path_stays_in_base(&config, &path, "DB path")?;
    if path.exists() {
        bail!("DB file は既に存在します: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("DB directory を作成できません: {}", parent.display()))?;
    }
    Connection::open(&path)
        .with_context(|| format!("DB file を作成できません: {}", path.display()))?;
    if let Some(group) = request
        .db_group
        .as_deref()
        .map(str::trim)
        .filter(|group| !group.is_empty())
    {
        let group = clean_name(group, "DB group name")?;
        let entry = config.db_groups.entry(group).or_default();
        if !entry.iter().any(|item| item == &relative_path) {
            entry.push(relative_path.clone());
        }
    }
    if let Err(error) = persist_config(state, config) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(AdminResult::new(format!(
        "DB file を作成しました: {}",
        path.display()
    )))
}
