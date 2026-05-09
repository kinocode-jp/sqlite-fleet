pub(crate) fn restricted_statements<F>(sql: &str, mut collect_statement: F) -> Vec<String>
where
    F: FnMut(&str, &str, usize, &mut Vec<String>),
{
    let bytes = sql.as_bytes();
    let mut statements = Vec::new();
    let mut index = 0;
    let mut statement_start = true;
    let mut pending_trigger_body = false;
    let mut in_trigger_body = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\xEF' if is_utf8_bom_at(bytes, index) => index += 3,
            b'\'' | b'"' | b'`' | b'[' => index = skip_quoted(bytes, index),
            b'-' if bytes.get(index + 1) == Some(&b'-') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            b';' => {
                if pending_trigger_body && !in_trigger_body {
                    pending_trigger_body = false;
                }
                statement_start = true;
                index += 1;
            }
            byte if byte.is_ascii_alphabetic() => {
                let start = index;
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    index += 1;
                }
                let keyword = sql[start..index].to_ascii_uppercase();
                if pending_trigger_body && keyword == "BEGIN" {
                    in_trigger_body = true;
                    pending_trigger_body = false;
                    statement_start = true;
                    continue;
                }
                if statement_start {
                    if in_trigger_body && keyword == "END" {
                        if next_keyword(sql, index).as_deref() == Some("TRANSACTION") {
                            statements.push("END TRANSACTION".to_string());
                        } else {
                            in_trigger_body = false;
                        }
                    } else {
                        collect_statement(&keyword, sql, index, &mut statements);
                    }
                    if keyword == "CREATE" && is_create_trigger_statement(sql, index) {
                        pending_trigger_body = true;
                    }
                }
                statement_start = false;
            }
            byte if byte.is_ascii_whitespace() => {
                index += 1;
            }
            _ => {
                statement_start = false;
                index += 1;
            }
        }
    }
    statements
}

pub(crate) fn find_next_keyword_end(sql: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\xEF' if is_utf8_bom_at(bytes, index) => index += 3,
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            byte if byte.is_ascii_alphabetic() => {
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    index += 1;
                }
                return Some(index);
            }
            _ => return None,
        }
    }
    None
}

pub(crate) struct SqlIdentifier {
    pub(crate) text: String,
    pub(crate) end: usize,
}

pub(crate) struct SqlByte {
    pub(crate) byte: u8,
    pub(crate) end: usize,
}

pub(crate) fn next_identifier(sql: &str, start: usize) -> Option<SqlIdentifier> {
    let bytes = sql.as_bytes();
    let mut index = skip_sql_space_and_comments(bytes, start);
    match *bytes.get(index)? {
        b'"' | b'\'' | b'`' | b'[' => {
            let opener = bytes[index];
            let closer = if opener == b'[' { b']' } else { opener };
            index += 1;
            let mut text = String::new();
            while index < bytes.len() {
                if bytes[index] == closer {
                    if closer != b']' && bytes.get(index + 1) == Some(&closer) {
                        text.push(closer as char);
                        index += 2;
                    } else {
                        return Some(SqlIdentifier {
                            text,
                            end: index + 1,
                        });
                    }
                } else {
                    text.push(bytes[index] as char);
                    index += 1;
                }
            }
            None
        }
        byte if byte == b'_' || byte.is_ascii_alphabetic() => {
            let start = index;
            index += 1;
            while bytes
                .get(index)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                index += 1;
            }
            Some(SqlIdentifier {
                text: sql[start..index].to_string(),
                end: index,
            })
        }
        _ => None,
    }
}

pub(crate) fn next_non_whitespace(sql: &str, start: usize) -> Option<SqlByte> {
    let bytes = sql.as_bytes();
    let index = skip_sql_space_and_comments(bytes, start);
    bytes.get(index).map(|byte| SqlByte {
        byte: *byte,
        end: index + 1,
    })
}

pub(crate) fn skip_sql_space_and_comments(bytes: &[u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\xEF' if is_utf8_bom_at(bytes, index) => index += 3,
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            _ => return index,
        }
    }
    index
}

pub(crate) fn next_keyword(sql: &str, start: usize) -> Option<String> {
    let bytes = sql.as_bytes();
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\xEF' if is_utf8_bom_at(bytes, index) => index += 3,
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            byte if byte.is_ascii_alphabetic() => {
                let keyword_start = index;
                index += 1;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    index += 1;
                }
                return Some(sql[keyword_start..index].to_ascii_uppercase());
            }
            _ => return None,
        }
    }
    None
}

pub(crate) fn skip_balanced_parentheses(sql: &str, start: usize) -> Option<usize> {
    let bytes = sql.as_bytes();
    if bytes.get(start) != Some(&b'(') {
        return None;
    }
    let mut depth = 1usize;
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' | b'"' | b'`' | b'[' => index = skip_quoted(bytes, index),
            b'-' if bytes.get(index + 1) == Some(&b'-') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => index = skip_block_comment(bytes, index),
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index += 1,
        }
    }
    None
}

pub(crate) fn skip_quoted(bytes: &[u8], start: usize) -> usize {
    let opener = bytes[start];
    let closer = if opener == b'[' { b']' } else { opener };
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == closer {
            if closer != b']' && bytes.get(index + 1) == Some(&closer) {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    bytes.len()
}

pub(crate) fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset + 1)
}

pub(crate) fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

fn is_utf8_bom_at(bytes: &[u8], index: usize) -> bool {
    bytes.get(index..index + 3) == Some(&[0xEF, 0xBB, 0xBF])
}

fn is_create_trigger_statement(sql: &str, start: usize) -> bool {
    match next_keyword(sql, start).as_deref() {
        Some("TRIGGER") => true,
        Some("TEMP" | "TEMPORARY") => {
            let Some(temp_start) = find_next_keyword_end(sql, start) else {
                return false;
            };
            next_keyword(sql, temp_start).as_deref() == Some("TRIGGER")
        }
        _ => false,
    }
}
