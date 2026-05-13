use crate::{sqlite_ident::validate_identifier, AppliedMigration, Migration};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;

pub fn ensure_migrations_table(conn: &Connection, table: &str) -> Result<()> {
    validate_identifier(table)?;
    conn.execute(&create_migrations_table_sql(table), [])
        .with_context(|| format!("migration 管理テーブルを作成できません: {table}"))?;
    validate_migrations_table_schema(conn, table)?;
    Ok(())
}

pub fn read_applied_migrations(conn: &Connection, table: &str) -> Result<Vec<AppliedMigration>> {
    if !table_exists(conn, table)? {
        return Ok(Vec::new());
    }
    match migrations_table_schema(conn, table)? {
        MigrationsTableSchema::Current => read_current_applied_migrations(conn, table),
        MigrationsTableSchema::LegacyVersionPrimaryKey => {
            read_legacy_applied_migrations(conn, table, &[], LegacyResolution::AllowUnresolved)
        }
    }
}

pub fn read_applied_migrations_with_catalog(
    conn: &Connection,
    table: &str,
    migrations: &[Migration],
) -> Result<Vec<AppliedMigration>> {
    if !table_exists(conn, table)? {
        return Ok(Vec::new());
    }
    match migrations_table_schema(conn, table)? {
        MigrationsTableSchema::Current => read_current_applied_migrations(conn, table),
        MigrationsTableSchema::LegacyVersionPrimaryKey => read_legacy_applied_migrations(
            conn,
            table,
            migrations,
            LegacyResolution::AllowUnresolved,
        ),
    }
}

pub fn migrate_legacy_migrations_table(
    conn: &mut Connection,
    table: &str,
    migrations: &[Migration],
) -> Result<()> {
    if !table_exists(conn, table)? {
        return Ok(());
    }
    if migrations_table_schema(conn, table)? != MigrationsTableSchema::LegacyVersionPrimaryKey {
        return Ok(());
    }
    let applied =
        read_legacy_applied_migrations(conn, table, migrations, LegacyResolution::RequireResolved)?;
    let legacy_table = format!("{table}__legacy_version_pk");
    validate_identifier(&legacy_table)?;
    if table_exists(conn, &legacy_table)? {
        bail!("legacy migration 管理テーブルの退避先が既に存在します: {legacy_table}");
    }
    let tx = conn
        .transaction()
        .with_context(|| format!("migration 管理テーブルを移行できません: {table}"))?;
    tx.execute(
        &format!("ALTER TABLE main.{table} RENAME TO {legacy_table}"),
        [],
    )
    .with_context(|| format!("migration 管理テーブルを退避できません: {table}"))?;
    tx.execute(&create_migrations_table_sql(table), [])
        .with_context(|| format!("migration 管理テーブルを作成できません: {table}"))?;
    for migration in applied {
        tx.execute(
            &format!(
                "INSERT INTO main.{table} (filename, version, name, checksum, applied_at, execution_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
            ),
            rusqlite::params![
                migration.filename,
                migration.version,
                migration.name,
                migration.checksum,
                migration.applied_at,
                migration.execution_ms
            ],
        )
        .with_context(|| format!("migration 管理テーブルの履歴を移行できません: {table}"))?;
    }
    tx.execute(&format!("DROP TABLE main.{legacy_table}"), [])
        .with_context(|| {
            format!("legacy migration 管理テーブルを削除できません: {legacy_table}")
        })?;
    tx.commit()
        .with_context(|| format!("migration 管理テーブルを移行できません: {table}"))?;
    validate_migrations_table_schema(conn, table)?;
    Ok(())
}

fn read_current_applied_migrations(
    conn: &Connection,
    table: &str,
) -> Result<Vec<AppliedMigration>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT filename, version, name, checksum, applied_at, execution_ms FROM main.{table} ORDER BY filename"
        ))
        .with_context(|| format!("migration 管理テーブルを読めません: {table}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AppliedMigration {
                filename: row.get(0)?,
                version: row.get(1)?,
                name: row.get(2)?,
                checksum: row.get(3)?,
                applied_at: row.get(4)?,
                execution_ms: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn read_legacy_applied_migrations(
    conn: &Connection,
    table: &str,
    migrations: &[Migration],
    resolution: LegacyResolution,
) -> Result<Vec<AppliedMigration>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT version, name, checksum, applied_at, execution_ms FROM main.{table} ORDER BY version"
        ))
        .with_context(|| format!("migration 管理テーブルを読めません: {table}"))?;
    let rows = stmt
        .query_map([], |row| {
            let version = row.get::<_, String>(0)?;
            let checksum = row.get::<_, String>(2)?;
            Ok((
                version,
                row.get::<_, String>(1)?,
                checksum,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(version, name, checksum, applied_at, execution_ms)| {
            let filename = resolve_legacy_filename(&version, &checksum, migrations, resolution)?;
            Ok(AppliedMigration {
                filename,
                version,
                name,
                checksum,
                applied_at,
                execution_ms,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyResolution {
    AllowUnresolved,
    RequireResolved,
}

fn resolve_legacy_filename(
    version: &str,
    checksum: &str,
    migrations: &[Migration],
    resolution: LegacyResolution,
) -> Result<String> {
    let checksum_matches = unique_filenames(
        migrations
            .iter()
            .filter(|migration| migration.version == version && migration.checksum == checksum),
    );
    match checksum_matches.as_slice() {
        [filename] => return Ok(filename.clone()),
        filenames if filenames.len() > 1 => {
            bail!(
                "legacy migration履歴のversionが複数ファイルに一致します: {version} files={}",
                filenames.join(", ")
            );
        }
        _ => {}
    }

    let version_matches = unique_filenames(
        migrations
            .iter()
            .filter(|migration| migration.version == version),
    );
    match version_matches.as_slice() {
        [filename] => Ok(filename.clone()),
        [] if resolution == LegacyResolution::AllowUnresolved => Ok(version.to_string()),
        [] => bail!("legacy migration履歴のversionをローカルmigrationへ解決できません: {version}"),
        filenames => bail!(
            "legacy migration履歴のversionが曖昧です: {version} files={}",
            filenames.join(", ")
        ),
    }
}

fn unique_filenames<'a>(migrations: impl Iterator<Item = &'a Migration>) -> Vec<String> {
    let mut filenames = migrations
        .map(|migration| migration.filename.clone())
        .collect::<Vec<_>>();
    filenames.sort();
    filenames.dedup();
    filenames
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    validate_identifier(table)?;
    let object_type = conn
        .query_row(
            "SELECT type FROM main.sqlite_master WHERE name = ?1",
            [table],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .with_context(|| format!("migration 管理オブジェクトの確認に失敗しました: {table}"))?;
    match object_type.as_deref() {
        None => Ok(false),
        Some("table") => Ok(true),
        Some(other) => bail!("migration 管理名がtable以外で使用されています: {table} type={other}"),
    }
}

fn validate_migrations_table_schema(conn: &Connection, table: &str) -> Result<()> {
    if migrations_table_schema(conn, table)? == MigrationsTableSchema::Current {
        return Ok(());
    }
    bail!("migration 管理テーブルの列が不足しています: {table}.filename");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationsTableSchema {
    Current,
    LegacyVersionPrimaryKey,
}

fn migrations_table_schema(conn: &Connection, table: &str) -> Result<MigrationsTableSchema> {
    validate_identifier(table)?;
    let mut stmt = conn
        .prepare(&format!("PRAGMA main.table_info({table})"))
        .with_context(|| format!("migration 管理テーブルのスキーマを確認できません: {table}"))?;
    let columns = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                ColumnInfo {
                    column_type: row.get::<_, String>(2)?.to_ascii_uppercase(),
                    not_null: row.get::<_, i64>(3)? != 0,
                    primary_key_position: row.get::<_, i64>(5)?,
                },
            ))
        })?
        .collect::<std::result::Result<HashMap<_, _>, _>>()?;
    if is_legacy_version_primary_key_schema(&columns)? {
        return Ok(MigrationsTableSchema::LegacyVersionPrimaryKey);
    }
    for (required, expected_type) in [
        ("version", "TEXT"),
        ("filename", "TEXT"),
        ("name", "TEXT"),
        ("checksum", "TEXT"),
        ("applied_at", "INTEGER"),
        ("execution_ms", "INTEGER"),
    ] {
        let Some(column) = columns.get(required) else {
            bail!("migration 管理テーブルの列が不足しています: {table}.{required}");
        };
        if column.column_type != expected_type {
            bail!(
                "migration 管理テーブルの列型が不正です: {table}.{required} expected={expected_type} actual={}",
                column.column_type
            );
        }
    }
    if columns.len() != 6 {
        let mut actual = columns.keys().cloned().collect::<Vec<_>>();
        actual.sort();
        bail!(
            "migration 管理テーブルに想定外の列があります: {table} columns={}",
            actual.join(", ")
        );
    }
    let filename = columns
        .get("filename")
        .ok_or_else(|| anyhow!("migration 管理テーブルの列が不足しています: {table}.filename"))?;
    if filename.primary_key_position != 1 {
        bail!("migration 管理テーブルの主キー制約が不足しています: {table}.filename");
    }
    for non_key in ["version", "name", "checksum", "applied_at", "execution_ms"] {
        let Some(column) = columns.get(non_key) else {
            bail!("migration 管理テーブルの列が不足しています: {table}.{non_key}");
        };
        if column.primary_key_position != 0 {
            bail!(
                "migration 管理テーブルの主キーはfilename単独である必要があります: {table}.{non_key}"
            );
        }
    }
    for required in [
        "filename",
        "version",
        "name",
        "checksum",
        "applied_at",
        "execution_ms",
    ] {
        let Some(column) = columns.get(required) else {
            bail!("migration 管理テーブルの列が不足しています: {table}.{required}");
        };
        if !column.not_null {
            bail!("migration 管理テーブルのNOT NULL制約が不足しています: {table}.{required}");
        }
    }
    Ok(MigrationsTableSchema::Current)
}

fn is_legacy_version_primary_key_schema(columns: &HashMap<String, ColumnInfo>) -> Result<bool> {
    if columns.contains_key("filename") {
        return Ok(false);
    }
    for (required, expected_type) in [
        ("version", "TEXT"),
        ("name", "TEXT"),
        ("checksum", "TEXT"),
        ("applied_at", "INTEGER"),
        ("execution_ms", "INTEGER"),
    ] {
        let Some(column) = columns.get(required) else {
            return Ok(false);
        };
        if column.column_type != expected_type {
            return Ok(false);
        }
    }
    if columns.len() != 5 {
        let mut actual = columns.keys().cloned().collect::<Vec<_>>();
        actual.sort();
        bail!(
            "migration 管理テーブルに想定外の列があります: columns={}",
            actual.join(", ")
        );
    }
    let version = columns
        .get("version")
        .ok_or_else(|| anyhow!("migration 管理テーブルの列が不足しています: version"))?;
    if version.primary_key_position != 1 {
        return Ok(false);
    }
    for required in ["version", "name", "checksum", "applied_at", "execution_ms"] {
        let Some(column) = columns.get(required) else {
            return Ok(false);
        };
        if !column.not_null {
            return Ok(false);
        }
    }
    Ok(true)
}

struct ColumnInfo {
    column_type: String,
    not_null: bool,
    primary_key_position: i64,
}

pub(crate) fn create_migrations_table_sql(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS main.{table} (
            filename TEXT PRIMARY KEY NOT NULL,
            version TEXT NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )"
    )
}
