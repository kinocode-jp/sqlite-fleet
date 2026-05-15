#[derive(Serialize)]
struct StateData {
    project: Option<String>,
    status: sqlite_fleet::StatusReport,
    databases: Vec<sqlite_fleet::Database>,
    migrations: Vec<MigrationData>,
    migration_groups: Vec<MigrationGroupData>,
    db_groups: Vec<DbGroupData>,
    database_migration_rules: Vec<DatabaseMigrationRuleData>,
    database_migration_assignments: Vec<DatabaseMigrationAssignmentData>,
    gui_permissions: GuiPermissionData,
    settings: SettingsData,
}

#[derive(Serialize)]
struct MigrationData {
    group: String,
    filename: String,
    version: String,
    name: String,
    checksum: String,
    path: PathBuf,
    sql: String,
    applied_databases: Vec<String>,
}

#[derive(Serialize)]
struct MigrationGroupData {
    name: String,
    dir: Option<String>,
    migrations: Vec<sqlite_fleet::MigrationSummary>,
    databases: Vec<String>,
}

#[derive(Serialize)]
struct DbGroupData {
    name: String,
    selectors: Vec<String>,
    database_ids: Vec<String>,
}

#[derive(Serialize)]
struct DatabaseMigrationRuleData {
    selector: String,
    migration_groups: Vec<String>,
}

#[derive(Serialize)]
struct DatabaseMigrationAssignmentData {
    database_id: String,
    selector: String,
    migration_groups: Vec<String>,
}

#[derive(Serialize)]
struct GuiPermissionData {
    allow_check: bool,
    allow_migrate: bool,
    allow_backup: bool,
    allow_restore: bool,
    allow_sql_apply: bool,
    allow_migration_edit: bool,
    allow_gui_permission_edit: bool,
    allow_config_edit: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuiPermissionRequest {
    allow_check: bool,
    allow_migrate: bool,
    allow_backup: bool,
    allow_restore: bool,
    allow_sql_apply: bool,
    allow_migration_edit: bool,
    allow_gui_permission_edit: bool,
    allow_config_edit: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsRequest {
    project_name: Option<String>,
    #[serde(default)]
    allowed_roots: Vec<String>,
    discovery: String,
    databases_path_glob: Option<String>,
    databases_source: Option<String>,
    databases_query: Option<String>,
    databases_id_column: Option<String>,
    databases_path_column: Option<String>,
    databases_path_template: Option<String>,
    migrations_dir: String,
    migrations_table: String,
    report_format: String,
    report_path: Option<String>,
    backup_dir: String,
    backup_before_migrate: bool,
    backup_keep_last: usize,
    audit_path: Option<String>,
    parallel: usize,
    lock_timeout_ms: u64,
    continue_on_error: bool,
}

#[derive(Serialize)]
struct PathEntryData {
    name: String,
    path: String,
    kind: String,
    modified_at_ms: Option<u64>,
}

#[derive(Serialize)]
struct PathEntriesData {
    current: String,
    parent: Option<String>,
    entries: Vec<PathEntryData>,
}

impl GuiPermissionData {
    fn from_config(config: &Config) -> Self {
        Self {
            allow_check: config.gui.allow_check,
            allow_migrate: config.gui.allow_migrate,
            allow_backup: config.gui.allow_backup,
            allow_restore: config.gui.allow_restore,
            allow_sql_apply: config.gui.allow_sql_apply,
            allow_migration_edit: config.gui.allow_migration_edit,
            allow_gui_permission_edit: config.gui.allow_gui_permission_edit,
            allow_config_edit: config.gui.allow_config_edit,
        }
    }
}

#[derive(Serialize)]
struct SettingsData {
    project_name: Option<String>,
    allowed_roots: Vec<AllowedRootData>,
    discovery: String,
    databases_path_glob: Option<String>,
    databases_source: Option<String>,
    databases_query: Option<String>,
    databases_id_column: Option<String>,
    databases_path_column: Option<String>,
    databases_path_template: Option<String>,
    migrations_dir: String,
    migrations_table: String,
    migration_group_count: usize,
    database_migration_rule_count: usize,
    db_group_count: usize,
    backup_dir: String,
    backup_before_migrate: bool,
    backup_keep_last: usize,
    audit_path: Option<String>,
    report_path: Option<String>,
    report_format: String,
    parallel: usize,
    lock_timeout_ms: u64,
    continue_on_error: bool,
}

#[derive(Serialize)]
struct AllowedRootData {
    value: String,
    resolved_path: String,
    exists: bool,
    warnings: Vec<String>,
}

impl SettingsData {
    fn from_config(config: &Config) -> Self {
        Self {
            project_name: config.project.name.clone(),
            allowed_roots: allowed_root_data(config),
            discovery: config.databases.discovery.clone(),
            databases_path_glob: config.databases.path_glob.clone(),
            databases_source: config.databases.source.clone(),
            databases_query: config.databases.query.clone(),
            databases_id_column: config.databases.id_column.clone(),
            databases_path_column: config.databases.path_column.clone(),
            databases_path_template: config.databases.path_template.clone(),
            migrations_dir: config.migrations.dir.clone(),
            migrations_table: config.migrations.table.clone(),
            migration_group_count: config.migration_groups.len(),
            database_migration_rule_count: config.database_migration_groups.len(),
            db_group_count: config.db_groups.len() + config.groups.len(),
            backup_dir: config.backup.dir.clone(),
            backup_before_migrate: config.backup.before_migrate,
            backup_keep_last: config.backup.keep_last,
            audit_path: config.audit.path.clone(),
            report_path: config.report.path.clone(),
            report_format: config.report.format.clone(),
            parallel: config.execution.parallel,
            lock_timeout_ms: config.execution.lock_timeout_ms,
            continue_on_error: config.execution.continue_on_error,
        }
    }
}

#[derive(Serialize)]
struct DiscoveryPreviewData {
    count: usize,
    databases: Vec<DiscoveryPreviewDatabase>,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct DiscoveryPreviewDatabase {
    id: String,
    path: PathBuf,
    exists: bool,
    readable: bool,
    allowed_root: Option<PathBuf>,
    error: Option<String>,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationGroupRequest {
    name: String,
    dir: Option<String>,
    versions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DbGroupRequest {
    name: String,
    selectors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseMigrationGroupRequest {
    selector: String,
    groups: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationFileRequest {
    version: String,
    name: String,
    filename: Option<String>,
    group: Option<String>,
    sql: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationFileUpdateRequest {
    path: String,
    version: String,
    group: String,
    sql: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DatabaseFileRequest {
    path: String,
    db_group: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaselineRequest {
    databases: Vec<String>,
}

#[derive(Serialize)]
struct AdminResult {
    message: String,
}

impl AdminResult {
    fn new(message: String) -> Self {
        Self { message }
    }
}

fn locked_config(state: &ServerState) -> Result<Config> {
    state
        .config
        .lock()
        .map(|config| config.clone())
        .map_err(|_| anyhow::anyhow!("GUI設定状態が壊れています"))
}

fn persist_config(state: &ServerState, config: Config) -> Result<()> {
    config.validate()?;
    let text = toml::to_string_pretty(&config).context("設定をTOMLへ変換できません")?;
    let tmp = unique_config_tmp_path(&state.config_path)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .with_context(|| format!("設定ファイルを書き込めません: {}", tmp.display()))?;
    if let Err(error) = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .with_context(|| format!("設定ファイルを書き込めません: {}", tmp.display()))
    {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    drop(file);
    std::fs::rename(&tmp, &state.config_path).with_context(|| {
        let _ = std::fs::remove_file(&tmp);
        format!(
            "設定ファイルを置き換えられません: {}",
            state.config_path.display()
        )
    })?;
    *state
        .config
        .lock()
        .map_err(|_| anyhow::anyhow!("GUI設定状態が壊れています"))? = config;
    Ok(())
}

fn unique_config_tmp_path(config_path: &Path) -> Result<PathBuf> {
    let parent = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("設定ファイルの親ディレクトリが必要です"))?;
    for _ in 0..16 {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).context("設定ファイルの一時名を生成できません")?;
        let suffix = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let filename = config_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sqlite-fleet.toml");
        let tmp = parent.join(format!(".{filename}.{suffix}.tmp"));
        if std::fs::symlink_metadata(&tmp).is_err() {
            return Ok(tmp);
        }
    }
    bail!("設定ファイルの一時名を生成できません");
}

fn clean_name(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} は空にできません");
    }
    if value.chars().any(char::is_whitespace) {
        bail!("{label} に空白は使用できません");
    }
    Ok(value.to_string())
}

fn clean_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn clean_path_list(values: Vec<String>) -> Result<Vec<String>> {
    let mut cleaned = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() {
            bail!("allowed_roots に空のパスは指定できません");
        }
        if !cleaned.contains(&value) {
            cleaned.push(value);
        }
    }
    if cleaned.is_empty() {
        cleaned.push(".".to_string());
    }
    Ok(cleaned)
}

fn clean_list(values: Vec<String>, label: &str) -> Result<Vec<String>> {
    if values.is_empty() {
        bail!("{label} は1件以上必要です");
    }
    clean_list_allow_empty(values, label)
}

fn clean_list_allow_empty(values: Vec<String>, label: &str) -> Result<Vec<String>> {
    let mut cleaned = Vec::new();
    for value in values {
        let value = clean_name(&value, label)?;
        if !cleaned.contains(&value) {
            cleaned.push(value);
        }
    }
    Ok(cleaned)
}

fn clean_version(value: &str) -> Result<String> {
    let value = clean_name(value, "version")?;
    if !value.chars().all(|ch| ch.is_ascii_digit()) {
        bail!("version はASCII数字だけ使用できます: {value}");
    }
    Ok(value)
}

fn clean_migration_id(value: &str) -> Result<String> {
    let value = clean_name(value, "マイグレーションファイル名")?;
    if value.contains('/') || value.contains('\\') {
        let path = PathBuf::from(&value);
        let Some(filename) = path.file_name().and_then(|filename| filename.to_str()) else {
            bail!("マイグレーションファイル名が不正です: {value}");
        };
        return clean_migration_id(filename);
    }
    sqlite_fleet::parse_migration_file_name(&value)?;
    Ok(value)
}

fn clean_file_stem(value: &str, label: &str) -> Result<String> {
    let value = clean_name(value, label)?;
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        bail!("{label} は英数字、_、- だけ使用できます: {value}");
    }
    Ok(value)
}

fn clean_relative_path(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} は空にできません");
    }
    let path = Path::new(value);
    if path.is_absolute() {
        bail!("{label} は相対パスで指定してください");
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("{label} に .. は使用できません");
    }
    Ok(value.to_string())
}

fn resolve_existing_or_creatable_dir(config: &Config, value: &str) -> Result<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        bail!("directory は空にできません");
    }
    let path = config.resolve_path(value);
    validate_path_stays_in_base(config, &path, "directory")?;
    Ok(path)
}

fn validate_path_stays_in_base(config: &Config, path: &Path, label: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{label} の親ディレクトリが必要です"))?;
    let canonical_parent = if parent.exists() {
        std::fs::canonicalize(parent).with_context(|| {
            format!(
                "{label} の親ディレクトリを解決できません: {}",
                parent.display()
            )
        })?
    } else {
        let mut existing = parent;
        while !existing.exists() {
            existing = existing
                .parent()
                .ok_or_else(|| anyhow::anyhow!("{label} の親ディレクトリを解決できません"))?;
        }
        std::fs::canonicalize(existing).with_context(|| {
            format!(
                "{label} の既存親ディレクトリを解決できません: {}",
                existing.display()
            )
        })?
    };
    if config.allowed_root_for_path(&canonical_parent)?.is_none() {
        bail!(
            "{label} はsecurity.allowed_rootsの外を指せません: {}",
            path.display()
        );
    }
    Ok(())
}
