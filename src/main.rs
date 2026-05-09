use anyhow::{bail, Error, Result};
use clap::{Parser, Subcommand};
use sqlite_fleet::{
    check, discover_databases, doctor_and_write_report, init_project, load_migrations, migrate,
    status_report, write_report_json, Config, DEFAULT_CONFIG_PATH,
};
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
    },
    Check,
    Doctor,
    Gui {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
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
                            println!("  {}_{}", migration.version, migration.name);
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
        } => {
            let mut config = load_config_with_overrides(&config, parallel)?;
            if continue_on_error {
                config.execution.continue_on_error = true;
            }
            let report = migrate(&config, dry_run, database.as_deref())?;
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
                    if !database.success {
                        println!(
                            "失敗: {} {}",
                            database.database.path.display(),
                            database.error.as_deref().unwrap_or("不明なエラー")
                        );
                    }
                }
            }
            let report_write_error = write_report_json(&config, &report).err();
            if report.failed_databases > 0 {
                bail!(
                    "{}件のDBでマイグレーションに失敗しました",
                    report.failed_databases
                );
            }
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
        Commands::Gui { host, port } => {
            let config = load_config_with_overrides(&config, parallel)?;
            gui::serve(config, &host, port)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_command_uses_documented_defaults() {
        let cli = Cli::parse_from(["sqlite-fleet", "gui"]);

        match cli.command {
            Commands::Gui { host, port } => {
                assert_eq!(host, "127.0.0.1");
                assert_eq!(port, 8765);
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
            Commands::Gui { host, port } => {
                assert_eq!(host, "localhost");
                assert_eq!(port, 18782);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
