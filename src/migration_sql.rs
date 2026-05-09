use anyhow::{bail, Result};

use crate::migration_sql_guard::history_table_write_statements;
use crate::sql_scan::{
    next_identifier, next_keyword, next_non_whitespace, restricted_statements,
    skip_sql_space_and_comments,
};

pub(crate) fn validate_migration_sql(sql: &str) -> Result<()> {
    validate_closed_sql_tokens(sql)?;
    if let Some(keyword) = transaction_control_statements(sql).into_iter().next() {
        bail!("migration SQL にtransaction制御文は使用できません: {keyword}");
    }
    if let Some(keyword) = attachment_statements(sql).into_iter().next() {
        bail!("migration SQL に外部DB接続文は使用できません: {keyword}");
    }
    if let Some(keyword) = non_transactional_statements(sql).into_iter().next() {
        bail!("migration SQL に非transaction文は使用できません: {keyword}");
    }
    if let Some(pragma) = dangerous_pragma_statements(sql).into_iter().next() {
        bail!("migration SQL に危険PRAGMAは使用できません: {pragma}");
    }
    Ok(())
}

fn validate_closed_sql_tokens(sql: &str) -> Result<()> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => skip_closed_quoted_sql(bytes, &mut index, b'\'')?,
            b'"' => skip_closed_quoted_sql(bytes, &mut index, b'"')?,
            b'`' => skip_closed_quoted_sql(bytes, &mut index, b'`')?,
            b'[' => skip_closed_bracket_quoted_sql(bytes, &mut index)?,
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                skip_closed_line_comment(bytes, &mut index)
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                skip_closed_block_comment(bytes, &mut index)?
            }
            _ => index += 1,
        }
    }
    Ok(())
}

fn skip_closed_quoted_sql(bytes: &[u8], index: &mut usize, quote: u8) -> Result<()> {
    *index += 1;
    while *index < bytes.len() {
        if bytes[*index] == quote {
            if bytes.get(*index + 1) == Some(&quote) {
                *index += 2;
            } else {
                *index += 1;
                return Ok(());
            }
        } else {
            *index += 1;
        }
    }
    bail!("migration SQL のクォートが閉じていません");
}

fn skip_closed_bracket_quoted_sql(bytes: &[u8], index: &mut usize) -> Result<()> {
    *index += 1;
    while *index < bytes.len() {
        if bytes[*index] == b']' {
            *index += 1;
            return Ok(());
        }
        *index += 1;
    }
    bail!("migration SQL のクォートが閉じていません");
}

fn skip_closed_line_comment(bytes: &[u8], index: &mut usize) {
    *index += 2;
    while *index < bytes.len() && bytes[*index] != b'\n' {
        *index += 1;
    }
}

fn skip_closed_block_comment(bytes: &[u8], index: &mut usize) -> Result<()> {
    *index += 2;
    while *index + 1 < bytes.len() {
        if bytes[*index] == b'*' && bytes[*index + 1] == b'/' {
            *index += 2;
            return Ok(());
        }
        *index += 1;
    }
    bail!("migration SQL のコメントが閉じていません");
}

pub(crate) fn validate_migration_sql_for_history_table(sql: &str, table: &str) -> Result<()> {
    if let Some(statement) = history_table_write_statements(sql, table)
        .into_iter()
        .next()
    {
        bail!("migration SQL は履歴テーブルを直接変更できません: {statement}");
    }
    Ok(())
}

fn non_transactional_statements(sql: &str) -> Vec<String> {
    restricted_statements(sql, collect_non_transactional_statement)
}

fn attachment_statements(sql: &str) -> Vec<String> {
    restricted_statements(sql, collect_attachment_statement)
}

fn transaction_control_statements(sql: &str) -> Vec<String> {
    restricted_statements(sql, collect_transaction_control_statement)
}

fn dangerous_pragma_statements(sql: &str) -> Vec<String> {
    restricted_statements(sql, collect_dangerous_pragma_statement)
}

fn collect_attachment_statement(keyword: &str, _: &str, _: usize, statements: &mut Vec<String>) {
    if matches!(keyword, "ATTACH" | "DETACH") {
        statements.push(keyword.to_string());
    }
}

fn collect_non_transactional_statement(
    keyword: &str,
    _: &str,
    _: usize,
    statements: &mut Vec<String>,
) {
    if keyword == "VACUUM" {
        statements.push(keyword.to_string());
    }
}

fn collect_transaction_control_statement(
    keyword: &str,
    sql: &str,
    end: usize,
    statements: &mut Vec<String>,
) {
    if matches!(keyword, "COMMIT" | "ROLLBACK" | "SAVEPOINT" | "RELEASE") {
        statements.push(keyword.to_string());
        return;
    }
    if keyword == "BEGIN" {
        let next = next_keyword(sql, end);
        if next.as_deref().is_none_or(|next| {
            matches!(next, "TRANSACTION" | "DEFERRED" | "IMMEDIATE" | "EXCLUSIVE")
        }) {
            statements.push(keyword.to_string());
        }
        return;
    }
    if keyword == "END" {
        match next_keyword(sql, end).as_deref() {
            Some("TRANSACTION") => statements.push("END TRANSACTION".to_string()),
            None => statements.push("END".to_string()),
            _ => {}
        }
    }
}

fn collect_dangerous_pragma_statement(
    keyword: &str,
    sql: &str,
    end: usize,
    statements: &mut Vec<String>,
) {
    if keyword != "PRAGMA" {
        return;
    }
    let Some(name) = pragma_name(sql, end) else {
        return;
    };
    if name.eq_ignore_ascii_case("writable_schema") {
        statements.push("writable_schema".to_string());
        return;
    }
    if name.eq_ignore_ascii_case("journal_mode")
        && pragma_assigned_value(sql, end).is_some_and(|value| is_journal_mode_off_value(&value))
    {
        statements.push("journal_mode=OFF".to_string());
    }
}

fn pragma_name(sql: &str, start: usize) -> Option<String> {
    let first = next_identifier(sql, start)?;
    match next_non_whitespace(sql, first.end) {
        Some(dot) if dot.byte == b'.' => {
            next_identifier(sql, dot.end).map(|identifier| identifier.text)
        }
        _ => Some(first.text),
    }
}

fn pragma_assigned_value(sql: &str, start: usize) -> Option<String> {
    let name = next_identifier(sql, start)?;
    let mut index = name.end;
    if next_non_whitespace(sql, index).is_some_and(|byte| byte.byte == b'.') {
        let dot = next_non_whitespace(sql, index)?;
        index = next_identifier(sql, dot.end)?.end;
    }
    let next = next_non_whitespace(sql, index)?;
    match next.byte {
        b'=' => next_pragma_value(sql, next.end),
        b'(' => next_pragma_value(sql, next.end),
        _ => None,
    }
}

fn next_pragma_value(sql: &str, start: usize) -> Option<String> {
    if let Some(identifier) = next_identifier(sql, start) {
        return Some(identifier.text);
    }
    let bytes = sql.as_bytes();
    let mut index = skip_sql_space_and_comments(bytes, start);
    let value_start = index;
    if bytes
        .get(index)
        .is_some_and(|byte| matches!(byte, b'+' | b'-'))
    {
        index += 1;
    }
    while bytes.get(index).is_some_and(|byte| byte.is_ascii_digit()) {
        index += 1;
    }
    if index > value_start {
        Some(sql[value_start..index].to_string())
    } else {
        None
    }
}

fn is_journal_mode_off_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("OFF") || is_numeric_zero(value)
}

fn is_numeric_zero(value: &str) -> bool {
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte == b'0')
}
