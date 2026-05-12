use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use rusqlite::{backup::StepResult, params, Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct DatabaseSelection {
    pub database: Option<String>,
    pub group: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct MigrateOptions {
    pub dry_run: bool,
    pub selection: DatabaseSelection,
    pub backup_before_migrate: Option<bool>,
}

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
pub use migration::{checksum_sql, load_migrations, parse_migration_file, validate_migration};
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
                migration_groups: config.migration_groups_for_database(database),
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

fn migrations_for_database(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
) -> Vec<Migration> {
    let migration_groups = config.migration_groups_for_database(database);
    let groups = migration_groups
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if groups.is_empty() {
        return migrations.to_vec();
    }
    let mut selected = migrations
        .iter()
        .filter(|migration| groups.contains(migration.group.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by(|a, b| {
        a.version_number
            .cmp(&b.version_number)
            .then_with(|| a.version.cmp(&b.version))
            .then_with(|| a.group.cmp(&b.group))
    });
    selected.dedup_by(|left, right| left.version == right.version);
    selected
}

pub fn build_database_plan(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
) -> DatabasePlan {
    let migrations = migrations_for_database(config, database, migrations);
    let known_versions = migrations
        .iter()
        .map(|migration| migration.version.as_str())
        .collect::<HashSet<_>>();
    let migration_groups = config.migration_groups_for_database(database);
    if let Err(error) = validate_runtime_config(config) {
        return DatabasePlan {
            database: database.clone(),
            migration_groups,
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
            migration_groups,
            applied_count: 0,
            pending: migrations.iter().map(MigrationSummary::from).collect(),
            checksum_errors: Vec::new(),
            unknown_applied: Vec::new(),
            error: Some(error.to_string()),
        };
    }
    if let Err(error) = validate_migrations(config, &migrations) {
        return DatabasePlan {
            database: database.clone(),
            migration_groups,
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
            migration_groups,
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
            migration_groups,
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
                migration_groups,
                applied_count: applied.len(),
                pending,
                checksum_errors,
                unknown_applied,
                error: None,
            }
        }
        Err(error) => DatabasePlan {
            database,
            migration_groups,
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
    migrate_with_options(
        config,
        MigrateOptions {
            dry_run,
            selection: DatabaseSelection {
                database: only_database.map(str::to_string),
                ..DatabaseSelection::default()
            },
            backup_before_migrate: None,
        },
    )
}

pub fn migrate_with_options(config: &Config, options: MigrateOptions) -> Result<MigrateReport> {
    validate_runtime_config(config)?;
    let databases = select_databases(config, &options.selection)?;
    ensure_databases_found(&databases)?;
    let migrations = load_migrations(config)?;
    let backup_before_migrate = options
        .backup_before_migrate
        .unwrap_or(config.backup.before_migrate)
        && !options.dry_run;

    let results = if config.execution.continue_on_error {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(config.execution.parallel.max(1))
            .build()
            .context("並列実行プールを作成できません")?;
        pool.install(|| {
            databases
                .par_iter()
                .map(|database| {
                    migrate_database_with_pre_backup(
                        config,
                        database,
                        &migrations,
                        options.dry_run,
                        backup_before_migrate,
                    )
                })
                .collect::<Vec<_>>()
        })
    } else {
        let mut results = Vec::new();
        for database in &databases {
            let result = migrate_database_with_pre_backup(
                config,
                database,
                &migrations,
                options.dry_run,
                backup_before_migrate,
            );
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
        dry_run: options.dry_run,
        database_count: databases.len(),
        processed_databases: results.len(),
        pending_databases,
        applied_databases,
        failed_databases,
        databases: results,
    })
}

fn migrate_database_with_pre_backup(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
    dry_run: bool,
    backup_before_migrate: bool,
) -> DatabaseMigrationResult {
    let _operation_lock = if dry_run {
        None
    } else {
        match acquire_database_operation_lock(config, &database.path, "migrate") {
            Ok(lock) => Some(lock),
            Err(error) => {
                return DatabaseMigrationResult {
                    database: refresh_database_state_if_path_allowed(config, database),
                    applied: Vec::new(),
                    pending: Vec::new(),
                    pre_backup: None,
                    success: false,
                    error: Some(error.to_string()),
                };
            }
        }
    };
    let pre_backup = if backup_before_migrate {
        let backup = backup_database_inner_result(config, database, &[]);
        if !backup.success {
            return DatabaseMigrationResult {
                database: refresh_database_state_if_path_allowed(config, database),
                applied: Vec::new(),
                pending: Vec::new(),
                pre_backup: Some(backup.clone()),
                success: false,
                error: backup.error.clone(),
            };
        }
        Some(backup)
    } else {
        None
    };
    let mut result = migrate_database_unlocked(config, database, migrations, dry_run);
    result.pre_backup = pre_backup;
    result
}

pub fn migrate_database(
    config: &Config,
    database: &Database,
    migrations: &[Migration],
    dry_run: bool,
) -> DatabaseMigrationResult {
    let _operation_lock = if dry_run {
        None
    } else {
        match acquire_database_operation_lock(config, &database.path, "migrate") {
            Ok(lock) => Some(lock),
            Err(error) => {
                return DatabaseMigrationResult {
                    database: refresh_database_state_if_path_allowed(config, database),
                    applied: Vec::new(),
                    pending: Vec::new(),
                    pre_backup: None,
                    success: false,
                    error: Some(error.to_string()),
                };
            }
        }
    };
    migrate_database_unlocked(config, database, migrations, dry_run)
}

fn migrate_database_unlocked(
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
            pre_backup: None,
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
    let unknown_versions = applied
        .iter()
        .filter(|migration| !known_versions.contains(migration.version.as_str()))
        .map(|migration| migration.version.as_str())
        .collect::<Vec<_>>();
    if !unknown_versions.is_empty() {
        bail!(
            "対象外またはローカルに存在しない適用済みmigrationがあります: {}",
            unknown_versions.join(", ")
        );
    }

    let mut checksum_errors = Vec::new();
    for migration in &migrations {
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
            pre_backup: None,
            success: true,
            error: None,
        });
    }

    if pending_migrations.is_empty() {
        return Ok(DatabaseMigrationResult {
            database: database.clone(),
            applied: Vec::new(),
            pending,
            pre_backup: None,
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
        pre_backup: None,
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
        pre_backup: None,
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

pub fn backup(config: &Config, selection: DatabaseSelection) -> Result<BackupReport> {
    validate_runtime_config(config)?;
    let databases = select_databases(config, &selection)?;
    ensure_databases_found(&databases)?;
    let backups = databases
        .iter()
        .map(|database| backup_database(config, database))
        .collect::<Vec<_>>();
    let backed_up = backups.iter().filter(|backup| backup.success).count();
    let failed = backups.len() - backed_up;
    Ok(BackupReport {
        database_count: backups.len(),
        backed_up,
        failed,
        backups,
    })
}

pub fn backup_database(config: &Config, database: &Database) -> DatabaseBackupResult {
    let _operation_lock = match acquire_database_operation_lock(config, &database.path, "backup") {
        Ok(lock) => lock,
        Err(error) => {
            return DatabaseBackupResult {
                database: refresh_database_state_if_path_allowed(config, database),
                path: None,
                bytes: None,
                success: false,
                error: Some(error.to_string()),
            };
        }
    };
    backup_database_inner_result(config, database, &[])
}

fn backup_database_inner_result(
    config: &Config,
    database: &Database,
    protected_paths: &[PathBuf],
) -> DatabaseBackupResult {
    match backup_database_inner(config, database, protected_paths) {
        Ok(result) => result,
        Err(error) => DatabaseBackupResult {
            database: refresh_database_state_if_path_allowed(config, database),
            path: None,
            bytes: None,
            success: false,
            error: Some(error.to_string()),
        },
    }
}

fn backup_database_inner(
    config: &Config,
    database: &Database,
    protected_paths: &[PathBuf],
) -> Result<DatabaseBackupResult> {
    validate_runtime_config(config)?;
    validate_database_id(&database.id)?;
    config.validate_resolved_path_within_base("DBパス", &database.path)?;
    let database = refresh_database_state(database);
    ensure_existing_database_file(&database.path)?;
    let backup_dir = backup_directory(config)?;
    let database_component = backup_path_component(&database.id);
    let database_dir = backup_dir.join(&database_component);
    fs::create_dir_all(&database_dir).with_context(|| {
        format!(
            "backup ディレクトリを作成できません: {}",
            database_dir.display()
        )
    })?;
    config.validate_resolved_path_within_base("backup.dir", &database_dir)?;
    let destination = database_dir.join(format!(
        "{}_{}.db",
        unix_timestamp_nanos(),
        database_component
    ));
    backup_database_file_with_sqlite_backup(config, &database.path, &destination)
        .with_context(|| format!("backup を作成できません: {}", destination.display()))?;
    let bytes = fs::metadata(&destination)
        .ok()
        .map(|metadata| metadata.len());
    let mut prune_protected_paths = protected_paths.to_vec();
    prune_protected_paths.push(destination.clone());
    prune_old_backups(config, &database_dir, &prune_protected_paths)?;
    Ok(DatabaseBackupResult {
        database,
        path: Some(destination),
        bytes,
        success: true,
        error: None,
    })
}

fn backup_database_file_with_sqlite_backup(
    config: &Config,
    source: &Path,
    destination: &Path,
) -> Result<()> {
    if destination.exists() {
        bail!(
            "backup 先ファイルが既に存在します: {}",
            destination.display()
        );
    }
    let source_conn = open_existing_readonly(source)
        .with_context(|| format!("DBを開けません: {}", source.display()))?;
    source_conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let mut destination_conn = Connection::open_with_flags(
        destination,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .with_context(|| format!("backup 先DBを作成できません: {}", destination.display()))?;
    destination_conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let copy_result = (|| -> Result<()> {
        let backup = rusqlite::backup::Backup::new(&source_conn, &mut destination_conn)
            .with_context(|| {
                format!(
                    "backup コピーの準備に失敗しました: {} -> {}",
                    source.display(),
                    destination.display()
                )
            })?;
        match backup.step(-1).with_context(|| {
            format!(
                "backup コピーに失敗しました: {} -> {}",
                source.display(),
                destination.display()
            )
        })? {
            StepResult::Done => Ok(()),
            StepResult::Busy | StepResult::Locked => {
                bail!(
                    "backup 用の読み取りロックを取得できません: {}",
                    source.display()
                )
            }
            StepResult::More => unreachable!("backup with step(-1) should finish in one step"),
            _ => bail!("backup が未知の状態を返しました: {}", source.display()),
        }
    })();
    drop(destination_conn);
    if let Err(error) = copy_result {
        remove_sqlite_database_files(destination);
        return Err(error);
    }
    Ok(())
}

pub fn restore(
    config: &Config,
    database_selector: &str,
    backup_path: &Path,
) -> Result<RestoreReport> {
    validate_runtime_config(config)?;
    let backup_path = config.resolve_path(backup_path);
    let databases = select_databases(
        config,
        &DatabaseSelection {
            database: Some(database_selector.to_string()),
            ..DatabaseSelection::default()
        },
    )?;
    let database = databases
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("指定されたDBが見つかりません: {database_selector}"))?;
    match restore_database_inner(config, database, &backup_path) {
        Ok(report) => Ok(report),
        Err(error) => {
            let database =
                find_database_by_selector(config, database_selector).unwrap_or_else(|_| Database {
                    id: database_selector.to_string(),
                    path: config.resolve_path(database_selector),
                    exists: false,
                    readable: false,
                });
            Ok(RestoreReport {
                database,
                restored_from: backup_path,
                pre_restore_backup: None,
                success: false,
                error: Some(error.to_string()),
            })
        }
    }
}

fn restore_database_inner(
    config: &Config,
    database: Database,
    backup_path: &Path,
) -> Result<RestoreReport> {
    let _operation_lock = acquire_database_operation_lock(config, &database.path, "restore")?;
    config.validate_resolved_path_within_base("backup path", backup_path)?;
    ensure_existing_database_file(backup_path)?;
    validate_sqlite_database_file(backup_path, "restore元backup")?;
    config.validate_resolved_path_within_base("DBパス", &database.path)?;
    ensure_existing_database_file(&database.path)?;
    let restore_source = RestoreSourceCopy::new(config, backup_path, &database.path)?;
    let pre_restore_backup = backup_database_inner_result(
        config,
        &database,
        &[
            backup_path.to_path_buf(),
            restore_source.path().to_path_buf(),
        ],
    );
    if !pre_restore_backup.success {
        let error = pre_restore_backup
            .error
            .clone()
            .unwrap_or_else(|| "restore 前backupに失敗しました".to_string());
        return Ok(RestoreReport {
            database: refresh_database_state(&database),
            restored_from: backup_path.to_path_buf(),
            pre_restore_backup: Some(pre_restore_backup),
            success: false,
            error: Some(error),
        });
    }
    if let Err(error) = ensure_database_file_writable_for_restore(&database.path).and_then(|_| {
        restore_database_file_with_sqlite_backup(config, &database, restore_source.path())
    }) {
        return Ok(RestoreReport {
            database: refresh_database_state(&database),
            restored_from: backup_path.to_path_buf(),
            pre_restore_backup: Some(pre_restore_backup),
            success: false,
            error: Some(error.to_string()),
        });
    }
    Ok(RestoreReport {
        database: refresh_database_state(&database),
        restored_from: backup_path.to_path_buf(),
        pre_restore_backup: Some(pre_restore_backup),
        success: true,
        error: None,
    })
}

fn ensure_database_file_writable_for_restore(path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("restore対象DBのmetadataを読めません: {}", path.display()))?;
    if metadata.permissions().readonly() {
        bail!("restore対象DBが読み取り専用です: {}", path.display());
    }
    Ok(())
}

fn validate_sqlite_database_file(path: &Path, label: &str) -> Result<()> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("{label}をSQLiteとして開けません: {}", path.display()))?;
    let quick_check: String = conn
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .with_context(|| {
            format!(
                "{label}のPRAGMA quick_checkに失敗しました: {}",
                path.display()
            )
        })?;
    if quick_check != "ok" {
        bail!("{label}のPRAGMA quick_checkがokではありません: {quick_check}");
    }
    Ok(())
}

struct RestoreSourceCopy {
    path: PathBuf,
}

impl RestoreSourceCopy {
    fn new(config: &Config, source: &Path, destination: &Path) -> Result<Self> {
        let parent = destination
            .parent()
            .ok_or_else(|| anyhow::anyhow!("DBパスの親ディレクトリが不正です"))?;
        let file_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "DBファイル名がUTF-8ではありません: {}",
                    destination.display()
                )
            })?;
        let path = parent.join(format!(
            ".{file_name}.sqlite-fleet-restore-source-{}.tmp",
            unix_timestamp_nanos()
        ));
        let copy_result = (|| -> Result<()> {
            backup_database_file_with_sqlite_backup(config, source, &path).with_context(|| {
                format!(
                    "restore元backupの一貫した一時コピーを作成できません: {} -> {}",
                    source.display(),
                    path.display()
                )
            })?;
            validate_sqlite_database_file(&path, "restore元backup一時コピー")?;
            File::open(&path)
                .with_context(|| {
                    format!("restore元backup一時コピーを開けません: {}", path.display())
                })?
                .sync_all()
                .with_context(|| {
                    format!(
                        "restore元backup一時コピーを同期できません: {}",
                        path.display()
                    )
                })?;
            Ok(())
        })();
        if let Err(error) = copy_result {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RestoreSourceCopy {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn restore_database_file_with_sqlite_backup(
    config: &Config,
    database: &Database,
    restore_source: &Path,
) -> Result<()> {
    let source_conn = open_existing_readonly(restore_source)
        .with_context(|| format!("restore元backupを開けません: {}", restore_source.display()))?;
    source_conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let mut target_conn = open_existing_readwrite(&database.path)
        .with_context(|| format!("restore用にDBを開けません: {}", database.path.display()))?;
    target_conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let backup =
        rusqlite::backup::Backup::new(&source_conn, &mut target_conn).with_context(|| {
            format!(
                "restore元backupを対象DBへ書き戻す準備に失敗しました: {} -> {}",
                restore_source.display(),
                database.path.display()
            )
        })?;
    match backup.step(-1).with_context(|| {
        format!(
            "restore元backupを対象DBへ書き戻せません: {} -> {}",
            restore_source.display(),
            database.path.display()
        )
    })? {
        StepResult::Done => Ok(()),
        StepResult::Busy | StepResult::Locked => {
            bail!(
                "restore用の排他ロックを取得できません: {}",
                database.path.display()
            )
        }
        StepResult::More => unreachable!("restore with step(-1) should finish in one step"),
        _ => bail!(
            "restore が未知の状態を返しました: {}",
            database.path.display()
        ),
    }
}

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

impl From<&Migration> for MigrationSummary {
    fn from(migration: &Migration) -> Self {
        Self {
            group: migration.group.clone(),
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
