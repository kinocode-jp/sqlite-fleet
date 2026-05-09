use crate::{
    discovery::{validate_configured_db_path, validate_path_template_syntax},
    discovery_query::validate_discovery_query,
    path_utils::normalize_path_for_comparison,
    sqlite_ident::validate_identifier,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DEFAULT_CONFIG_PATH: &str = "sqlite-fleet.toml";
pub const DEFAULT_MIGRATIONS_TABLE: &str = "_sqlite_fleet_migrations";

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
    pub execution: ExecutionConfig,
    #[serde(default)]
    pub report: ReportConfig,
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

#[derive(Debug, Clone, Serialize)]
pub struct Database {
    pub id: String,
    pub path: PathBuf,
    pub exists: bool,
    pub readable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Migration {
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
    pub version: String,
    pub name: String,
    pub checksum: String,
    pub applied_at: i64,
    pub execution_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabasePlan {
    pub database: Database,
    pub applied_count: usize,
    pub pending: Vec<MigrationSummary>,
    pub checksum_errors: Vec<ChecksumError>,
    pub unknown_applied: Vec<MigrationSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrationSummary {
    pub version: String,
    pub name: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChecksumError {
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

impl Default for Config {
    fn default() -> Self {
        Self {
            project: ProjectConfig::default(),
            databases: DatabasesConfig::default(),
            migrations: MigrationsConfig::default(),
            execution: ExecutionConfig::default(),
            report: ReportConfig::default(),
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
        Ok(())
    }

    pub fn validate_operation(&self) -> Result<()> {
        self.validate_discovery()?;

        if self.migrations.dir.trim().is_empty() {
            bail!("migrations.dir は空にできません");
        }
        validate_configured_db_path(self, "migrations.dir", &self.migrations.dir)?;
        validate_identifier(&self.migrations.table)?;
        if self.execution.parallel == 0 {
            bail!("execution.parallel は1以上が必要です");
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

pub fn init_project(config_path: impl AsRef<Path>) -> Result<()> {
    let config_path = config_path.as_ref();
    let base_dir = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(base_dir).with_context(|| {
        format!(
            "設定ファイルの親ディレクトリを作成できません: {}",
            base_dir.display()
        )
    })?;

    if !config_path.exists() {
        fs::write(config_path, default_config_template())
            .with_context(|| format!("設定ファイルを作成できません: {}", config_path.display()))?;
    }

    fs::create_dir_all(base_dir.join("migrations"))
        .context("migrations ディレクトリを作成できません")?;
    Ok(())
}

pub fn default_config_template() -> &'static str {
    r#"[project]
name = "sqlite-fleet-project"

[databases]
discovery = "glob"
path_glob = "./data/**/*.db"

[migrations]
dir = "./migrations"
table = "_sqlite_fleet_migrations"

[execution]
parallel = 4
lock_timeout_ms = 5000
continue_on_error = false

[report]
format = "json"
path = "./sqlite-fleet-report.json"
"#
}

fn is_blank(value: Option<&str>) -> bool {
    value.is_none_or(|value| value.trim().is_empty())
}

fn default_discovery() -> String {
    "glob".to_string()
}

fn default_migrations_dir() -> String {
    "./migrations".to_string()
}

fn default_migrations_table() -> String {
    DEFAULT_MIGRATIONS_TABLE.to_string()
}

fn default_parallel() -> usize {
    4
}

fn default_lock_timeout_ms() -> u64 {
    5000
}

fn default_report_format() -> String {
    "json".to_string()
}
