pub fn init_project(config_path: impl AsRef<Path>) -> Result<()> {
    let config_path = config_path.as_ref();
    let base_dir = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(base_dir).with_context(|| {
        format!(
            "設定ファイルの親ディレクトリを作成できません: {}",
            base_dir.display()
        )
    })?;

    if !config_path.exists() {
        fs::write(config_path, default_config_template())
            .with_context(|| format!("設定ファイルを作成できません: {}", config_path.display()))?;
    }

    fs::create_dir_all(base_dir.join("migrations"))
        .context("migrations ディレクトリを作成できません")?;
    Ok(())
}

pub fn default_config_template() -> &'static str {
    r#"[project]
name = "sqlite-fleet-project"

[databases]
discovery = "glob"
path_glob = "./data/**/*.db"

[migrations]
dir = "./migrations"
table = "_sqlite_fleet_migrations"

# Uncomment after creating matching files in migrations/.
# [migration_groups]
# main = ["001", "002"]

# [db_groups]
# canary = ["tenant-a"]

# [database_migration_groups]
# tenant-a = ["main"]

[execution]
parallel = 4
lock_timeout_ms = 5000
continue_on_error = false

[report]
format = "json"
path = "./sqlite-fleet-report.json"

[backup]
dir = "./backups"
before_migrate = false
keep_last = 10

[audit]
path = "./sqlite-fleet-audit.jsonl"

[gui]
allow_check = true
allow_migrate = true
allow_backup = true
allow_restore = true
allow_sql_apply = true
allow_migration_edit = true
"#
}

fn is_blank(value: Option<&str>) -> bool {
    value.is_none_or(|value| value.trim().is_empty())
}

fn validate_group_name(label: &str, name: &str) -> Result<()> {
    if name.trim().is_empty() || name.trim() != name || name.chars().any(char::is_whitespace) {
        bail!("{label} の名前は空白なしの非空文字列である必要があります");
    }
    if name.chars().any(char::is_control) {
        bail!("{label} の名前に制御文字は使用できません: {name}");
    }
    if name.contains('/') || name.contains('\\') {
        bail!("{label} の名前にパス区切り文字は使用できません: {name}");
    }
    if name == "." || name == ".." {
        bail!("{label} の名前に特殊なパス成分は使用できません: {name}");
    }
    Ok(())
}

fn database_matches_config_selector(config: &Config, database: &Database, selector: &str) -> bool {
    if database.id == selector {
        return true;
    }
    let selector_path = config.resolve_path(selector);
    normalize_path_for_comparison(&selector_path) == normalize_path_for_comparison(&database.path)
}

fn default_discovery() -> String {
    "glob".to_string()
}

fn default_migrations_dir() -> String {
    "./migrations".to_string()
}

fn default_migrations_table() -> String {
    DEFAULT_MIGRATIONS_TABLE.to_string()
}

fn default_parallel() -> usize {
    4
}

fn default_lock_timeout_ms() -> u64 {
    5000
}

fn default_report_format() -> String {
    "json".to_string()
}

fn default_backup_dir() -> String {
    "./backups".to_string()
}

fn default_backup_keep_last() -> usize {
    10
}

fn default_true() -> bool {
    true
}
