use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use rusqlite::{backup::StepResult, params, Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
pub struct DatabaseSelection {
    pub database: Option<String>,
    pub group: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct MigrateOptions {
    pub dry_run: bool,
    pub selection: DatabaseSelection,
    pub backup_before_migrate: Option<bool>,
}

mod config;
mod diagnostics;
mod discovery;
mod discovery_query;
mod history;
mod migration;
mod migration_sql;
mod migration_sql_guard;
mod path_utils;
mod sql_scan;
mod sqlite_ident;
pub use config::*;
pub use diagnostics::{
    doctor, doctor_and_write_report, doctor_config, doctor_with_overrides, write_report_json,
};
pub use discovery::{
    discover_by_glob, discover_by_query, discover_databases, render_path_template,
};
use discovery::{validate_database_id, validate_database_set};
pub use history::read_applied_migrations_with_catalog;
use history::{create_migrations_table_sql, migrate_legacy_migrations_table};
pub use history::{ensure_migrations_table, read_applied_migrations};
use migration::validate_migrations;
pub use migration::{
    checksum_sql, load_migrations, parse_migration_file, parse_migration_file_name,
    validate_migration,
};
use path_utils::normalize_path_for_comparison;
use sqlite_ident::validate_identifier;

include!("core/plan.rs");
include!("core/migrate.rs");
include!("core/backup_restore.rs");
include!("core/drift_check_audit.rs");
include!("core/runtime.rs");
