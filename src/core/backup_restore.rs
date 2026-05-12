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

