use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

mod config;
mod diagnostics;
mod discovery;
mod discovery_query;
mod history;
mod migration;
mod migration_sql;
mod migration_sql_guard;
mod path_utils;
mod sql_scan;
mod sqlite_ident;
pub use config::*;
pub use diagnostics::{
    doctor, doctor_and_write_report, doctor_config, doctor_with_overrides, write_report_json,
};
pub use discovery::{
    discover_by_glob, discover_by_query, discover_databases, render_path_template,
};
use discovery::{validate_database_id, validate_database_set};
use history::create_migrations_table_sql;
pub use history::{ensure_migrations_table, read_applied_migrations};
use migration::validate_migrations;
pub use migration::{checksum_sql, load_migrations, parse_migration_file};
use path_utils::normalize_path_for_comparison;
use sqlite_ident::validate_identifier;

pub fn build_plan(
    config: &Config,
    databases: &[Database],
    migrations: &[Migration],
) -> Vec<DatabasePlan> {
    if let Err(error) = validate_database_set(databases) {
        let error = error.to_string();
        return databases
            .iter()
            .map(|database| DatabasePlan {
                database: database.clone(),
                applied_count: 0,
                pending: migrations.iter().map(MigrationSummary::from).collect(),
                checksum_errors: Vec::new(),
                unknown_applied: Vec::new(),
                error: Some(error.clone()),
            })
            .collect();
    }
    databases
        .iter()
        .map(|database| build_database_plan(config, database, migrations))
        .collect()
}

pub fn build_database_plan(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
) -> DatabasePlan {
    if let Err(error) = validate_runtime_config(config) {
        return DatabasePlan {
            database: database.clone(),
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    if let Err(error) = validate_database_id(&database.id) {
        return DatabasePlan {
            database: database.clone(),
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    if let Err(error) = validate_migrations(config, migrations) {
        return DatabasePlan {
            database: database.clone(),
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    if let Err(error) = config.validate_resolved_path_within_base("DBパス", &database.path) {
        return DatabasePlan {
            database: database.clone(),
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    let database = refresh_database_state(database);
    if let Err(error) = ensure_existing_database_file(&database.path) {
        return DatabasePlan {
            database,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }

    match open_existing_readonly(&database.path)
        .and_then(|conn| {
            conn.busy_timeout(std::time::Duration::from_millis(
                config.execution.lock_timeout_ms,
            ))?;
            Ok(conn)
        })
        .and_then(|conn| read_applied_migrations(&conn, config.migrations_table()))
    {
        Ok(applied) => {
            let applied_by_version: HashMap<&str, &AppliedMigration> =
                applied.iter().map(|m| (m.version.as_str(), m)).collect();
            let applied_versions: HashSet<&str> = applied_by_version.keys().copied().collect();
            let known_versions: HashSet<&str> =
                migrations.iter().map(|m| m.version.as_str()).collect();
            let pending = migrations
                .iter()
                .filter(|migration| !applied_versions.contains(migration.version.as_str()))
                .map(MigrationSummary::from)
                .collect();
            let checksum_errors = migrations
                .iter()
                .filter_map(|migration| {
                    applied_by_version
                        .get(migration.version.as_str())
                        .and_then(|applied| {
                            (applied.checksum != migration.checksum).then(|| ChecksumError {
                                version: migration.version.clone(),
                                expected: migration.checksum.clone(),
                                actual: applied.checksum.clone(),
                            })
                        })
                })
                .collect();
            let unknown_applied = applied
                .iter()
                .filter(|migration| !known_versions.contains(migration.version.as_str()))
                .map(MigrationSummary::from)
                .collect();
            DatabasePlan {
                database,
                applied_count: applied.len(),
                pending,
                checksum_errors,
                unknown_applied,
                error: None,
            }
        }
        Err(error) => DatabasePlan {
            database,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        },
    }
}

pub fn status_report(config: &Config) -> Result<StatusReport> {
    validate_runtime_config(config)?;
    let databases = discover_databases(config)?;
    ensure_databases_found(&databases)?;
    let migrations = load_migrations(config)?;
    let plans = build_plan(config, &databases, &migrations);
    let latest_migration = migrations.last().map(MigrationSummary::from);
    let up_to_date = plans
        .iter()
        .filter(|plan| {
            plan.error.is_none()
                && plan.pending.is_empty()
                && plan.checksum_errors.is_empty()
                && plan.unknown_applied.is_empty()
        })
        .count();
    let pending = plans
        .iter()
        .filter(|plan| plan.error.is_none() && !plan.pending.is_empty())
        .count();
    let failed = plans.iter().filter(|plan| plan.error.is_some()).count();
    let missing = plans.iter().filter(|plan| !plan.database.exists).count();
    let corrupt = plans
        .iter()
        .filter(|plan| !plan.checksum_errors.is_empty() || !plan.unknown_applied.is_empty())
        .count();
    Ok(StatusReport {
        database_count: databases.len(),
        latest_migration,
        up_to_date,
        pending,
        failed,
        missing,
        corrupt,
        plans,
    })
}

pub fn migrate(
    config: &Config,
    dry_run: bool,
    only_database: Option<&str>,
) -> Result<MigrateReport> {
    validate_runtime_config(config)?;
    let mut databases = discover_databases(config)?;
    ensure_databases_found(&databases)?;
    if let Some(id) = only_database {
        databases.retain(|database| database_matches_selector(config, database, id));
        if databases.is_empty() {
            bail!("指定されたDBが見つかりません: {id}");
        }
    }
    let migrations = load_migrations(config)?;

    let results = if config.execution.continue_on_error {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(config.execution.parallel.max(1))
            .build()
            .context("並列実行プールを作成できません")?;
        pool.install(|| {
            databases
                .par_iter()
                .map(|database| migrate_database(config, database, &migrations, dry_run))
                .collect::<Vec<_>>()
        })
    } else {
        let mut results = Vec::new();
        for database in &databases {
            let result = migrate_database(config, database, &migrations, dry_run);
            let success = result.success;
            results.push(result);
            if !success {
                break;
            }
        }
        results
    };

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
        dry_run,
        database_count: databases.len(),
        processed_databases: results.len(),
        pending_databases,
        applied_databases,
        failed_databases,
        databases: results,
    })
}

pub fn migrate_database(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
    dry_run: bool,
) -> DatabaseMigrationResult {
    match migrate_database_inner(config, database, migrations, dry_run) {
        Ok(result) => result,
        Err(error) => DatabaseMigrationResult {
            database: refresh_database_state_if_path_allowed(config, database),
            applied: Vec::new(),
            pending: Vec::new(),
            success: false,
            error: Some(error.to_string()),
        },
    }
}

fn migrate_database_inner(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
    dry_run: bool,
) -> Result<DatabaseMigrationResult> {
    validate_runtime_config(config)?;
    validate_database_id(&database.id)?;
    validate_migrations(config, migrations)?;
    config.validate_resolved_path_within_base("DBパス", &database.path)?;
    let database = refresh_database_state(database);
    ensure_existing_database_file(&database.path)?;
    let mut conn = if dry_run {
        open_existing_readonly(&database.path)
    } else {
        open_existing_readwrite(&database.path)
    }
    .with_context(|| format!("DBを開けません: {}", database.path.display()))?;
    conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let applied = read_applied_migrations(&conn, config.migrations_table())?;
    let applied_by_version: HashMap<&str, &AppliedMigration> =
        applied.iter().map(|m| (m.version.as_str(), m)).collect();
    let known_versions: HashSet<&str> = migrations.iter().map(|m| m.version.as_str()).collect();
    let unknown_versions = applied
        .iter()
        .filter(|migration| !known_versions.contains(migration.version.as_str()))
        .map(|migration| migration.version.as_str())
        .collect::<Vec<_>>();
    if !unknown_versions.is_empty() {
        bail!(
            "ローカルに存在しない適用済みmigrationがあります: {}",
            unknown_versions.join(", ")
        );
    }

    let mut checksum_errors = Vec::new();
    for migration in migrations {
        if let Some(applied) = applied_by_version.get(migration.version.as_str()) {
            if applied.checksum != migration.checksum {
                checksum_errors.push(format!(
                    "{} のchecksumが一致しません expected={} actual={}",
                    migration.version, migration.checksum, applied.checksum
                ));
            }
        }
    }
    if !checksum_errors.is_empty() {
        bail!("{}", checksum_errors.join(", "));
    }

    let pending_migrations: Vec<&Migration> = migrations
        .iter()
        .filter(|migration| !applied_by_version.contains_key(migration.version.as_str()))
        .collect();
    let pending = pending_migrations
        .iter()
        .map(|migration| MigrationSummary::from(*migration))
        .collect::<Vec<_>>();

    if dry_run {
        return Ok(DatabaseMigrationResult {
            database: database.clone(),
            applied: Vec::new(),
            pending,
            success: true,
            error: None,
        });
    }

    if pending_migrations.is_empty() {
        return Ok(DatabaseMigrationResult {
            database: database.clone(),
            applied: Vec::new(),
            pending,
            success: true,
            error: None,
        });
    }

    validate_identifier(config.migrations_table())?;
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(error) => {
            return Ok(failed_migration_result(
                &database,
                &pending,
                format!("migration transaction を開始できません: {error}"),
            ));
        }
    };
    if let Err(error) = tx.execute(&create_migrations_table_sql(config.migrations_table()), []) {
        return Ok(failed_migration_result(
            &database,
            &pending,
            format!(
                "migration 管理テーブルを作成できません: {}: {error}",
                config.migrations_table()
            ),
        ));
    }
    let mut applied_now = Vec::new();
    for migration in pending_migrations {
        let start = Instant::now();
        if let Err(error) = tx.execute_batch(&migration.sql) {
            return Ok(failed_migration_result(
                &database,
                &pending,
                format!(
                    "migration 適用に失敗しました: {}: {error}",
                    migration.path.display()
                ),
            ));
        }
        let execution_ms = start.elapsed().as_millis().min(i64::MAX as u128) as i64;
        if let Err(error) = tx.execute(
            &format!(
                "INSERT INTO main.{} (version, name, checksum, applied_at, execution_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                config.migrations_table()
            ),
            params![
                migration.version,
                migration.name,
                migration.checksum,
                unix_timestamp(),
                execution_ms
            ],
        ) {
            return Ok(failed_migration_result(
                &database,
                &pending,
                format!(
                    "migration 履歴を保存できません: {}: {error}",
                    migration.version
                ),
            ));
        }
        applied_now.push(MigrationSummary::from(migration));
    }
    if let Err(error) = tx.commit() {
        return Ok(failed_migration_result(
            &database,
            &pending,
            format!("migration transaction をコミットできません: {error}"),
        ));
    }

    Ok(DatabaseMigrationResult {
        database: database.clone(),
        applied: applied_now,
        pending: Vec::new(),
        success: true,
        error: None,
    })
}

fn failed_migration_result(
    database: &Database,
    pending: &[MigrationSummary],
    error: String,
) -> DatabaseMigrationResult {
    DatabaseMigrationResult {
        database: database.clone(),
        applied: Vec::new(),
        pending: pending.to_vec(),
        success: false,
        error: Some(error),
    }
}

pub fn check(config: &Config) -> Result<CheckReport> {
    validate_runtime_config(config)?;
    let databases = discover_databases(config)?;
    ensure_databases_found(&databases)?;
    let migrations = load_migrations(config)?;
    let results = databases
        .iter()
        .map(|database| check_database(config, database, &migrations))
        .collect::<Vec<_>>();
    let ok = results.iter().filter(|result| result.success).count();
    let failed = results.len() - ok;
    Ok(CheckReport {
        database_count: results.len(),
        ok,
        failed,
        databases: results,
    })
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
    validate_runtime_config(config)?;
    validate_database_id(&database.id)?;
    validate_migrations(config, migrations)?;
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
        .filter(|migration| !known.contains_key(migration.version.as_str()))
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

impl From<&Migration> for MigrationSummary {
    fn from(migration: &Migration) -> Self {
        Self {
            version: migration.version.clone(),
            name: migration.name.clone(),
            checksum: migration.checksum.clone(),
        }
    }
}

impl From<&AppliedMigration> for MigrationSummary {
    fn from(migration: &AppliedMigration) -> Self {
        Self {
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
    validate_identifier(config.migrations_table())?;
    Ok(())
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

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
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
