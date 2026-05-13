use crate::{
    discovery::{validate_configured_db_path, validate_path_template_syntax},
    discovery_query::validate_discovery_query,
    path_utils::normalize_path_for_comparison,
    sqlite_ident::validate_identifier,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_CONFIG_PATH: &str = "sqlite-fleet.toml";
pub const DEFAULT_MIGRATIONS_TABLE: &str = "_sqlite_fleet_migrations";
pub const MAIN_MIGRATION_GROUP: &str = "main";
pub const ALL_DB_GROUP: &str = "all";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub databases: DatabasesConfig,
    #[serde(default)]
    pub migrations: MigrationsConfig,
    #[serde(default)]
    pub migration_groups: HashMap<String, MigrationGroupConfig>,
    #[serde(default)]
    pub database_migration_groups: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub execution: ExecutionConfig,
    #[serde(default)]
    pub report: ReportConfig,
    #[serde(default)]
    pub backup: BackupConfig,
    #[serde(default)]
    pub audit: AuditConfig,
    #[serde(default)]
    pub gui: GuiConfig,
    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub db_groups: HashMap<String, Vec<String>>,
    #[serde(skip)]
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DatabasesConfig {
    #[serde(default = "default_discovery")]
    pub discovery: String,
    pub path_glob: Option<String>,
    pub source: Option<String>,
    pub query: Option<String>,
    pub id_column: Option<String>,
    pub path_column: Option<String>,
    pub path_template: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationsConfig {
    #[serde(default = "default_migrations_dir")]
    pub dir: String,
    #[serde(default = "default_migrations_table")]
    pub table: String,
}

#[derive(Debug, Clone)]
pub struct MigrationGroupConfig {
    pub dir: Option<String>,
    pub migrations: Vec<String>,
}

impl MigrationGroupConfig {
    pub fn legacy_dir(dir: impl Into<String>) -> Self {
        Self {
            dir: Some(dir.into()),
            migrations: Vec::new(),
        }
    }

    pub fn versions(versions: Vec<String>) -> Self {
        Self {
            dir: None,
            migrations: versions,
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MigrationGroupConfigInput {
    Versions(Vec<String>),
    Table(MigrationGroupConfigTable),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationGroupConfigTable {
    dir: Option<String>,
    #[serde(default)]
    migrations: Vec<String>,
}

impl<'de> Deserialize<'de> for MigrationGroupConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match MigrationGroupConfigInput::deserialize(deserializer)? {
            MigrationGroupConfigInput::Versions(migrations) => {
                Ok(MigrationGroupConfig::versions(migrations))
            }
            MigrationGroupConfigInput::Table(MigrationGroupConfigTable { dir, migrations }) => {
                Ok(MigrationGroupConfig { dir, migrations })
            }
        }
    }
}

impl Serialize for MigrationGroupConfig {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.dir.is_none() {
            return self.migrations.serialize(serializer);
        }
        let mut state = serializer.serialize_struct("MigrationGroupConfig", 2)?;
        if let Some(dir) = &self.dir {
            state.serialize_field("dir", dir)?;
        }
        if !self.migrations.is_empty() {
            state.serialize_field("migrations", &self.migrations)?;
        }
        state.end()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConfig {
    #[serde(default = "default_parallel")]
    pub parallel: usize,
    #[serde(default = "default_lock_timeout_ms")]
    pub lock_timeout_ms: u64,
    #[serde(default)]
    pub continue_on_error: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReportConfig {
    #[serde(default = "default_report_format")]
    pub format: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    #[serde(default = "default_backup_dir")]
    pub dir: String,
    #[serde(default)]
    pub before_migrate: bool,
    #[serde(default = "default_backup_keep_last")]
    pub keep_last: usize,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditConfig {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuiConfig {
    #[serde(default = "default_true")]
    pub allow_check: bool,
    #[serde(default = "default_true")]
    pub allow_migrate: bool,
    #[serde(default = "default_true")]
    pub allow_backup: bool,
    #[serde(default = "default_true")]
    pub allow_restore: bool,
    #[serde(default = "default_true")]
    pub allow_sql_apply: bool,
    #[serde(default = "default_true")]
    pub allow_migration_edit: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Database {
    pub id: String,
    pub path: PathBuf,
    pub exists: bool,
    pub readable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Migration {
    pub group: String,
    pub filename: String,
    pub version: String,
    #[serde(skip_serializing)]
    pub version_number: u64,
    pub name: String,
    pub checksum: String,
    pub path: PathBuf,
    #[serde(skip_serializing)]
    pub sql: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppliedMigration {
    pub filename: String,
    pub version: String,
    pub name: String,
    pub checksum: String,
    pub applied_at: i64,
    pub execution_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabasePlan {
    pub database: Database,
    pub migration_groups: Vec<String>,
    pub applied_count: usize,
    pub pending: Vec<MigrationSummary>,
    pub checksum_errors: Vec<ChecksumError>,
    pub unknown_applied: Vec<MigrationSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationSummary {
    pub group: String,
    pub filename: String,
    pub version: String,
    pub name: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChecksumError {
    pub filename: String,
    pub version: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub database_count: usize,
    pub latest_migration: Option<MigrationSummary>,
    pub up_to_date: usize,
    pub pending: usize,
    pub failed: usize,
    pub missing: usize,
    pub corrupt: usize,
    pub plans: Vec<DatabasePlan>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrateReport {
    pub dry_run: bool,
    pub database_count: usize,
    pub processed_databases: usize,
    pub pending_databases: usize,
    pub applied_databases: usize,
    pub failed_databases: usize,
    pub databases: Vec<DatabaseMigrationResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseMigrationResult {
    pub database: Database,
    pub applied: Vec<MigrationSummary>,
    pub pending: Vec<MigrationSummary>,
    pub pre_backup: Option<DatabaseBackupResult>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    pub database_count: usize,
    pub ok: usize,
    pub failed: usize,
    pub databases: Vec<DatabaseCheckResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseCheckResult {
    pub database: Database,
    pub quick_check: Option<String>,
    pub integrity_check: Option<String>,
    pub wal_bytes: Option<u64>,
    pub shm_bytes: Option<u64>,
    pub checksum_errors: Vec<ChecksumError>,
    pub unknown_applied: Vec<MigrationSummary>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub config_ok: bool,
    pub discovery_ok: bool,
    pub migrations_ok: bool,
    pub database_count: usize,
    pub migration_count: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupReport {
    pub database_count: usize,
    pub backed_up: usize,
    pub failed: usize,
    pub backups: Vec<DatabaseBackupResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseBackupResult {
    pub database: Database,
    pub path: Option<PathBuf>,
    pub bytes: Option<u64>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub database: Database,
    pub restored_from: PathBuf,
    pub pre_restore_backup: Option<DatabaseBackupResult>,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaDriftReport {
    pub database_count: usize,
    pub baseline_database: Option<Database>,
    pub drifted: usize,
    pub failed: usize,
    pub databases: Vec<DatabaseSchemaDriftResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseSchemaDriftResult {
    pub database: Database,
    pub matches_baseline: bool,
    pub missing_objects: Vec<String>,
    pub extra_objects: Vec<String>,
    pub changed_objects: Vec<String>,
    pub success: bool,
    pub error: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            project: ProjectConfig::default(),
            databases: DatabasesConfig::default(),
            migrations: MigrationsConfig::default(),
            migration_groups: HashMap::new(),
            database_migration_groups: HashMap::new(),
            execution: ExecutionConfig::default(),
            report: ReportConfig::default(),
            backup: BackupConfig::default(),
            audit: AuditConfig::default(),
            gui: GuiConfig::default(),
            groups: HashMap::new(),
            db_groups: HashMap::new(),
            base_dir: PathBuf::from("."),
        }
    }
}

impl Default for DatabasesConfig {
    fn default() -> Self {
        Self {
            discovery: default_discovery(),
            path_glob: Some("./data/**/*.db".to_string()),
            source: None,
            query: None,
            id_column: Some("id".to_string()),
            path_column: None,
            path_template: None,
        }
    }
}

impl Default for MigrationsConfig {
    fn default() -> Self {
        Self {
            dir: default_migrations_dir(),
            table: default_migrations_table(),
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            parallel: default_parallel(),
            lock_timeout_ms: default_lock_timeout_ms(),
            continue_on_error: false,
        }
    }
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            format: default_report_format(),
            path: None,
        }
    }
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            dir: default_backup_dir(),
            before_migrate: false,
            keep_last: default_backup_keep_last(),
        }
    }
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            allow_check: true,
            allow_migrate: true,
            allow_backup: true,
            allow_restore: true,
            allow_sql_apply: true,
            allow_migration_edit: true,
        }
    }
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let config = Self::load_unvalidated(path)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_for_discovery(path: impl AsRef<Path>) -> Result<Self> {
        let config = Self::load_unvalidated(path)?;
        config.validate_discovery()?;
        Ok(config)
    }

    pub fn load_for_operation(path: impl AsRef<Path>) -> Result<Self> {
        let config = Self::load_unvalidated(path)?;
        config.validate_operation()?;
        Ok(config)
    }

    pub(crate) fn load_unvalidated(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .with_context(|| format!("設定ファイルを読み込めません: {}", path.display()))?;
        let mut config: Config = toml::from_str(&text).map_err(|error| {
            anyhow!(
                "設定ファイルのTOML解析に失敗しました: {}: {error}",
                path.display()
            )
        })?;
        let base_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        config.base_dir = fs::canonicalize(&base_dir).with_context(|| {
            format!(
                "設定ファイルの親ディレクトリを解決できません: {}",
                base_dir.display()
            )
        })?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_operation()?;
        if self.report.format.trim() != self.report.format {
            bail!("report.format の前後に空白は使用できません");
        }
        if self.report.format != "json" {
            bail!("現在対応している report.format は json のみです");
        }
        if self
            .report
            .path
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            bail!("report.path は空にできません");
        }
        if let Some(path) = self.report.path.as_deref() {
            validate_configured_db_path(self, "report.path", path)?;
        }
        self.validate_extended_runtime()?;
        Ok(())
    }

    pub fn validate_operation(&self) -> Result<()> {
        self.validate_discovery()?;

        if self.migrations.dir.trim().is_empty() {
            bail!("migrations.dir は空にできません");
        }
        validate_configured_db_path(self, "migrations.dir", &self.migrations.dir)?;
        validate_identifier(&self.migrations.table)?;
        for (group, config) in &self.migration_groups {
            validate_group_name("migration_groups", group)?;
            if let Some(dir) = config.dir.as_deref() {
                if dir.trim().is_empty() {
                    bail!("migration_groups.{group}.dir は空にできません");
                }
                validate_configured_db_path(self, &format!("migration_groups.{group}.dir"), dir)?;
            }
            let mut migrations = std::collections::HashSet::new();
            for migration in &config.migrations {
                if migration.trim().is_empty() || migration.trim() != migration {
                    bail!(
                        "migration_groups.{group} のmigrationは空白なしの非空文字列である必要があります"
                    );
                }
                if migration.contains('/') || migration.contains('\\') {
                    bail!(
                        "migration_groups.{group} のmigrationにパス区切りは使用できません: {migration}"
                    );
                }
                if !migrations.insert(migration) {
                    bail!("migration_groups.{group} のmigrationが重複しています: {migration}");
                }
            }
        }
        self.validate_database_migration_groups()?;
        if self.execution.parallel == 0 {
            bail!("execution.parallel は1以上が必要です");
        }
        self.validate_extended_runtime()?;
        Ok(())
    }

    fn validate_extended_runtime(&self) -> Result<()> {
        if self.backup.dir.trim().is_empty() {
            bail!("backup.dir は空にできません");
        }
        validate_configured_db_path(self, "backup.dir", &self.backup.dir)?;
        if self.backup.keep_last > 10_000 {
            bail!("backup.keep_last が大きすぎます");
        }
        if let Some(path) = self.audit.path.as_deref() {
            if path.trim().is_empty() {
                bail!("audit.path は空にできません");
            }
            validate_configured_db_path(self, "audit.path", path)?;
        }
        for (group, selectors) in self.effective_db_groups() {
            validate_group_name("db_groups", &group)?;
            if selectors.is_empty() {
                bail!("db_groups.{group} は1件以上のDB selectorが必要です");
            }
            for selector in selectors {
                if selector.trim().is_empty() || selector.trim() != selector {
                    bail!(
                        "db_groups.{group} のDB selectorは空白なしの非空文字列である必要があります"
                    );
                }
            }
        }
        Ok(())
    }

    pub fn validate_discovery(&self) -> Result<()> {
        if self.databases.discovery.trim().is_empty() {
            bail!("databases.discovery は空にできません");
        }
        if self.databases.discovery.trim() != self.databases.discovery {
            bail!("databases.discovery の前後に空白は使用できません");
        }
        match self.databases.discovery.as_str() {
            "glob" => {
                if is_blank(self.databases.path_glob.as_deref()) {
                    bail!("glob discovery には databases.path_glob が必要です");
                }
                if let Some(path_glob) = self.databases.path_glob.as_deref() {
                    validate_configured_db_path(self, "databases.path_glob", path_glob)?;
                }
            }
            "query" => {
                if is_blank(self.databases.source.as_deref()) {
                    bail!("query discovery には databases.source が必要です");
                }
                if let Some(source) = self.databases.source.as_deref() {
                    validate_configured_db_path(self, "databases.source", source)?;
                }
                if is_blank(self.databases.query.as_deref()) {
                    bail!("query discovery には databases.query が必要です");
                }
                if let Some(query) = self.databases.query.as_deref() {
                    validate_discovery_query(query)?;
                }
                if is_blank(self.databases.path_column.as_deref())
                    && is_blank(self.databases.path_template.as_deref())
                {
                    bail!("query discovery には databases.path_column または databases.path_template が必要です");
                }
                if self
                    .databases
                    .id_column
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    bail!("databases.id_column は空にできません");
                }
                if self
                    .databases
                    .id_column
                    .as_deref()
                    .is_some_and(|value| value.trim() != value)
                {
                    bail!("databases.id_column の前後に空白は使用できません");
                }
                if self
                    .databases
                    .path_column
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    bail!("databases.path_column は空にできません");
                }
                if self
                    .databases
                    .path_column
                    .as_deref()
                    .is_some_and(|value| value.trim() != value)
                {
                    bail!("databases.path_column の前後に空白は使用できません");
                }
                if self
                    .databases
                    .path_template
                    .as_deref()
                    .is_some_and(|value| value.trim().is_empty())
                {
                    bail!("databases.path_template は空にできません");
                }
                if let Some(path_template) = self.databases.path_template.as_deref() {
                    validate_configured_db_path(self, "databases.path_template", path_template)?;
                    validate_path_template_syntax(path_template)?;
                }
            }
            other => bail!("未対応の discovery です: {other}"),
        }
        Ok(())
    }

    pub fn validate_database_migration_groups(&self) -> Result<()> {
        let migration_groups = self.effective_migration_groups();
        for (selector, groups) in &self.database_migration_groups {
            if selector.trim().is_empty() || selector.trim() != selector {
                bail!("database_migration_groups のDB selectorは空白なしの非空文字列である必要があります");
            }
            for group in groups {
                validate_group_name(&format!("database_migration_groups.{selector}"), group)?;
                if !migration_groups.contains_key(group) {
                    bail!("database_migration_groups.{selector} のマイグレーショングループが見つかりません: {group}");
                }
            }
        }
        Ok(())
    }

    pub fn resolve_path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        }
    }

    pub fn migrations_table(&self) -> &str {
        &self.migrations.table
    }

    pub fn effective_migration_groups(&self) -> HashMap<String, MigrationGroupConfig> {
        if self.migration_groups.is_empty() {
            return HashMap::from([(
                MAIN_MIGRATION_GROUP.to_string(),
                MigrationGroupConfig {
                    dir: Some(self.migrations.dir.clone()),
                    migrations: Vec::new(),
                },
            )]);
        }
        let mut groups = self.migration_groups.clone();
        if groups.values().all(is_empty_version_migration_group)
            && !groups.contains_key(MAIN_MIGRATION_GROUP)
        {
            groups.insert(
                MAIN_MIGRATION_GROUP.to_string(),
                MigrationGroupConfig {
                    dir: Some(self.migrations.dir.clone()),
                    migrations: Vec::new(),
                },
            );
        }
        groups
    }

    pub fn effective_db_groups(&self) -> HashMap<String, Vec<String>> {
        let mut groups = self.groups.clone();
        for (name, selectors) in &self.db_groups {
            groups.insert(name.clone(), selectors.clone());
        }
        groups
    }

    pub fn migration_groups_for_database(&self, database: &Database) -> Vec<String> {
        let configured = self.effective_migration_groups();
        let mut matched_rule = false;
        let mut groups = Vec::new();
        for (selector, rule_groups) in &self.database_migration_groups {
            if database_matches_config_selector(self, database, selector) {
                matched_rule = true;
                groups.extend(rule_groups.iter().cloned());
            }
        }
        if !matched_rule {
            groups = {
                if configured.contains_key(MAIN_MIGRATION_GROUP) {
                    vec![MAIN_MIGRATION_GROUP.to_string()]
                } else {
                    let mut names = configured
                        .iter()
                        .filter(|(_name, group)| !is_empty_version_migration_group(group))
                        .map(|(name, _group)| name.clone())
                        .collect::<Vec<_>>();
                    names.sort();
                    names
                }
            };
        }
        groups.sort();
        groups.dedup();
        groups
    }

    pub fn database_matches_selector(&self, database: &Database, selector: &str) -> bool {
        database_matches_config_selector(self, database, selector)
    }

    pub(crate) fn validate_path_within_base(&self, label: &str, path: &str) -> Result<()> {
        self.validate_resolved_path_within_base(label, &self.resolve_path(path))
    }

    pub(crate) fn validate_resolved_path_within_base(
        &self,
        label: &str,
        path: &Path,
    ) -> Result<()> {
        let base_dir = normalize_path_for_comparison(&self.base_dir);
        let resolved = normalize_path_for_comparison(path);
        if !resolved.starts_with(&base_dir) {
            if label == "DBパス" {
                bail!(
                    "DBパスが設定ディレクトリ外を指しています: {}",
                    path.display()
                );
            }
            bail!(
                "{label} は設定ファイルのディレクトリ外を指せません: {}",
                path.display()
            );
        }
        Ok(())
    }
}

fn is_empty_version_migration_group(group: &MigrationGroupConfig) -> bool {
    group.dir.is_none() && group.migrations.is_empty()
}

include!("config/template.rs");
