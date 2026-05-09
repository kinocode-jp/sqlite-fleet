use crate::sql_scan::{
    find_next_keyword_end, next_identifier, next_keyword, next_non_whitespace,
    restricted_statements, skip_balanced_parentheses, skip_block_comment, skip_line_comment,
    skip_quoted,
};

pub(crate) fn history_table_write_statements(sql: &str, table: &str) -> Vec<String> {
    restricted_statements(sql, |keyword, sql, end, statements| {
        collect_history_table_write_statement(keyword, sql, end, table, statements);
    })
}

fn collect_history_table_write_statement(
    keyword: &str,
    sql: &str,
    end: usize,
    table: &str,
    statements: &mut Vec<String>,
) {
    match keyword {
        "WITH" => {
            if let Some((body_keyword, body_end)) = with_body_keyword(sql, end) {
                collect_history_table_write_statement(
                    &body_keyword,
                    sql,
                    body_end,
                    table,
                    statements,
                );
            }
        }
        "INSERT" => {
            let start = skip_optional_conflict_clause(sql, end);
            if next_keyword(sql, start).as_deref() == Some("INTO")
                && next_table_name(sql, find_next_keyword_end(sql, start).unwrap_or(start))
                    .is_some_and(|name| identifier_matches(&name, table))
            {
                statements.push("INSERT".to_string());
            }
        }
        "REPLACE"
            if next_keyword(sql, end).as_deref() == Some("INTO")
                && table_after_keyword_matches(sql, end, table) =>
        {
            statements.push("REPLACE".to_string());
        }
        "UPDATE" => {
            let start = skip_optional_conflict_clause(sql, end);
            if next_table_name(sql, start).is_some_and(|name| identifier_matches(&name, table)) {
                statements.push("UPDATE".to_string());
            }
        }
        "DELETE"
            if next_keyword(sql, end).as_deref() == Some("FROM")
                && table_after_keyword_matches(sql, end, table) =>
        {
            statements.push("DELETE".to_string());
        }
        "DROP" => {
            if let Some(statement) = drop_protected_object_statement(sql, end, table) {
                statements.push(statement);
            }
        }
        "ALTER"
            if next_keyword(sql, end).as_deref() == Some("TABLE")
                && table_after_keyword_matches(sql, end, table) =>
        {
            statements.push("ALTER TABLE".to_string());
        }
        "CREATE" if create_table_start(sql, end).is_some() => {
            let table_start =
                skip_optional_if_not_exists(sql, create_table_start(sql, end).unwrap());
            if next_table_name(sql, table_start)
                .is_some_and(|name| identifier_matches(&name, table))
            {
                statements.push("CREATE TABLE".to_string());
            }
        }
        "CREATE" if create_view_start(sql, end).is_some() => {
            let view_start = skip_optional_if_not_exists(sql, create_view_start(sql, end).unwrap());
            if next_table_name(sql, view_start).is_some_and(|name| identifier_matches(&name, table))
            {
                statements.push("CREATE VIEW".to_string());
            }
        }
        "CREATE"
            if create_index_table_start(sql, end).is_some()
                && next_table_name(sql, create_index_table_start(sql, end).unwrap())
                    .is_some_and(|name| identifier_matches(&name, table)) =>
        {
            statements.push("CREATE INDEX".to_string());
        }
        "CREATE"
            if create_trigger_table_start(sql, end).is_some()
                && next_table_name(sql, create_trigger_table_start(sql, end).unwrap())
                    .is_some_and(|name| identifier_matches(&name, table)) =>
        {
            statements.push("CREATE TRIGGER".to_string());
        }
        _ => {}
    }
}

fn table_after_keyword_matches(sql: &str, keyword_end: usize, table: &str) -> bool {
    next_table_name(
        sql,
        find_next_keyword_end(sql, keyword_end).unwrap_or(keyword_end),
    )
    .is_some_and(|name| identifier_matches(&name, table))
}

fn drop_protected_object_statement(sql: &str, start: usize, table: &str) -> Option<String> {
    let object_type = next_keyword(sql, start)?;
    if !matches!(object_type.as_str(), "TABLE" | "VIEW" | "INDEX" | "TRIGGER") {
        return None;
    }
    let object_start = skip_optional_if_exists(sql, find_next_keyword_end(sql, start)?);
    next_table_name(sql, object_start)
        .is_some_and(|name| identifier_matches(&name, table))
        .then(|| format!("DROP {object_type}"))
}

fn create_table_start(sql: &str, start: usize) -> Option<usize> {
    let first_keyword = next_keyword(sql, start)?;
    let after_first = find_next_keyword_end(sql, start)?;
    match first_keyword.as_str() {
        "TABLE" => Some(after_first),
        "VIRTUAL" if next_keyword(sql, after_first).as_deref() == Some("TABLE") => {
            find_next_keyword_end(sql, after_first)
        }
        "TEMP" | "TEMPORARY" if next_keyword(sql, after_first).as_deref() == Some("TABLE") => {
            find_next_keyword_end(sql, after_first)
        }
        _ => None,
    }
}

fn create_view_start(sql: &str, start: usize) -> Option<usize> {
    let first_keyword = next_keyword(sql, start)?;
    let after_first = find_next_keyword_end(sql, start)?;
    match first_keyword.as_str() {
        "VIEW" => Some(after_first),
        "TEMP" | "TEMPORARY" if next_keyword(sql, after_first).as_deref() == Some("VIEW") => {
            find_next_keyword_end(sql, after_first)
        }
        _ => None,
    }
}

fn create_index_table_start(sql: &str, start: usize) -> Option<usize> {
    let mut index = start;
    if matches!(
        next_keyword(sql, index).as_deref(),
        Some("TEMP" | "TEMPORARY")
    ) {
        index = find_next_keyword_end(sql, index)?;
    }
    if next_keyword(sql, index).as_deref() == Some("UNIQUE") {
        index = find_next_keyword_end(sql, index)?;
    }
    if next_keyword(sql, index).as_deref() != Some("INDEX") {
        return None;
    }
    index = find_next_keyword_end(sql, index)?;
    index = skip_optional_if_not_exists(sql, index);
    index = skip_qualified_identifier(sql, index)?;
    if next_keyword(sql, index).as_deref() == Some("ON") {
        find_next_keyword_end(sql, index)
    } else {
        None
    }
}

fn create_trigger_table_start(sql: &str, start: usize) -> Option<usize> {
    let mut index = start;
    if matches!(
        next_keyword(sql, index).as_deref(),
        Some("TEMP" | "TEMPORARY")
    ) {
        index = find_next_keyword_end(sql, index)?;
    }
    if next_keyword(sql, index).as_deref() != Some("TRIGGER") {
        return None;
    }
    index = find_next_keyword_end(sql, index)?;
    index = skip_optional_if_not_exists(sql, index);
    index = skip_qualified_identifier(sql, index)?;
    find_statement_keyword_end_before_body(sql, index, "ON")
}

fn with_body_keyword(sql: &str, start: usize) -> Option<(String, usize)> {
    let mut index = start;
    if next_keyword(sql, index).as_deref() == Some("RECURSIVE") {
        index = find_next_keyword_end(sql, index)?;
    }
    loop {
        let cte_name = next_identifier(sql, index)?;
        index = cte_name.end;
        if next_non_whitespace(sql, index)?.byte == b'(' {
            index = skip_balanced_parentheses(sql, next_non_whitespace(sql, index)?.end - 1)?;
        }
        if next_keyword(sql, index).as_deref() != Some("AS") {
            return None;
        }
        index = find_next_keyword_end(sql, index)?;
        index = skip_optional_materialization_hint(sql, index)?;
        let body_open = next_non_whitespace(sql, index)?;
        if body_open.byte != b'(' {
            return None;
        }
        index = skip_balanced_parentheses(sql, body_open.end - 1)?;
        match next_non_whitespace(sql, index) {
            Some(byte) if byte.byte == b',' => index = byte.end,
            _ => break,
        }
    }
    let keyword = next_keyword(sql, index)?;
    let end = find_next_keyword_end(sql, index)?;
    Some((keyword, end))
}

fn skip_optional_materialization_hint(sql: &str, start: usize) -> Option<usize> {
    match next_keyword(sql, start).as_deref() {
        Some("MATERIALIZED") => find_next_keyword_end(sql, start),
        Some("NOT") => {
            let after_not = find_next_keyword_end(sql, start)?;
            if next_keyword(sql, after_not).as_deref() == Some("MATERIALIZED") {
                find_next_keyword_end(sql, after_not)
            } else {
                Some(start)
            }
        }
        _ => Some(start),
    }
}

fn skip_optional_conflict_clause(sql: &str, start: usize) -> usize {
    if next_keyword(sql, start).as_deref() != Some("OR") {
        return start;
    }
    let after_or = find_next_keyword_end(sql, start).unwrap_or(start);
    match next_keyword(sql, after_or).as_deref() {
        Some("ROLLBACK" | "ABORT" | "REPLACE" | "FAIL" | "IGNORE") => {
            find_next_keyword_end(sql, after_or).unwrap_or(after_or)
        }
        _ => start,
    }
}

fn skip_optional_if_exists(sql: &str, start: usize) -> usize {
    if next_keyword(sql, start).as_deref() != Some("IF") {
        return start;
    }
    let after_if = find_next_keyword_end(sql, start).unwrap_or(start);
    if next_keyword(sql, after_if).as_deref() == Some("EXISTS") {
        find_next_keyword_end(sql, after_if).unwrap_or(after_if)
    } else {
        start
    }
}

fn skip_optional_if_not_exists(sql: &str, start: usize) -> usize {
    if next_keyword(sql, start).as_deref() != Some("IF") {
        return start;
    }
    let after_if = find_next_keyword_end(sql, start).unwrap_or(start);
    if next_keyword(sql, after_if).as_deref() != Some("NOT") {
        return start;
    }
    let after_not = find_next_keyword_end(sql, after_if).unwrap_or(after_if);
    if next_keyword(sql, after_not).as_deref() == Some("EXISTS") {
        find_next_keyword_end(sql, after_not).unwrap_or(after_not)
    } else {
        start
    }
}

fn next_table_name(sql: &str, start: usize) -> Option<String> {
    let first = next_identifier(sql, start)?;
    let after_first = first.end;
    match next_non_whitespace(sql, after_first) {
        Some(dot) if dot.byte == b'.' => {
            next_identifier(sql, dot.end).map(|identifier| identifier.text)
        }
        _ => Some(first.text),
    }
}

fn skip_qualified_identifier(sql: &str, start: usize) -> Option<usize> {
    let first = next_identifier(sql, start)?;
    match next_non_whitespace(sql, first.end) {
        Some(dot) if dot.byte == b'.' => {
            next_identifier(sql, dot.end).map(|identifier| identifier.end)
        }
        _ => Some(first.end),
    }
}

fn find_statement_keyword_end_before_body(sql: &str, start: usize, target: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' | b'[' => index = skip_quoted(bytes, index),
            b'-' if bytes.get(index + 1) == Some(&b'-') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            b';' => return None,
            byte if byte.is_ascii_alphabetic() => {
                let keyword_start = index;
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    index += 1;
                }
                let keyword = sql[keyword_start..index].to_ascii_uppercase();
                if keyword == target {
                    return Some(index);
                }
                if keyword == "BEGIN" {
                    return None;
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn identifier_matches(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}
