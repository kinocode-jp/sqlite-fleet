use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sqlite_fleet::{
    backup, check, discover_databases, load_migrations, migrate_with_options, status_report,
    write_audit_event, write_report_json, Config, DatabaseSelection, MigrateOptions,
    MigrationGroupConfig, ALL_DB_GROUP, MAIN_MIGRATION_GROUP,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_SQL_BYTES: usize = 2 * 1024 * 1024;
const MAX_HTTP_BODY_BYTES: usize = MAX_SQL_BYTES + 4 * 1024;

include!("gui/server.rs");
include!("gui/request.rs");
include!("gui/api.rs");
include!("gui/sql_runner.rs");
include!("gui/admin_types.rs");
include!("gui/http.rs");

const INDEX_HTML: &str = concat!(
    include_str!("gui/index.00.html"),
    include_str!("gui/index.01.html"),
    include_str!("gui/index.02.html"),
);

#[cfg(test)]
mod tests {
    use super::*;

    include!("gui/tests_core.rs");
    include!("gui/tests_http_sql.rs");
    include!("gui/tests_sql_schema.rs");
    include!("gui/tests_html.rs");
}
