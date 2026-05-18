use anyhow::{bail, Error, Result};
use clap::{Parser, Subcommand};
use sqlite_fleet::{
    backup, check, discover_databases, doctor_and_write_report, init_project, load_migrations,
    migrate_with_options, restore, schema_drift, status_report, write_audit_event,
    write_report_json, Config, DatabaseSelection, MigrateOptions, DEFAULT_CONFIG_PATH,
};
use std::net::ToSocketAddrs;
use std::path::{Path, PathBuf};

mod gui;

#[derive(Debug, Parser)]
#[command(name = "sqlite-fleet")]
#[command(about = "複数のSQLite DBに対するマイグレーションを管理します")]
struct Cli {
    #[arg(short, long, global = true, default_value = DEFAULT_CONFIG_PATH)]
    config: PathBuf,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    parallel: Option<usize>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init,
    Discover,
    Status,
    Plan,
    Migrate {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        continue_on_error: bool,
        #[arg(long)]
        database: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        backup: bool,
        #[arg(long)]
        no_backup: bool,
    },
    Backup {
        #[arg(long)]
        database: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    Restore {
        #[arg(long)]
        database: String,
        #[arg(long)]
        from: PathBuf,
    },
    Drift {
        #[arg(long)]
        database: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    Check,
    Doctor,
    Gui {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[arg(long)]
        allow_remote: bool,
        #[arg(long)]
        ssh_user: Option<String>,
        #[arg(long)]
        ssh_host: Option<String>,
        #[arg(long)]
        ssh_port: Option<u16>,
        #[arg(long)]
        local_port: Option<u16>,
    },
}

fn main() -> Result<()> {
    let Cli {
        config,
        json,
        parallel,
        command,
    } = Cli::parse();

    match command {
        Commands::Init => {
            let existed = config.exists();
            init_project(&config)?;
            if existed {
                println!("既存の設定ファイルを維持しました: {}", config.display());
            } else {
                println!("初期設定を作成しました: {}", config.display());
            }
        }
        Commands::Discover => {
            let config = load_discovery_config_with_overrides(&config, parallel)?;
            let databases = discover_databases(&config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&databases)?);
            } else {
                for database in &databases {
                    println!(
                        "{}\t{}\t存在={}\t読取可能={}",
                        database.id,
                        database.path.display(),
                        database.exists,
                        database.readable
                    );
                }
                println!("DB数: {}", databases.len());
            }
        }
        Commands::Status => {
            let config = load_config_with_overrides(&config, parallel)?;
            let report = status_report(&config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("DB数: {}", report.database_count);
                println!(
                    "最新マイグレーション: {}",
                    report
                        .latest_migration
                        .as_ref()
                        .map(|migration| format!("{}_{}", migration.version, migration.name))
                        .unwrap_or_else(|| "(なし)".to_string())
                );
                println!();
                println!("最新適用済み: {}", report.up_to_date);
                println!("未適用あり:   {}", report.pending);
                println!("失敗:         {}", report.failed);
                println!("DBなし:       {}", report.missing);
                println!("不整合:       {}", report.corrupt);
            }
            let report_write_error = write_report_json(&config, &report).err();
            if report.failed > 0 || report.corrupt > 0 {
                bail!(
                    "status で問題を検出しました: failed={}, corrupt={}",
                    report.failed,
                    report.corrupt
                );
            }
            fail_if_report_write_failed(report_write_error)?;
        }
        Commands::Plan => {
            let config = load_config_with_overrides(&config, parallel)?;
            let databases = discover_databases(&config)?;
            if databases.is_empty() {
                bail!("対象DBが見つかりません");
            }
            let migrations = load_migrations(&config)?;
            let plans = sqlite_fleet::build_plan(&config, &databases, &migrations);
            let failed = plans.iter().filter(|plan| plan.error.is_some()).count();
            let corrupt = plans
                .iter()
                .filter(|plan| !plan.checksum_errors.is_empty() || !plan.unknown_applied.is_empty())
                .count();
            if json {
                println!("{}", serde_json::to_string_pretty(&plans)?);
            } else {
                for plan in &plans {
                    println!("{}", plan.database.path.display());
                    if let Some(error) = &plan.error {
                        println!("  エラー: {error}");
                        continue;
                    }
                    for migration in &plan.unknown_applied {
                        println!(
                            "  不明な適用済みmigration: {}_{}",
                            migration.version, migration.name
                        );
                    }
                    for error in &plan.checksum_errors {
                        println!(
                            "  checksum不一致: {} expected={} actual={}",
                            error.version, error.expected, error.actual
                        );
                    }
                    if plan.pending.is_empty() {
                        println!("  適用予定なし");
                    } else {
                        for migration in &plan.pending {
                            println!(
                                "  [{}] {}_{}",
                                migration.group, migration.version, migration.name
                            );
                        }
                    }
                }
            }
            let report_write_error = write_report_json(&config, &plans).err();
            if failed > 0 || corrupt > 0 {
                bail!("plan で問題を検出しました: failed={failed}, corrupt={corrupt}");
            }
            fail_if_report_write_failed(report_write_error)?;
        }
        Commands::Migrate {
            dry_run,
            continue_on_error,
            database,
            group,
            limit,
            backup,
            no_backup,
        } => {
            let mut config = load_config_with_overrides(&config, parallel)?;
            if continue_on_error {
                config.execution.continue_on_error = true;
            }
            let backup_before_migrate = if backup {
                Some(true)
            } else if no_backup {
                Some(false)
            } else {
                None
            };
            let report = migrate_with_options(
                &config,
                MigrateOptions {
                    dry_run,
                    selection: DatabaseSelection {
                        database,
                        group,
                        limit,
                    },
                    backup_before_migrate,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("ドライラン: {}", report.dry_run);
                println!("DB数: {}", report.database_count);
                println!("処理済みDB数: {}", report.processed_databases);
                println!("未適用ありDB数: {}", report.pending_databases);
                println!("適用DB数: {}", report.applied_databases);
                println!("失敗DB数: {}", report.failed_databases);
                for database in &report.databases {
                    if let Some(backup) = &database.pre_backup {
                        if let Some(path) = &backup.path {
                            println!("backup: {} -> {}", database.database.id, path.display());
                        }
                    }
                    if !database.success {
                        println!(
                            "失敗: {} {}",
                            database.database.path.display(),
                            database.error.as_deref().unwrap_or("不明なエラー")
                        );
                    }
                }
            }
            let audit_error = write_audit_event(&config, "migrate", &report).err();
            let report_write_error = write_report_json(&config, &report).err();
            if report.failed_databases > 0 {
                bail!(
                    "{}件のDBでマイグレーションに失敗しました",
                    report.failed_databases
                );
            }
            fail_if_report_write_failed(audit_error)?;
            fail_if_report_write_failed(report_write_error)?;
        }
        Commands::Backup {
            database,
            group,
            limit,
        } => {
            let config = load_config_with_overrides(&config, parallel)?;
            let report = backup(
                &config,
                DatabaseSelection {
                    database,
                    group,
                    limit,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("DB数: {}", report.database_count);
                println!("backup成功: {}", report.backed_up);
                println!("失敗: {}", report.failed);
                for backup in &report.backups {
                    if let Some(path) = &backup.path {
                        println!("{} -> {}", backup.database.id, path.display());
                    }
                    if !backup.success {
                        println!(
                            "失敗: {} {}",
                            backup.database.path.display(),
                            backup.error.as_deref().unwrap_or("backupに失敗しました")
                        );
                    }
                }
            }
            let audit_error = write_audit_event(&config, "backup", &report).err();
            let report_write_error = write_report_json(&config, &report).err();
            if report.failed > 0 {
                bail!("{}件のDB backupに失敗しました", report.failed);
            }
            fail_if_report_write_failed(audit_error)?;
            fail_if_report_write_failed(report_write_error)?;
        }
        Commands::Restore { database, from } => {
            let config = load_config_with_overrides(&config, parallel)?;
            let report = restore(&config, &database, &from)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if report.success {
                println!("復元しました: {} <- {}", report.database.id, from.display());
                if let Some(backup) = &report.pre_restore_backup {
                    if let Some(path) = &backup.path {
                        println!("復元前backup: {}", path.display());
                    }
                }
            } else {
                println!(
                    "復元に失敗しました: {}",
                    report.error.as_deref().unwrap_or("不明なエラー")
                );
            }
            let audit_error = write_audit_event(&config, "restore", &report).err();
            let report_write_error = write_report_json(&config, &report).err();
            if !report.success {
                bail!("restore に失敗しました");
            }
            fail_if_report_write_failed(audit_error)?;
            fail_if_report_write_failed(report_write_error)?;
        }
        Commands::Drift {
            database,
            group,
            limit,
        } => {
            let config = load_config_with_overrides(&config, parallel)?;
            let report = schema_drift(
                &config,
                DatabaseSelection {
                    database,
                    group,
                    limit,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("DB数: {}", report.database_count);
                println!("drift: {}", report.drifted);
                println!("失敗: {}", report.failed);
                for database in &report.databases {
                    if !database.success {
                        println!(
                            "失敗: {} {}",
                            database.database.path.display(),
                            database
                                .error
                                .as_deref()
                                .unwrap_or("schema drift検査に失敗しました")
                        );
                    } else if !database.matches_baseline {
                        println!("drift: {}", database.database.id);
                        for object in &database.missing_objects {
                            println!("  missing: {object}");
                        }
                        for object in &database.extra_objects {
                            println!("  extra: {object}");
                        }
                        for object in &database.changed_objects {
                            println!("  changed: {object}");
                        }
                    }
                }
            }
            let audit_error = write_audit_event(&config, "drift", &report).err();
            let report_write_error = write_report_json(&config, &report).err();
            if report.failed > 0 || report.drifted > 0 {
                bail!(
                    "schema drift を検出しました: drifted={}, failed={}",
                    report.drifted,
                    report.failed
                );
            }
            fail_if_report_write_failed(audit_error)?;
            fail_if_report_write_failed(report_write_error)?;
        }
        Commands::Check => {
            let config = load_config_with_overrides(&config, parallel)?;
            let report = check(&config)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("DB数: {}", report.database_count);
                println!("正常: {}", report.ok);
                println!("失敗: {}", report.failed);
                for database in &report.databases {
                    if !database.success {
                        println!(
                            "失敗: {} {}",
                            database.database.path.display(),
                            database.error.as_deref().unwrap_or("検査に失敗しました")
                        );
                    }
                }
            }
            let report_write_error = write_report_json(&config, &report).err();
            if report.failed > 0 {
                bail!("{}件のDB検査に失敗しました", report.failed);
            }
            fail_if_report_write_failed(report_write_error)?;
        }
        Commands::Doctor => {
            let report = doctor_and_write_report(&config, parallel);
            let ok = report.config_ok
                && report.discovery_ok
                && report.migrations_ok
                && report.errors.is_empty();
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("設定: {}", ok_text(report.config_ok));
                println!("DB探索: {}", ok_text(report.discovery_ok));
                println!("マイグレーション: {}", ok_text(report.migrations_ok));
                println!("DB数: {}", report.database_count);
                println!("マイグレーション数: {}", report.migration_count);
                for error in &report.errors {
                    println!("エラー: {error}");
                }
            }
            if !ok {
                bail!("doctor で問題を検出しました");
            }
        }
        Commands::Gui {
            host,
            port,
            allow_remote,
            ssh_user,
            ssh_host,
            ssh_port,
            local_port,
        } => {
            let loaded_config = load_config_with_overrides(&config, parallel)?;
            validate_gui_remote_startup(&loaded_config, &host, allow_remote)?;
            gui::serve(
                loaded_config,
                config.clone(),
                &host,
                port,
                allow_remote,
                gui::GuiAccessOptions {
                    ssh_user,
                    ssh_host,
                    ssh_port,
                    local_port,
                },
            )?;
        }
    }

    Ok(())
}

fn load_config_with_overrides(config_path: &Path, parallel: Option<usize>) -> Result<Config> {
    let mut config = Config::load_for_operation(config_path)?;
    if let Some(parallel) = parallel {
        config.execution.parallel = parallel;
        config.validate_operation()?;
    }
    Ok(config)
}

fn load_discovery_config_with_overrides(
    config_path: &Path,
    parallel: Option<usize>,
) -> Result<Config> {
    let mut config = Config::load_for_discovery(config_path)?;
    if let Some(parallel) = parallel {
        config.execution.parallel = parallel;
        if parallel == 0 {
            bail!("execution.parallel は1以上が必要です");
        }
    }
    Ok(config)
}

fn ok_text(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "ng"
    }
}

fn fail_if_report_write_failed(error: Option<Error>) -> Result<()> {
    if let Some(error) = error {
        Err(error)
    } else {
        Ok(())
    }
}

fn validate_gui_remote_startup(config: &Config, host: &str, allow_remote: bool) -> Result<()> {
    if !gui_host_is_remote(host)? {
        return Ok(());
    }
    if !allow_remote {
        bail!("GUI host が外部公開アドレスです。外部公開する場合は --allow-remote を指定してください: {host}");
    }
    if config.gui_users.is_empty() {
        bail!("外部公開するGUIでは gui_users の初期管理ユーザーが必要です");
    }
    if !config
        .gui_users
        .values()
        .any(|user| user.permissions.allow_gui_permission_edit)
    {
        bail!("外部公開するGUIでは allow_gui_permission_edit を持つGUIユーザーが1人以上必要です");
    }
    Ok(())
}

fn gui_host_is_remote(host: &str) -> Result<bool> {
    let addrs = (host, 0)
        .to_socket_addrs()
        .map_err(|error| anyhow::anyhow!("GUI host を解決できません: {host}: {error}"))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        bail!("GUI host を解決できません: {host}");
    }
    Ok(!addrs.iter().all(|addr| addr.ip().is_loopback()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_command_uses_documented_defaults() {
        let cli = Cli::parse_from(["sqlite-fleet", "gui"]);

        match cli.command {
            Commands::Gui {
                host,
                port,
                allow_remote,
                ssh_user,
                ssh_host,
                ssh_port,
                local_port,
            } => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, 8765);
                assert!(!allow_remote);
                assert_eq!(ssh_user, None);
                assert_eq!(ssh_host, None);
                assert_eq!(ssh_port, None);
                assert_eq!(local_port, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn gui_command_accepts_host_and_port_overrides() {
        let cli = Cli::parse_from([
            "sqlite-fleet",
            "gui",
            "--host",
            "localhost",
            "--port",
            "18782",
        ]);

        match cli.command {
            Commands::Gui {
                host,
                port,
                allow_remote,
                ssh_user,
                ssh_host,
                ssh_port,
                local_port,
            } => {
                assert_eq!(host, "localhost");
                assert_eq!(port, 18782);
                assert!(!allow_remote);
                assert_eq!(ssh_user, None);
                assert_eq!(ssh_host, None);
                assert_eq!(ssh_port, None);
                assert_eq!(local_port, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn gui_command_accepts_explicit_remote_flag() {
        let cli = Cli::parse_from(["sqlite-fleet", "gui", "--host", "0.0.0.0", "--allow-remote"]);

        match cli.command {
            Commands::Gui {
                host, allow_remote, ..
            } => {
                assert_eq!(host, "0.0.0.0");
                assert!(allow_remote);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn gui_command_accepts_ssh_tunnel_hint_options() {
        let cli = Cli::parse_from([
            "sqlite-fleet",
            "gui",
            "--ssh-user",
            "ubuntu",
            "--ssh-host",
            "161.33.9.53",
            "--ssh-port",
            "2222",
            "--local-port",
            "9876",
        ]);

        match cli.command {
            Commands::Gui {
                ssh_user,
                ssh_host,
                ssh_port,
                local_port,
                ..
            } => {
                assert_eq!(ssh_user.as_deref(), Some("ubuntu"));
                assert_eq!(ssh_host.as_deref(), Some("161.33.9.53"));
                assert_eq!(ssh_port, Some(2222));
                assert_eq!(local_port, Some(9876));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn remote_gui_startup_requires_explicit_flag_and_admin_user() {
        let config = Config::default();
        let error = validate_gui_remote_startup(&config, "0.0.0.0", false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("--allow-remote"), "{error}");

        let error = validate_gui_remote_startup(&config, "0.0.0.0", true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("gui_users の初期管理ユーザー"), "{error}");

        let mut config = Config::default();
        config.gui_users.insert(
            "viewer".to_string(),
            sqlite_fleet::GuiUserConfig::with_hashed_token(
                "viewer-token",
                sqlite_fleet::GuiConfig::default(),
            )
            .unwrap(),
        );
        let error = validate_gui_remote_startup(&config, "0.0.0.0", true)
            .unwrap_err()
            .to_string();
        assert!(error.contains("allow_gui_permission_edit"), "{error}");

        config.gui_users.insert(
            "owner".to_string(),
            sqlite_fleet::GuiUserConfig::with_hashed_token(
                "owner-token",
                sqlite_fleet::GuiConfig {
                    allow_gui_permission_edit: true,
                    ..sqlite_fleet::GuiConfig::default()
                },
            )
            .unwrap(),
        );
        validate_gui_remote_startup(&config, "0.0.0.0", true).unwrap();
        validate_gui_remote_startup(&Config::default(), "127.0.0.1", false).unwrap();
    }
}
