use crate::{
    discovery_query::validate_discovery_query, open_existing_readonly,
    path_utils::normalize_path_for_comparison, Config, Database,
};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{types::ValueRef, Row};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_TEMPLATE_WIDTH: usize = 1024;
const MAX_TEMPLATE_SPLIT: usize = 1024;

pub fn discover_databases(config: &Config) -> Result<Vec<Database>> {
    if config.databases.discovery.trim().is_empty() {
        bail!("databases.discovery は空にできません");
    }
    if config.databases.discovery.trim() != config.databases.discovery {
        bail!("databases.discovery の前後に空白は使用できません");
    }
    match config.databases.discovery.as_str() {
        "glob" => discover_by_glob(config),
        "query" => discover_by_query(config),
        other => bail!("未対応の discovery です: {other}"),
    }
}

pub fn discover_by_glob(config: &Config) -> Result<Vec<Database>> {
    let pattern = config
        .databases
        .path_glob
        .as_deref()
        .ok_or_else(|| anyhow!("glob discovery には databases.path_glob が必要です"))?;
    if pattern.trim().is_empty() {
        bail!("glob discovery には databases.path_glob が必要です");
    }
    validate_configured_db_path(config, "databases.path_glob", pattern)?;
    let resolved = config.resolve_path(pattern);
    let pattern_text = resolved
        .to_str()
        .ok_or_else(|| anyhow!("path_glob がUTF-8ではありません: {}", resolved.display()))?;

    let mut databases = Vec::new();
    for entry in glob::glob(pattern_text)
        .with_context(|| format!("glob を解析できません: {pattern_text}"))?
    {
        let path = entry.with_context(|| format!("glob の展開に失敗しました: {pattern_text}"))?;
        let metadata = fs::metadata(&path)
            .with_context(|| format!("DBメタデータを取得できません: {}", path.display()))?;
        if !metadata.is_file() {
            bail!(
                "DBパスは通常ファイルである必要があります: {}",
                path.display()
            );
        }
        config.validate_resolved_path_within_base("DBパス", &path)?;
        databases.push(database_from_path(path));
    }
    databases.sort_by(|a, b| a.path.cmp(&b.path));
    validate_database_set(&databases)?;
    Ok(databases)
}

pub fn discover_by_query(config: &Config) -> Result<Vec<Database>> {
    let source = config
        .databases
        .source
        .as_deref()
        .ok_or_else(|| anyhow!("query discovery には databases.source が必要です"))?;
    if source.trim().is_empty() {
        bail!("query discovery には databases.source が必要です");
    }
    validate_configured_db_path(config, "databases.source", source)?;
    let query = config
        .databases
        .query
        .as_deref()
        .ok_or_else(|| anyhow!("query discovery には databases.query が必要です"))?;
    if query.trim().is_empty() {
        bail!("query discovery には databases.query が必要です");
    }
    validate_discovery_query(query)?;
    let id_column = config.databases.id_column.as_deref().unwrap_or("id");
    if id_column.trim().is_empty() {
        bail!("databases.id_column は空にできません");
    }
    if id_column.trim() != id_column {
        bail!("databases.id_column の前後に空白は使用できません");
    }
    if config
        .databases
        .path_column
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("databases.path_column は空にできません");
    }
    if config
        .databases
        .path_column
        .as_deref()
        .is_some_and(|value| value.trim() != value)
    {
        bail!("databases.path_column の前後に空白は使用できません");
    }
    if config
        .databases
        .path_template
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("databases.path_template は空にできません");
    }
    if let Some(template) = config.databases.path_template.as_deref() {
        validate_configured_db_path(config, "databases.path_template", template)?;
        validate_path_template_syntax(template)?;
    }
    if config
        .databases
        .path_column
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
        && config
            .databases
            .path_template
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        bail!(
            "query discovery には databases.path_column または databases.path_template が必要です"
        );
    }
    let source_path = config.resolve_path(source);
    if source_path.exists()
        && !fs::metadata(&source_path)
            .with_context(|| {
                format!(
                    "discovery source のメタデータを取得できません: {}",
                    source_path.display()
                )
            })?
            .is_file()
    {
        bail!(
            "discovery source は通常ファイルである必要があります: {}",
            source_path.display()
        );
    }
    let conn = open_existing_readonly(&source_path)
        .with_context(|| format!("discovery source を開けません: {}", source_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_millis(
        config.execution.lock_timeout_ms,
    ))?;
    let mut stmt = conn
        .prepare(query)
        .context("discovery query を準備できません")?;
    if !stmt.readonly() {
        bail!("discovery query は読み取り専用である必要があります");
    }
    let mut rows = stmt.query([]).context("discovery query を実行できません")?;
    let mut databases = Vec::new();

    while let Some(row) = rows
        .next()
        .context("discovery query の行取得に失敗しました")?
    {
        let id = row_get_required_string(row, id_column)
            .with_context(|| format!("id_column を取得できません: {id_column}"))?;
        let path = if let Some(path_column) = config.databases.path_column.as_deref() {
            let raw_path = row_get_required_string(row, path_column)
                .with_context(|| format!("path_column を取得できません: {path_column}"))?;
            validate_discovered_db_path(config, "path_column", &raw_path)?
        } else if let Some(template) = config.databases.path_template.as_deref() {
            config.resolve_path(render_path_template(template, &id)?)
        } else {
            bail!("query discovery には databases.path_column または databases.path_template が必要です");
        };
        config.validate_resolved_path_within_base("DBパス", &path)?;
        if path.exists()
            && !fs::metadata(&path)
                .with_context(|| format!("DBメタデータを取得できません: {}", path.display()))?
                .is_file()
        {
            bail!(
                "DBパスは通常ファイルである必要があります: {}",
                path.display()
            );
        }

        databases.push(Database {
            id,
            exists: path.exists(),
            readable: fs::File::open(&path).is_ok(),
            path,
        });
    }

    databases.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.path.cmp(&b.path)));
    validate_database_set(&databases)?;
    Ok(databases)
}

pub(crate) fn validate_database_set(databases: &[Database]) -> Result<()> {
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    for database in databases {
        validate_database_id(&database.id)?;
        if !seen_ids.insert(database.id.as_str()) {
            bail!(
                "DB ID が重複しています: {}。id_column またはDBファイル名を一意にしてください",
                database.id
            );
        }
        let normalized_path = normalize_path_for_comparison(&database.path);
        if !seen_paths.insert(normalized_path) {
            bail!(
                "DBパスが重複しています: {}。同じDBを複数IDで処理しない設定にしてください",
                database.path.display()
            );
        }
    }
    Ok(())
}

pub fn render_path_template(template: &str, id: &str) -> Result<String> {
    validate_template_id(id)?;
    validate_path_template_syntax(template)?;
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let end = after_start
            .find('}')
            .ok_or_else(|| anyhow!("path_template の置換式が閉じていません: {template}"))?;
        let expr = &after_start[..end];
        output.push_str(&render_template_expr(expr, id)?);
        rest = &after_start[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

pub(crate) fn validate_path_template_syntax(template: &str) -> Result<()> {
    let mut rest = template;
    let mut has_id_expr = false;
    while let Some(start) = rest.find('{') {
        if rest[..start].contains('}') {
            bail!("path_template の置換式が開いていません: {template}");
        }
        let after_start = &rest[start + 1..];
        let end = after_start
            .find('}')
            .ok_or_else(|| anyhow!("path_template の置換式が閉じていません: {template}"))?;
        let expr = &after_start[..end];
        validate_template_expr(expr)?;
        has_id_expr = true;
        rest = &after_start[end + 1..];
    }
    if rest.contains('}') {
        bail!("path_template の置換式が開いていません: {template}");
    }
    if !has_id_expr {
        bail!("path_template には {{id}} または {{id:幅:split幅}} が必要です: {template}");
    }
    Ok(())
}

fn validate_template_id(id: &str) -> Result<()> {
    if is_invalid_database_id(id) {
        bail!("path_template に埋め込むIDとして不正です: {id}");
    }
    Ok(())
}

pub(crate) fn validate_database_id(id: &str) -> Result<()> {
    if is_invalid_database_id(id) {
        bail!("DB ID として不正です: {id}");
    }
    Ok(())
}

fn is_invalid_database_id(id: &str) -> bool {
    id.trim().is_empty()
        || id.trim() != id
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
}

fn render_template_expr(expr: &str, id: &str) -> Result<String> {
    match parse_template_expr(expr)? {
        TemplateExpr::Id => Ok(id.to_string()),
        TemplateExpr::Split { width, split } => {
            let padded = pad_id(id, width);
            let padded_len = padded.chars().count();
            let first = if padded_len >= split {
                padded.chars().take(split).collect::<String>()
            } else {
                padded.clone()
            };
            let second = if padded_len >= split.saturating_mul(2) {
                padded.chars().skip(split).take(split).collect::<String>()
            } else {
                "0".repeat(split)
            };
            Ok(format!("{first}/{second}/{padded}"))
        }
    }
}

fn validate_template_expr(expr: &str) -> Result<()> {
    parse_template_expr(expr).map(|_| ())
}

enum TemplateExpr {
    Id,
    Split { width: usize, split: usize },
}

fn parse_template_expr(expr: &str) -> Result<TemplateExpr> {
    if expr == "id" {
        return Ok(TemplateExpr::Id);
    }

    let parts: Vec<&str> = expr.split(':').collect();
    if parts.len() == 3 && parts[0] == "id" && parts[2].starts_with("split") {
        if parts[1].is_empty() || !parts[1].chars().all(|ch| ch.is_ascii_digit()) {
            bail!("path_template のゼロ埋め幅が不正です: {expr}");
        }
        let width = parts[1]
            .parse::<usize>()
            .with_context(|| format!("path_template のゼロ埋め幅が不正です: {expr}"))?;
        if width == 0 {
            bail!("path_template のゼロ埋め幅は1以上が必要です: {expr}");
        }
        if width > MAX_TEMPLATE_WIDTH {
            bail!("path_template のゼロ埋め幅は{MAX_TEMPLATE_WIDTH}以下が必要です: {expr}");
        }
        let split_text = parts[2].strip_prefix("split").unwrap_or_default();
        if split_text.is_empty() || !split_text.chars().all(|ch| ch.is_ascii_digit()) {
            bail!("path_template のsplit指定が不正です: {expr}");
        }
        let split = split_text
            .parse::<usize>()
            .with_context(|| format!("path_template のsplit指定が不正です: {expr}"))?;
        if split == 0 {
            bail!("path_template のsplit指定は1以上が必要です: {expr}");
        }
        if split > MAX_TEMPLATE_SPLIT {
            bail!("path_template のsplit指定は{MAX_TEMPLATE_SPLIT}以下が必要です: {expr}");
        }
        if width < split {
            bail!("path_template のゼロ埋め幅はsplit指定以上が必要です: {expr}");
        }
        return Ok(TemplateExpr::Split { width, split });
    }

    bail!("未対応の path_template 置換式です: {{{expr}}}");
}

fn row_get_required_string(row: &Row<'_>, column: &str) -> rusqlite::Result<String> {
    let value = row_get_string(row, column)?;
    if value.trim().is_empty() {
        return Err(rusqlite::Error::InvalidColumnName(format!(
            "{column} is NULL or empty"
        )));
    }
    if value.trim() != value {
        return Err(rusqlite::Error::InvalidColumnName(format!(
            "{column} has surrounding whitespace"
        )));
    }
    Ok(value)
}

fn row_get_string(row: &Row<'_>, column: &str) -> rusqlite::Result<String> {
    match row.get_ref(column)? {
        ValueRef::Null => Ok(String::new()),
        ValueRef::Integer(value) => Ok(value.to_string()),
        ValueRef::Real(value) => Ok(value.to_string()),
        ValueRef::Text(value) => std::str::from_utf8(value)
            .map(str::to_owned)
            .map_err(|_| rusqlite::Error::InvalidColumnName(format!("{column} is not UTF-8"))),
        ValueRef::Blob(_) => Err(rusqlite::Error::InvalidColumnName(format!(
            "{column} is BLOB"
        ))),
    }
}

fn reject_parent_dir_component(label: &str, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        bail!(
            "{label} に親ディレクトリ成分は使用できません: {}",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn validate_configured_db_path(config: &Config, label: &str, path: &str) -> Result<()> {
    if path.trim() != path {
        bail!("{label} の前後に空白は使用できません");
    }
    if path_has_parent_dir(path) && !path_starts_with_parent_dir(path) {
        reject_parent_dir_component(label, path)?;
    }
    config.validate_path_within_base(label, path)?;
    reject_parent_dir_component(label, path)?;
    Ok(())
}

fn validate_discovered_db_path(config: &Config, label: &str, path: &str) -> Result<PathBuf> {
    let resolved = config.resolve_path(path);
    if path_has_parent_dir(path) && !path_starts_with_parent_dir(path) {
        reject_parent_dir_component(label, path)?;
    }
    config.validate_resolved_path_within_base("DBパス", &resolved)?;
    reject_parent_dir_component(label, path)?;
    Ok(resolved)
}

fn path_has_parent_dir(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .components()
        .any(|component| component == Component::ParentDir)
}

fn path_starts_with_parent_dir(path: impl AsRef<Path>) -> bool {
    path.as_ref().components().next() == Some(Component::ParentDir)
}

fn pad_id(id: &str, width: usize) -> String {
    let id_len = id.chars().count();
    if id_len >= width {
        id.to_string()
    } else {
        format!("{}{}", "0".repeat(width - id_len), id)
    }
}

fn database_from_path(path: PathBuf) -> Database {
    let exists = path.exists();
    let readable = fs::File::open(&path).is_ok();
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_string();
    Database {
        id,
        path,
        exists,
        readable,
    }
}
