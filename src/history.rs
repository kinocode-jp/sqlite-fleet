use crate::{sqlite_ident::validate_identifier, AppliedMigration};
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
    validate_migrations_table_schema(conn, table)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT version, name, checksum, applied_at, execution_ms FROM main.{table} ORDER BY version"
        ))
        .with_context(|| format!("migration 管理テーブルを読めません: {table}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(AppliedMigration {
                version: row.get(0)?,
                name: row.get(1)?,
                checksum: row.get(2)?,
                applied_at: row.get(3)?,
                execution_ms: row.get(4)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
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
    for (required, expected_type) in [
        ("version", "TEXT"),
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
    if columns.len() != 5 {
        let mut actual = columns.keys().cloned().collect::<Vec<_>>();
        actual.sort();
        bail!(
            "migration 管理テーブルに想定外の列があります: {table} columns={}",
            actual.join(", ")
        );
    }
    let version = columns
        .get("version")
        .ok_or_else(|| anyhow!("migration 管理テーブルの列が不足しています: {table}.version"))?;
    if version.primary_key_position != 1 {
        bail!("migration 管理テーブルの主キー制約が不足しています: {table}.version");
    }
    for required in ["version", "name", "checksum", "applied_at", "execution_ms"] {
        let Some(column) = columns.get(required) else {
            bail!("migration 管理テーブルの列が不足しています: {table}.{required}");
        };
        if !column.not_null {
            bail!("migration 管理テーブルのNOT NULL制約が不足しています: {table}.{required}");
        }
    }
    Ok(())
}

struct ColumnInfo {
    column_type: String,
    not_null: bool,
    primary_key_position: i64,
}

pub(crate) fn create_migrations_table_sql(table: &str) -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS main.{table} (
            version TEXT PRIMARY KEY NOT NULL,
            name TEXT NOT NULL,
            checksum TEXT NOT NULL,
            applied_at INTEGER NOT NULL,
            execution_ms INTEGER NOT NULL
        )"
    )
}
