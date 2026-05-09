use crate::{
    config::{Config, DoctorReport},
    discovery::{discover_databases, validate_configured_db_path},
    migration::load_migrations,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn doctor(config_path: impl AsRef<Path>) -> DoctorReport {
    doctor_with_overrides(config_path, None)
}

pub fn doctor_with_overrides(
    config_path: impl AsRef<Path>,
    parallel: Option<usize>,
) -> DoctorReport {
    let (report, _) = doctor_with_config(config_path, parallel);
    report
}

pub fn doctor_and_write_report(
    config_path: impl AsRef<Path>,
    parallel: Option<usize>,
) -> DoctorReport {
    let (mut report, config) = doctor_with_config(config_path, parallel);
    if let Some(config) = config {
        if let Err(error) = write_report_json(&config, &report) {
            report
                .errors
                .push(format!("JSONレポートを書き出せません: {error}"));
        }
    }
    report
}

fn doctor_with_config(
    config_path: impl AsRef<Path>,
    parallel: Option<usize>,
) -> (DoctorReport, Option<Config>) {
    let config = match Config::load_unvalidated(config_path.as_ref()) {
        Ok(config) => config,
        Err(error) => {
            return (config_error_report(error), None);
        }
    };
    let mut config = config;
    if let Some(parallel) = parallel {
        config.execution.parallel = parallel;
    }
    let report = doctor_config(&config);
    (report, Some(config))
}

fn config_error_report(error: impl ToString) -> DoctorReport {
    DoctorReport {
        config_ok: false,
        discovery_ok: false,
        migrations_ok: false,
        database_count: 0,
        migration_count: 0,
        errors: vec![error.to_string()],
    }
}

pub fn doctor_config(config: &Config) -> DoctorReport {
    let mut errors = Vec::new();
    let mut database_count = 0;
    let mut migration_count = 0;

    if let Err(error) = config.validate() {
        return config_error_report(error);
    }

    let discovery_ok = match discover_databases(config) {
        Ok(databases) => {
            database_count = databases.len();
            true
        }
        Err(error) => {
            errors.push(error.to_string());
            false
        }
    };

    let migrations_ok = match load_migrations(config) {
        Ok(migrations) => {
            migration_count = migrations.len();
            true
        }
        Err(error) => {
            errors.push(error.to_string());
            false
        }
    };

    DoctorReport {
        config_ok: true,
        discovery_ok,
        migrations_ok,
        database_count,
        migration_count,
        errors,
    }
}

pub fn write_report_json<T: Serialize>(config: &Config, value: &T) -> Result<Option<PathBuf>> {
    if config.report.format.trim() != config.report.format {
        bail!("report.format の前後に空白は使用できません");
    }
    if config.report.format != "json" {
        bail!("現在対応している report.format は json のみです");
    }
    let Some(path) = config.report.path.as_deref() else {
        return Ok(None);
    };
    if path.trim().is_empty() {
        bail!("report.path は空にできません");
    }
    validate_configured_db_path(config, "report.path", path)?;
    let path = config.resolve_path(path);
    validate_report_path(config, &path)?;
    let text = serde_json::to_string_pretty(value).context("JSONレポートを生成できません")?;
    write_report_atomically(&path, text.as_bytes())?;
    Ok(Some(path))
}

fn validate_report_path(config: &Config, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("report.path の親ディレクトリを解決できません"))?;
    config.validate_resolved_path_within_base("report.path", parent)?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "レポート出力先ディレクトリを作成できません: {}",
            parent.display()
        )
    })?;
    config.validate_resolved_path_within_base("report.path", parent)?;

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                bail!(
                    "report.path にシンボリックリンクは使用できません: {}",
                    path.display()
                );
            }
            if !metadata.is_file() {
                bail!(
                    "report.path は通常ファイルである必要があります: {}",
                    path.display()
                );
            }
            config.validate_resolved_path_within_base("report.path", path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            config.validate_resolved_path_within_base("report.path", path)?;
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "report.path のメタデータを取得できません: {}",
                    path.display()
                )
            });
        }
    }

    Ok(())
}

fn write_report_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("report.path の親ディレクトリを解決できません"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("report.path のファイル名を解決できません"))?;
    let tmp_name = format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        unique_report_suffix()
    );
    let tmp_path = parent.join(tmp_name);

    let write_result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .with_context(|| {
                format!(
                    "一時レポートファイルを作成できません: {}",
                    tmp_path.display()
                )
            })?;
        file.write_all(bytes)
            .context("JSONレポートを一時ファイルへ書き込めません")?;
        file.sync_all()
            .context("JSONレポートの一時ファイルを同期できません")?;
        fs::rename(&tmp_path, path)
            .with_context(|| format!("JSONレポートを書き込めません: {}", path.display()))?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    write_result
}

fn unique_report_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}.{}", std::process::id(), nanos)
}
