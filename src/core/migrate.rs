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

