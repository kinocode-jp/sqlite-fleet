#[derive(Serialize)]
struct ApiEnvelope<T> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

fn api_state(config: &Config, permissions: &sqlite_fleet::GuiConfig) -> ApiEnvelope<StateData> {
    if config.gui_users.is_empty() {
        return ApiEnvelope {
            ok: true,
            data: Some(StateData {
                migration_groups: Vec::new(),
                db_groups: Vec::new(),
                database_migration_rules: Vec::new(),
                database_migration_assignments: Vec::new(),
                gui_permissions: GuiPermissionData::from_permissions(permissions),
                gui_users: None,
                gui_user_setup_available: true,
                settings: SettingsData::from_config(config),
                project: config.project.name.clone(),
                status: sqlite_fleet::StatusReport {
                    database_count: 0,
                    latest_migration: None,
                    up_to_date: 0,
                    pending: 0,
                    failed: 0,
                    missing: 0,
                    corrupt: 0,
                    plans: Vec::new(),
                },
                databases: Vec::new(),
                migrations: Vec::new(),
            }),
            error: None,
        };
    }
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
                    gui_permissions: GuiPermissionData::from_permissions(permissions),
                    gui_users: api_gui_users(config, permissions),
                    gui_user_setup_available: config.gui_users.is_empty(),
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
    let group_configs = config.effective_migration_groups();
    let mut names = group_configs.keys().cloned().collect::<Vec<_>>();
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
                dir: group_configs.get(&name).and_then(|group| group.dir.clone()),
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

fn api_path_entries(config: &Config, dir: Option<&str>) -> Result<PathEntriesData> {
    let base_dir = std::fs::canonicalize(&config.base_dir)
        .with_context(|| format!("base_dir を解決できません: {}", config.base_dir.display()))?;
    let requested = dir.unwrap_or("").trim();
    if requested.contains('*') {
        bail!("参照パスにglobは指定できません");
    }
    let target = if requested.is_empty() || requested == "." {
        base_dir.clone()
    } else {
        config.resolve_path(requested)
    };
    let target = std::fs::canonicalize(&target)
        .with_context(|| format!("参照パスを解決できません: {}", target.display()))?;
    if !target.starts_with(&base_dir) {
        bail!("参照パスはbase_dir配下である必要があります: {}", target.display());
    }
    if !target.is_dir() {
        bail!("参照パスはディレクトリである必要があります: {}", target.display());
    }
    let current = relative_path_string(&base_dir, &target)?;
    let parent = target
        .parent()
        .filter(|parent| parent.starts_with(&base_dir) && *parent != target)
        .map(|parent| relative_path_string(&base_dir, parent))
        .transpose()?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&target)
        .with_context(|| format!("参照パスを読めません: {}", target.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_dir() && !metadata.is_file() {
            continue;
        }
        let canonical_path = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if !canonical_path.starts_with(&base_dir) {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let kind = if metadata.is_dir() { "dir" } else { "file" };
        let modified_at_ms = metadata.modified().ok().and_then(|time| {
            time.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        });
        entries.push(PathEntryData {
            name: name.to_string(),
            path: relative_path_string(&base_dir, &path)?,
            kind: kind.to_string(),
            modified_at_ms,
        });
    }
    entries.sort_by(|a, b| {
        (a.kind != "dir")
            .cmp(&(b.kind != "dir"))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries.truncate(500);
    Ok(PathEntriesData {
        current,
        parent,
        entries,
    })
}

fn allowed_root_data(config: &Config) -> Vec<AllowedRootData> {
    config
        .effective_allowed_roots()
        .into_iter()
        .map(|value| {
            let path = config.resolve_path(&value);
            let resolved = normalize_path_for_display(&path);
            let exists = path.exists();
            let mut warnings = broad_root_warnings(&resolved);
            if !exists {
                warnings.push("root does not exist".to_string());
            }
            AllowedRootData {
                value,
                resolved_path: resolved,
                exists,
                warnings,
            }
        })
        .collect()
}

fn normalize_path_for_display(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn broad_root_warnings(path: &str) -> Vec<String> {
    if path == "/" {
        return vec!["root is broad; confirm this is intentional".to_string()];
    }
    let normalized = path.trim_end_matches(std::path::MAIN_SEPARATOR);
    let broad = matches!(
        normalized,
        "/" | "/Users" | "/home" | "/var" | "C:" | "C:\\"
    ) || normalized.len() == 2 && normalized.ends_with(':');
    if broad {
        vec!["root is broad; confirm this is intentional".to_string()]
    } else {
        Vec::new()
    }
}

fn relative_path_string(base_dir: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(base_dir)
        .with_context(|| format!("base_dirからの相対パスを作れません: {}", path.display()))?;
    if relative.as_os_str().is_empty() {
        return Ok(String::new());
    }
    let parts = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("参照パスはUTF-8である必要があります"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(parts.join("/"))
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
        bail!("ATTACH/DETACH を含むSQLはDry runできません。外部DBへ影響する可能性があるため、内容を確認してから適用してください");
    }
    if sql_contains_vacuum_into(sql) {
        bail!("VACUUM INTO を含むSQLはGUIでは実行できません。外部ファイルを作成する可能性があるため、sqlite3などの外部ツールで実行してください");
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
            "Dry run OK".to_string()
        } else {
            "SQL applied".to_string()
        },
    })
}

fn api_gui_users(
    config: &Config,
    permissions: &sqlite_fleet::GuiConfig,
) -> Option<Vec<GuiUserData>> {
    if config.gui_users.is_empty() || !permissions.allow_gui_permission_edit {
        return None;
    }
    let mut users = config
        .gui_users
        .iter()
        .map(|(name, user)| GuiUserData {
            name: name.clone(),
            token: String::new(),
            permissions: GuiPermissionData::from_permissions(&user.permissions),
        })
        .collect::<Vec<_>>();
    users.sort_by(|left, right| left.name.cmp(&right.name));
    Some(users)
}

#[cfg(test)]
fn api_save_gui_permissions(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request = parse_gui_permission_request(&body)?;
    api_save_gui_permissions_request(state, request)
}

fn parse_gui_permission_request(body: &[u8]) -> Result<GuiPermissionRequest> {
    let value: serde_json::Value =
        serde_json::from_slice(body).context("GUI permissions request body のJSONが不正です")?;
    if matches!(value.get("gui_users"), Some(serde_json::Value::Null)) {
        bail!("gui_users は配列で指定してください");
    }
    serde_json::from_value(value).context("GUI permissions request body のJSONが不正です")
}

fn api_save_gui_permissions_request(
    state: &ServerState,
    request: GuiPermissionRequest,
) -> Result<AdminResult> {
    let mut config = locked_config(state)?;
    if let Some(gui_users) = request.gui_users {
        if gui_users.is_empty() {
            bail!("GUI user は1人以上必要です");
        }
        let mut retained_existing_users = std::collections::HashSet::new();
        for user in &gui_users {
            if user.token.is_empty() && !retained_existing_users.insert(existing_gui_user_name(user)?) {
                bail!("GUI user original_name が重複しています");
            }
        }
        let mut users = HashMap::new();
        let mut submitted_tokens = std::collections::HashSet::new();
        for user in &gui_users {
            let name = clean_name(&user.name, "GUI user")?;
            let token = user.token.trim();
            if token != user.token || token.chars().any(char::is_whitespace) {
                bail!("GUI user token は空白なしの文字列である必要があります");
            }
            let permissions = sqlite_fleet::GuiConfig::from(user);
            let gui_user = if token.is_empty() {
                let existing_name = existing_gui_user_name(user)?;
                let Some(existing) = config.gui_users.get(&existing_name) else {
                    bail!("新しいGUI userにはtokenが必要です");
                };
                existing.with_upgraded_token(permissions)?
            } else {
                if !submitted_tokens.insert(token.to_string()) {
                    bail!("GUI user token が重複しています");
                }
                if config.gui_users.iter().any(|(existing_name, existing)| {
                    existing_name != &name
                        && retained_existing_users.contains(existing_name.as_str())
                        && existing.token_matches(token)
                }) {
                    bail!("GUI user token が重複しています");
                }
                sqlite_fleet::GuiUserConfig::with_hashed_token(token, permissions)?
            };
            if users
                .insert(name, gui_user)
                .is_some()
            {
                bail!("GUI user name が重複しています");
            }
        }
        if !users
            .values()
            .any(|user| user.permissions.allow_gui_permission_edit)
        {
            bail!("少なくとも1人のGUI userに allow_gui_permission_edit が必要です");
        }
        config.gui_users = users;
    } else {
        config.gui.allow_check = request.allow_check;
        config.gui.allow_migrate = request.allow_migrate;
        config.gui.allow_backup = request.allow_backup;
        config.gui.allow_restore = request.allow_restore;
        config.gui.allow_sql_apply = request.allow_sql_apply;
        config.gui.allow_migration_edit = request.allow_migration_edit;
        config.gui.allow_gui_permission_edit = request.allow_gui_permission_edit;
        config.gui.allow_config_edit = request.allow_config_edit;
    }
    persist_config(state, config)?;
    Ok(AdminResult::new("GUI permissions を保存しました".to_string()))
}

fn existing_gui_user_name(user: &GuiUserRequest) -> Result<String> {
    match user.original_name.as_deref() {
        Some(original_name) if !original_name.trim().is_empty() => {
            clean_name(original_name, "GUI user")
        }
        _ => clean_name(&user.name, "GUI user"),
    }
}

fn api_save_settings(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: SettingsRequest =
        serde_json::from_slice(&body).context("settings request body のJSONが不正です")?;
    let mut config = locked_config(state)?;
    config.project.name = clean_optional_string(request.project_name);
    config.security.allowed_roots = clean_path_list(request.allowed_roots)?;
    config.databases.discovery = request.discovery.trim().to_string();
    config.databases.path_glob = clean_optional_string(request.databases_path_glob);
    config.databases.source = clean_optional_string(request.databases_source);
    config.databases.query = clean_optional_string(request.databases_query);
    config.databases.id_column = clean_optional_string(request.databases_id_column);
    config.databases.path_column = clean_optional_string(request.databases_path_column);
    config.databases.path_template = clean_optional_string(request.databases_path_template);
    config.migrations.dir = request.migrations_dir.trim().to_string();
    config.migrations.table = request.migrations_table.trim().to_string();
    config.report.format = request.report_format.trim().to_string();
    config.report.path = clean_optional_string(request.report_path);
    config.backup.dir = request.backup_dir.trim().to_string();
    config.backup.before_migrate = request.backup_before_migrate;
    config.backup.keep_last = request.backup_keep_last;
    config.audit.path = clean_optional_string(request.audit_path);
    config.execution.parallel = request.parallel;
    config.execution.lock_timeout_ms = request.lock_timeout_ms;
    config.execution.continue_on_error = request.continue_on_error;
    persist_config(state, config)?;
    Ok(AdminResult::new("settings を保存しました".to_string()))
}

fn api_preview_discovery(state: &ServerState, body: Vec<u8>) -> Result<DiscoveryPreviewData> {
    let request: SettingsRequest =
        serde_json::from_slice(&body).context("settings request body のJSONが不正です")?;
    let mut config = locked_config(state)?;
    apply_settings_request(&mut config, request)?;
    match config.validate_discovery().and_then(|()| discover_databases(&config)) {
        Ok(databases) => {
            let rows = databases
                .into_iter()
                .map(|database| DiscoveryPreviewDatabase {
                    allowed_root: config.allowed_root_for_path(&database.path).ok().flatten(),
                    id: database.id,
                    path: database.path,
                    exists: database.exists,
                    readable: database.readable,
                    error: None,
                })
                .collect::<Vec<_>>();
            Ok(DiscoveryPreviewData {
                count: rows.len(),
                databases: rows,
                errors: Vec::new(),
            })
        }
        Err(error) => Ok(DiscoveryPreviewData {
            count: 0,
            databases: Vec::new(),
            errors: vec![error.to_string()],
        }),
    }
}

fn apply_settings_request(config: &mut Config, request: SettingsRequest) -> Result<()> {
    config.project.name = clean_optional_string(request.project_name);
    config.security.allowed_roots = clean_path_list(request.allowed_roots)?;
    config.databases.discovery = request.discovery.trim().to_string();
    config.databases.path_glob = clean_optional_string(request.databases_path_glob);
    config.databases.source = clean_optional_string(request.databases_source);
    config.databases.query = clean_optional_string(request.databases_query);
    config.databases.id_column = clean_optional_string(request.databases_id_column);
    config.databases.path_column = clean_optional_string(request.databases_path_column);
    config.databases.path_template = clean_optional_string(request.databases_path_template);
    config.migrations.dir = request.migrations_dir.trim().to_string();
    config.migrations.table = request.migrations_table.trim().to_string();
    config.report.format = request.report_format.trim().to_string();
    config.report.path = clean_optional_string(request.report_path);
    config.backup.dir = request.backup_dir.trim().to_string();
    config.backup.before_migrate = request.backup_before_migrate;
    config.backup.keep_last = request.backup_keep_last;
    config.audit.path = clean_optional_string(request.audit_path);
    config.execution.parallel = request.parallel;
    config.execution.lock_timeout_ms = request.lock_timeout_ms;
    config.execution.continue_on_error = request.continue_on_error;
    Ok(())
}

fn api_save_migration_group(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: MigrationGroupRequest =
        serde_json::from_slice(&body).context("マイグレーショングループ request body のJSONが不正です")?;
    let name = clean_name(&request.name, "マイグレーショングループ名")?;
    let mut migrations = clean_migration_id_list(request.versions)?;
    let dir = clean_optional_string(request.dir);
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
    if dir.is_none() && !existed && name != MAIN_MIGRATION_GROUP && migrations.is_empty() {
        migrations = config
            .migration_groups
            .get(MAIN_MIGRATION_GROUP)
            .map(|group| group.migrations.clone())
            .unwrap_or_default();
    }
    match config.migration_groups.get_mut(&name) {
        Some(group) => {
            if let Some(dir) = dir {
                group.dir = Some(dir);
            }
            group.migrations = migrations;
        }
        None => {
            config.migration_groups.insert(
                name.clone(),
                MigrationGroupConfig { dir, migrations },
            );
        }
    }
    persist_config(state, config)?;
    Ok(AdminResult::new(format!(
        "マイグレーショングループを保存しました: {name}"
    )))
}

fn api_baseline_migrations(state: &ServerState, body: Vec<u8>) -> Result<MigrateReport> {
    let request: BaselineRequest =
        serde_json::from_slice(&body).context("baseline request body のJSONが不正です")?;
    if request.databases.is_empty() {
        bail!("baseline対象DBが選択されていません");
    }
    let config = locked_config(state)?;
    let databases = discover_databases(&config)?;
    let migrations = load_migrations(&config)?;
    let selected = request
        .databases
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let plans = sqlite_fleet::build_plan(&config, &databases, &migrations)
        .into_iter()
        .filter(|plan| selected.contains(&plan.database.id))
        .collect::<Vec<_>>();
    if plans.is_empty() {
        bail!("baseline対象DBが見つかりません");
    }
    let database_count = plans.len();
    let mut results = Vec::new();
    for plan in plans {
        let result = baseline_database(&config, plan, &migrations);
        let success = result.success;
        results.push(result);
        if !config.execution.continue_on_error && !success {
            break;
        }
    }
    let applied_databases = results
        .iter()
        .filter(|result| result.success && !result.applied.is_empty())
        .count();
    let pending_databases = results
        .iter()
        .filter(|result| !result.pending.is_empty())
        .count();
    let failed_databases = results.iter().filter(|result| !result.success).count();
    Ok(MigrateReport {
        dry_run: false,
        database_count,
        processed_databases: results.len(),
        pending_databases,
        applied_databases,
        failed_databases,
        databases: results,
    })
}

fn baseline_database(
    config: &Config,
    plan: sqlite_fleet::DatabasePlan,
    migrations: &[sqlite_fleet::Migration],
) -> DatabaseMigrationResult {
    let _operation_lock =
        match acquire_database_operation_lock(config, &plan.database.path, "baseline") {
            Ok(lock) => lock,
            Err(error) => {
                return baseline_failed_result(plan.database, plan.pending, error.to_string());
            }
        };
    let plan = sqlite_fleet::build_database_plan(config, &plan.database, migrations);
    let pending = plan.pending.clone();
    if let Some(error) = plan.error {
        return baseline_failed_result(plan.database, pending, error);
    }
    if !plan.unknown_applied.is_empty() {
        return baseline_failed_result(
            plan.database,
            pending,
            "不明な適用履歴があるためbaseline登録できません".to_string(),
        );
    }
    if !plan.checksum_errors.is_empty() {
        return baseline_failed_result(
            plan.database,
            pending,
            "チェックサム不一致があるためbaseline登録できません".to_string(),
        );
    }
    if pending.is_empty() {
        return DatabaseMigrationResult {
            database: plan.database,
            applied: Vec::new(),
            pending: Vec::new(),
            pre_backup: None,
            success: true,
            error: None,
        };
    }
    let database_migrations = sqlite_fleet::migrations_for_database(config, &plan.database, migrations);
    let mut conn = match open_gui_database(config, &plan.database, false) {
        Ok(conn) => conn,
        Err(error) => return baseline_failed_result(plan.database, pending, error.to_string()),
    };
    if let Err(error) =
        migrate_legacy_migrations_table(&mut conn, config.migrations_table(), &database_migrations)
    {
        return baseline_failed_result(plan.database, pending, error.to_string());
    }
    drop(conn);
    let plan = sqlite_fleet::build_database_plan(config, &plan.database, migrations);
    let pending = plan.pending.clone();
    if let Some(error) = plan.error {
        return baseline_failed_result(plan.database, pending, error);
    }
    if !plan.unknown_applied.is_empty() {
        return baseline_failed_result(
            plan.database,
            pending,
            "不明な適用履歴があるためbaseline登録できません".to_string(),
        );
    }
    if !plan.checksum_errors.is_empty() {
        return baseline_failed_result(
            plan.database,
            pending,
            "チェックサム不一致があるためbaseline登録できません".to_string(),
        );
    }
    if pending.is_empty() {
        return DatabaseMigrationResult {
            database: plan.database,
            applied: Vec::new(),
            pending: Vec::new(),
            pre_backup: None,
            success: true,
            error: None,
        };
    }
    let mut conn = match open_gui_database(config, &plan.database, false) {
        Ok(conn) => conn,
        Err(error) => return baseline_failed_result(plan.database, pending, error.to_string()),
    };
    if let Err(error) = ensure_migrations_table(&conn, config.migrations_table()) {
        return baseline_failed_result(plan.database, pending, error.to_string());
    }
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(error) => return baseline_failed_result(plan.database, pending, error.to_string()),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0);
    for migration in &pending {
        if let Err(error) = tx.execute(
            &format!(
                "INSERT INTO main.{} (filename, version, name, checksum, applied_at, execution_ms) VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                config.migrations_table()
            ),
            rusqlite::params![
                migration.filename,
                migration.version,
                migration.name,
                migration.checksum,
                now
            ],
        ) {
            return baseline_failed_result(plan.database, pending, error.to_string());
        }
    }
    if let Err(error) = tx.commit() {
        return baseline_failed_result(plan.database, pending, error.to_string());
    }
    DatabaseMigrationResult {
        database: plan.database,
        applied: pending,
        pending: Vec::new(),
        pre_backup: None,
        success: true,
        error: None,
    }
}

fn baseline_failed_result(
    database: sqlite_fleet::Database,
    pending: Vec<sqlite_fleet::MigrationSummary>,
    error: String,
) -> DatabaseMigrationResult {
    DatabaseMigrationResult {
        database,
        applied: Vec::new(),
        pending,
        pre_backup: None,
        success: false,
        error: Some(error),
    }
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
        serde_json::from_slice(&body).context("DBグループ request body のJSONが不正です")?;
    let name = clean_name(&request.name, "DBグループ名")?;
    let selectors = clean_list(request.selectors, "selectors")?;
    let mut config = locked_config(state)?;
    config.db_groups.insert(name.clone(), selectors);
    persist_config(state, config)?;
    Ok(AdminResult::new(format!("DBグループを保存しました: {name}")))
}

fn api_save_database_migration_group(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: DatabaseMigrationGroupRequest = serde_json::from_slice(&body)
        .context("DB別マイグレーショングループ request body のJSONが不正です")?;
    let selector = clean_name(&request.selector, "DB selector")?;
    let groups = clean_list_allow_empty(request.groups, "マイグレーショングループ")?;
    let mut config = locked_config(state)?;
    config
        .database_migration_groups
        .insert(selector.clone(), groups);
    persist_config(state, config)?;
    Ok(AdminResult::new(format!(
        "DBのマイグレーショングループ割当を保存しました: {selector}"
    )))
}

fn api_create_migration_file(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: MigrationFileRequest =
        serde_json::from_slice(&body).context("マイグレーションファイル request body のJSONが不正です")?;
    let (filename, version, name) = match request.filename.as_deref() {
        Some(filename) => {
            let filename = filename.trim();
            let (version, name) = sqlite_fleet::parse_migration_file_name(filename)?;
            (filename.to_string(), version, name)
        }
        None => {
            let version = clean_version(&request.version)?;
            let name = clean_file_stem(&request.name, "マイグレーション名")?;
            (format!("{version}_{name}.sql"), version, name)
        }
    };
    let request_version = clean_version(&request.version)?;
    let request_name = clean_file_stem(&request.name, "マイグレーション名")?;
    if request_version != version || request_name != name {
        bail!("マイグレーションファイル名と version/name が一致しません");
    }
    let sql = request.sql.trim();
    if sql.is_empty() {
        bail!("マイグレーションSQLは空にできません");
    }
    if utf8_byte_len(sql) > MAX_SQL_BYTES {
        bail!("マイグレーションSQLが大きすぎます");
    }
    let mut config = locked_config(state)?;
    let target_group = request
        .group
        .as_deref()
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .map(|group| clean_name(group, "マイグレーショングループ名"))
        .transpose()?;
    if !config.migration_groups.is_empty() && target_group.is_none() {
        bail!("明示的な migration_groups がある設定ではマイグレーショングループの指定が必要です");
    }
    let migrations_dir_value = target_group
        .as_deref()
        .and_then(|group| config.migration_groups.get(group))
        .and_then(|group| group.dir.as_deref())
        .unwrap_or(&config.migrations.dir);
    let migrations_dir = resolve_existing_or_creatable_dir(&config, migrations_dir_value)?;
    std::fs::create_dir_all(&migrations_dir).with_context(|| {
        format!(
            "マイグレーションディレクトリを作成できません: {}",
            migrations_dir.display()
        )
    })?;
    let path = migrations_dir.join(&filename);
    validate_path_stays_in_base(&config, &path, "マイグレーションファイル")?;
    create_new_file_no_symlink(&path, sql.as_bytes(), "マイグレーションファイル")?;
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
        "マイグレーションファイルを作成しました: {}",
        path.display()
    )))
}

fn api_update_migration_file(state: &ServerState, body: Vec<u8>) -> Result<AdminResult> {
    let request: MigrationFileUpdateRequest =
        serde_json::from_slice(&body).context("マイグレーションファイル更新 request body のJSONが不正です")?;
    let request_version = clean_version(&request.version)?;
    let group = clean_name(&request.group, "マイグレーショングループ名")?;
    let sql = request.sql.trim();
    if sql.is_empty() {
        bail!("マイグレーションSQLは空にできません");
    }
    if utf8_byte_len(sql) > MAX_SQL_BYTES {
        bail!("マイグレーションSQLが大きすぎます");
    }
    let config = locked_config(state)?;
    let databases = discover_databases(&config)?;
    let migrations = load_migrations(&config)?;
    let requested_path = PathBuf::from(request.path.trim());
    if requested_path.as_os_str().is_empty() {
        bail!("マイグレーションファイルパスは空にできません");
    }
    validate_path_stays_in_base(&config, &requested_path, "マイグレーションファイル")?;
    let filename = requested_path
        .file_name()
        .and_then(|filename| filename.to_str())
        .ok_or_else(|| anyhow::anyhow!("マイグレーションファイル名が不正です"))?;
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
        .ok_or_else(|| anyhow::anyhow!("指定されたマイグレーションファイルが見つかりません"))?;
    let previous_sql = std::fs::read_to_string(&migration.path).with_context(|| {
        format!(
            "マイグレーションファイルの現在内容を読めません: {}",
            migration.path.display()
        )
    })?;
    std::fs::write(&migration.path, sql)
        .with_context(|| format!("マイグレーションファイルを更新できません: {}", migration.path.display()))?;
    if let Err(error) = load_migrations(&config) {
        let _ = std::fs::write(&migration.path, previous_sql);
        return Err(error);
    }
    Ok(AdminResult::new(format!(
        "マイグレーションファイルを更新しました: {}",
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("DB directory を作成できません: {}", parent.display()))?;
    }
    create_new_file_no_symlink(&path, &[], "DB file")?;
    Connection::open(&path)
        .with_context(|| format!("DB file を作成できません: {}", path.display()))?;
    if let Some(group) = request
        .db_group
        .as_deref()
        .map(str::trim)
        .filter(|group| !group.is_empty())
    {
        let group = clean_name(group, "DBグループ名")?;
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

fn create_new_file_no_symlink(path: &Path, content: &[u8], label: &str) -> Result<()> {
    if let Ok(metadata) = std::fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            bail!("{label} はシンボリックリンクにできません: {}", path.display());
        }
        bail!("{label} は既に存在します: {}", path.display());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("{label} を作成できません: {}", path.display()))?;
    file.write_all(content)
        .with_context(|| format!("{label} を書き込めません: {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("{label} を保存できません: {}", path.display()))?;
    Ok(())
}
