#[cfg(unix)]
use sqlite_fleet::{check_database, Config, Database};
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use tempfile::tempdir;

#[cfg(unix)]
#[test]
fn check_database_reports_wal_size_for_non_utf8_database_path() {
    let dir = tempdir().unwrap();
    let db_path = dir
        .path()
        .join(OsString::from_vec(b"tenant-\xFF.db".to_vec()));
    if fs::write(&db_path, b"not sqlite").is_err() {
        return;
    }

    let mut wal_path = db_path.as_os_str().to_os_string();
    wal_path.push("-wal");
    if fs::write(PathBuf::from(wal_path), b"waldata").is_err() {
        return;
    }

    let config = Config {
        base_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let result = check_database(
        &config,
        &Database {
            id: "tenant".to_string(),
            path: db_path,
            exists: true,
            readable: true,
        },
        &[],
    );

    assert!(!result.success);
    assert_eq!(result.wal_bytes, Some(7));
}
