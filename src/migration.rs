use crate::{
    discovery::validate_configured_db_path,
    migration_sql::{validate_migration_sql, validate_migration_sql_for_history_table},
    Config, Migration, MAIN_MIGRATION_GROUP,
};
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub fn load_migrations(config: &Config) -> Result<Vec<Migration>> {
    let mut migrations = Vec::new();
    if config.migration_groups.is_empty() {
        migrations.extend(load_migrations_from_dir(
            config,
            "migrations.dir",
            &config.migrations.dir,
            MAIN_MIGRATION_GROUP,
        )?);
    } else {
        let group_configs = config.effective_migration_groups();
        let base_migrations = if group_configs
            .values()
            .any(|group_config| group_config.dir.is_none() && !group_config.migrations.is_empty())
        {
            Some(load_migrations_from_dir(
                config,
                "migrations.dir",
                &config.migrations.dir,
                MAIN_MIGRATION_GROUP,
            )?)
        } else {
            None
        };
        for (group, group_config) in group_configs {
            if let Some(dir) = group_config.dir.as_deref() {
                let dir_label = format!("migration_groups.{group}.dir");
                let mut group_migrations =
                    load_migrations_from_dir(config, &dir_label, dir, &group)?;
                if !group_config.migrations.is_empty() {
                    let versions = group_config
                        .migrations
                        .iter()
                        .map(String::as_str)
                        .collect::<HashSet<_>>();
                    group_migrations
                        .retain(|migration| versions.contains(migration.version.as_str()));
                    ensure_group_versions_exist(
                        &group,
                        &group_config.migrations,
                        &group_migrations,
                    )?;
                }
                migrations.extend(group_migrations);
            } else {
                if group_config.migrations.is_empty() {
                    continue;
                }
                let base_migrations = base_migrations
                    .as_ref()
                    .expect("base migrations are loaded for version-list migration groups");
                let mut group_migrations = Vec::new();
                for version in &group_config.migrations {
                    let mut migration = base_migrations
                        .iter()
                        .find(|migration| &migration.version == version)
                        .cloned()
                        .with_context(|| {
                            format!(
                                "migration_groups.{group} が存在しないversionを参照しています: {version}"
                            )
                        })?;
                    migration.group = group.clone();
                    group_migrations.push(migration);
                }
                migrations.extend(group_migrations);
            }
        }
    }

    migrations.sort_by(|a, b| {
        a.version_number
            .cmp(&b.version_number)
            .then_with(|| a.version.cmp(&b.version))
            .then_with(|| a.group.cmp(&b.group))
            .then_with(|| a.name.cmp(&b.name))
    });
    validate_migrations(config, &migrations)?;
    Ok(migrations)
}

fn load_migrations_from_dir(
    config: &Config,
    dir_label: &str,
    dir_value: &str,
    group: &str,
) -> Result<Vec<Migration>> {
    if dir_value.trim().is_empty() {
        bail!("{dir_label} は空にできません");
    }
    validate_configured_db_path(config, dir_label, dir_value)?;
    let dir = config.resolve_path(dir_value);
    if !dir.exists() {
        bail!("migrations ディレクトリが存在しません: {}", dir.display());
    }
    let mut migrations = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("migrations ディレクトリを読めません: {}", dir.display()))?
    {
        let entry = entry.context("migration ファイル一覧の取得に失敗しました")?;
        let path = entry.path();
        let extension = path.extension().and_then(|ext| ext.to_str());
        if !extension.is_some_and(|ext| ext.eq_ignore_ascii_case("sql")) {
            continue;
        }
        config.validate_resolved_path_within_base("migration ファイル", &path)?;
        let metadata = fs::metadata(&path).with_context(|| {
            format!(
                "migration ファイルのメタデータを取得できません: {}",
                path.display()
            )
        })?;
        if !metadata.is_file() {
            bail!(
                "migration ファイルは通常ファイルである必要があります: {}",
                path.display()
            );
        }
        let mut migration = parse_migration_file(&path)?;
        migration.group = group.to_string();
        migrations.push(migration);
    }
    Ok(migrations)
}

fn ensure_group_versions_exist(
    group: &str,
    versions: &[String],
    migrations: &[Migration],
) -> Result<()> {
    for version in versions {
        if !migrations
            .iter()
            .any(|migration| &migration.version == version)
        {
            bail!("migration_groups.{group} が存在しないversionを参照しています: {version}");
        }
    }
    Ok(())
}

pub fn validate_migration(config: &Config, migration: &Migration) -> Result<()> {
    validate_migrations(config, std::slice::from_ref(migration))
}

pub fn parse_migration_file(path: &Path) -> Result<Migration> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            anyhow!(
                "migration ファイル名がUTF-8ではありません: {}",
                path.display()
            )
        })?;
    let extension = path.extension().and_then(|ext| ext.to_str());
    if extension != Some("sql") {
        bail!("migration ファイルの拡張子は小文字 .sql である必要があります: {file_name}");
    }
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("migration ファイル名が不正です: {}", path.display()))?;
    let (version, name) = stem.split_once('_').ok_or_else(|| {
        anyhow!("migration ファイル名は <version>_<name>.sql 形式が必要です: {file_name}")
    })?;
    if version.is_empty() || name.is_empty() {
        bail!("migration ファイル名は <version>_<name>.sql 形式が必要です: {file_name}");
    }
    if !version.chars().all(|ch| ch.is_ascii_digit()) {
        bail!(
            "migration version は数値である必要があります。ASCII数字のみ使用できます: {file_name}"
        );
    }
    if !is_valid_migration_name(name) {
        bail!("migration name は英数字、_、- のみ使用できます: {file_name}");
    }
    let version_number = version
        .parse::<u64>()
        .with_context(|| format!("migration version は数値である必要があります: {file_name}"))?;
    let sql = fs::read_to_string(path)
        .with_context(|| format!("migration ファイルを読み込めません: {}", path.display()))?;
    if sql.trim().is_empty() {
        bail!("migration SQL が空です: {file_name}");
    }
    validate_migration_sql(&sql)?;
    let checksum = checksum_sql(&sql);
    Ok(Migration {
        version: version.to_string(),
        group: MAIN_MIGRATION_GROUP.to_string(),
        version_number,
        name: name.to_string(),
        checksum,
        path: path.to_path_buf(),
        sql,
    })
}

pub fn checksum_sql(sql: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sql.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn validate_migrations(config: &Config, migrations: &[Migration]) -> Result<()> {
    let mut seen_versions: std::collections::HashMap<&str, &Migration> =
        std::collections::HashMap::new();
    let mut seen_numbers: std::collections::HashMap<u64, &Migration> =
        std::collections::HashMap::new();
    for migration in migrations {
        if migration.group.trim().is_empty() || migration.group.trim() != migration.group {
            bail!("migration group は空白なしの非空文字列である必要があります");
        }
        config.validate_resolved_path_within_base("migration ファイル", &migration.path)?;
        let expected_file_name = format!("{}_{}.sql", migration.version, migration.name);
        if migration.path.file_name().and_then(|name| name.to_str()) != Some(&expected_file_name) {
            bail!(
                "migration path のファイル名がversion/nameと一致しません: expected={} actual={}",
                expected_file_name,
                migration.path.display()
            );
        }
        if !migration.version.chars().all(|ch| ch.is_ascii_digit()) {
            bail!(
                "migration version は数値である必要があります。ASCII数字のみ使用できます: {}",
                migration.version
            );
        }
        let parsed_version = migration.version.parse::<u64>().with_context(|| {
            format!(
                "migration version は数値である必要があります: {}",
                migration.version
            )
        })?;
        if parsed_version != migration.version_number {
            bail!(
                "migration version_number がversionと一致しません: version={} version_number={}",
                migration.version,
                migration.version_number
            );
        }
        if !is_valid_migration_name(&migration.name) {
            bail!(
                "migration name は英数字、_、- のみ使用できます: {}",
                migration.name
            );
        }
        if migration.sql.trim().is_empty() {
            bail!(
                "migration SQL が空です: {}_{}",
                migration.version,
                migration.name
            );
        }
        validate_migration_sql(&migration.sql)?;
        validate_migration_sql_for_history_table(&migration.sql, config.migrations_table())?;
        let actual_checksum = checksum_sql(&migration.sql);
        if migration.checksum != actual_checksum {
            bail!(
                "migration checksum がSQL内容と一致しません: {}_{} expected={} actual={}",
                migration.version,
                migration.name,
                actual_checksum,
                migration.checksum
            );
        }
        if let Some(previous) = seen_versions.insert(&migration.version, migration) {
            if previous.name != migration.name
                || previous.checksum != migration.checksum
                || previous.path != migration.path
            {
                bail!(
                    "migration version が重複しています: {}。同じversionを複数グループで共有する場合は同じファイルを参照してください",
                    migration.version
                );
            }
        }
        if let Some(previous) = seen_numbers.insert(migration.version_number, migration) {
            if previous.version != migration.version
                || previous.name != migration.name
                || previous.checksum != migration.checksum
                || previous.path != migration.path
            {
                bail!(
                    "migration version が数値として重複しています: {}",
                    migration.version_number
                );
            }
        }
    }
    Ok(())
}

fn is_valid_migration_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch == '-' || ch.is_ascii_alphanumeric())
}
