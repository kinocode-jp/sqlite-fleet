use anyhow::{bail, Result};
use rusqlite::ffi::sqlite3_keyword_check;
use std::os::raw::c_char;

pub(crate) fn validate_identifier(identifier: &str) -> Result<()> {
    if identifier.is_empty()
        || identifier
            .chars()
            .next()
            .is_some_and(|ch| ch != '_' && !ch.is_ascii_alphabetic())
        || !identifier
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        bail!("SQLite識別子として不正です: {identifier}");
    }
    if identifier.len() >= "sqlite_".len()
        && identifier[.."sqlite_".len()].eq_ignore_ascii_case("sqlite_")
    {
        bail!("SQLite内部予約名は識別子として使用できません: {identifier}");
    }
    let keyword_len = i32::try_from(identifier.len())?;
    let is_keyword =
        unsafe { sqlite3_keyword_check(identifier.as_ptr().cast::<c_char>(), keyword_len) != 0 };
    if is_keyword {
        bail!("SQLite予約語は識別子として使用できません: {identifier}");
    }
    Ok(())
}
