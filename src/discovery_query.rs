use anyhow::{anyhow, bail, Result};

pub(crate) fn validate_discovery_query(query: &str) -> Result<()> {
    let (keyword, keyword_end) = first_sql_keyword(query)?;
    match keyword.as_str() {
        "SELECT" => {
            validate_single_sql_statement(query)?;
            Ok(())
        }
        "WITH" => {
            validate_with_query_body_is_select(query, keyword_end)?;
            validate_single_sql_statement(query)?;
            Ok(())
        }
        _ => bail!("discovery query は SELECT または WITH で始まる必要があります: {keyword}"),
    }
}

fn validate_single_sql_statement(sql: &str) -> Result<()> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => skip_quoted_sql(bytes, &mut index, b'\'')?,
            b'"' => skip_quoted_sql(bytes, &mut index, b'"')?,
            b'`' => skip_quoted_sql(bytes, &mut index, b'`')?,
            b'[' => skip_bracket_quoted_sql(bytes, &mut index)?,
            b'-' if bytes.get(index + 1) == Some(&b'-') => skip_line_comment(bytes, &mut index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => skip_block_comment(bytes, &mut index)?,
            b';' => {
                index += 1;
                let rest = skip_sql_trivia(bytes, index)?;
                if rest < bytes.len() {
                    bail!("discovery query は1文のみ指定できます");
                }
                return Ok(());
            }
            _ => index += 1,
        }
    }
    Ok(())
}

fn first_sql_keyword(sql: &str) -> Result<(String, usize)> {
    let bytes = sql.as_bytes();
    let index = skip_sql_trivia(bytes, 0)?;
    read_sql_keyword(sql, index)
        .ok_or_else(|| anyhow!("discovery query は SELECT または WITH で始まる必要があります"))
}

fn validate_with_query_body_is_select(sql: &str, mut index: usize) -> Result<()> {
    let bytes = sql.as_bytes();
    index = skip_sql_trivia(bytes, index)?;
    if let Some((keyword, end)) = read_sql_keyword(sql, index) {
        if keyword == "RECURSIVE" {
            index = skip_sql_trivia(bytes, end)?;
        }
    }

    loop {
        skip_cte_name(sql, &mut index)?;
        index = skip_sql_trivia(bytes, index)?;
        if bytes.get(index) == Some(&b'(') {
            skip_balanced_parentheses(bytes, &mut index)?;
            index = skip_sql_trivia(bytes, index)?;
        }
        let Some((as_keyword, after_as)) = read_sql_keyword(sql, index) else {
            bail!("WITH discovery query のCTE定義が不正です");
        };
        if as_keyword != "AS" {
            bail!("WITH discovery query のCTE定義には AS が必要です");
        }
        index = skip_cte_materialization_hint(sql, after_as)?;
        if bytes.get(index) != Some(&b'(') {
            bail!("WITH discovery query のCTE本体には括弧が必要です");
        }
        skip_balanced_parentheses(bytes, &mut index)?;
        index = skip_sql_trivia(bytes, index)?;
        if bytes.get(index) == Some(&b',') {
            index += 1;
            index = skip_sql_trivia(bytes, index)?;
            continue;
        }
        let Some((body_keyword, _)) = read_sql_keyword(sql, index) else {
            bail!("WITH discovery query の本体が見つかりません");
        };
        if body_keyword != "SELECT" {
            bail!("WITH discovery query の本体は SELECT である必要があります: {body_keyword}");
        }
        return Ok(());
    }
}

fn skip_cte_materialization_hint(sql: &str, mut index: usize) -> Result<usize> {
    let bytes = sql.as_bytes();
    index = skip_sql_trivia(bytes, index)?;
    let Some((keyword, end)) = read_sql_keyword(sql, index) else {
        return Ok(index);
    };
    match keyword.as_str() {
        "MATERIALIZED" => skip_sql_trivia(bytes, end),
        "NOT" => {
            let materialized_start = skip_sql_trivia(bytes, end)?;
            let Some((materialized, materialized_end)) = read_sql_keyword(sql, materialized_start)
            else {
                bail!("WITH discovery query の materialization hint が不正です");
            };
            if materialized != "MATERIALIZED" {
                bail!(
                    "WITH discovery query の materialization hint が不正です: NOT {materialized}"
                );
            }
            skip_sql_trivia(bytes, materialized_end)
        }
        _ => Ok(index),
    }
}

fn read_sql_keyword(sql: &str, mut index: usize) -> Option<(String, usize)> {
    let bytes = sql.as_bytes();
    if !bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphabetic())
    {
        return None;
    }
    let start = index;
    index += 1;
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        index += 1;
    }
    Some((sql[start..index].to_ascii_uppercase(), index))
}

fn skip_sql_trivia(bytes: &[u8], mut index: usize) -> Result<usize> {
    while index < bytes.len() {
        match bytes[index] {
            b'\xEF' if is_utf8_bom_at(bytes, index) => index += 3,
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => skip_line_comment(bytes, &mut index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => skip_block_comment(bytes, &mut index)?,
            _ => return Ok(index),
        }
    }
    Ok(index)
}

fn is_utf8_bom_at(bytes: &[u8], index: usize) -> bool {
    bytes.get(index..index + 3) == Some(&[0xEF, 0xBB, 0xBF])
}

fn skip_cte_name(sql: &str, index: &mut usize) -> Result<()> {
    let bytes = sql.as_bytes();
    *index = skip_sql_trivia(bytes, *index)?;
    match bytes.get(*index).copied() {
        Some(b'"') => skip_quoted_sql(bytes, index, b'"'),
        Some(b'`') => skip_quoted_sql(bytes, index, b'`'),
        Some(b'[') => skip_bracket_quoted_sql(bytes, index),
        Some(byte) if byte.is_ascii_alphabetic() || byte == b'_' => {
            *index += 1;
            while bytes
                .get(*index)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                *index += 1;
            }
            Ok(())
        }
        _ => bail!("WITH discovery query のCTE名が不正です"),
    }
}

fn skip_balanced_parentheses(bytes: &[u8], index: &mut usize) -> Result<()> {
    if bytes.get(*index) != Some(&b'(') {
        bail!("discovery query の括弧が不正です");
    }
    *index += 1;
    let mut depth = 1usize;
    while *index < bytes.len() {
        match bytes[*index] {
            b'\'' => skip_quoted_sql(bytes, index, b'\'')?,
            b'"' => skip_quoted_sql(bytes, index, b'"')?,
            b'`' => skip_quoted_sql(bytes, index, b'`')?,
            b'[' => skip_bracket_quoted_sql(bytes, index)?,
            b'-' if bytes.get(*index + 1) == Some(&b'-') => skip_line_comment(bytes, index),
            b'/' if bytes.get(*index + 1) == Some(&b'*') => skip_block_comment(bytes, index)?,
            b'(' => {
                depth += 1;
                *index += 1;
            }
            b')' => {
                depth -= 1;
                *index += 1;
                if depth == 0 {
                    return Ok(());
                }
            }
            _ => *index += 1,
        }
    }
    bail!("discovery query の括弧が閉じていません");
}

fn skip_line_comment(bytes: &[u8], index: &mut usize) {
    *index += 2;
    while *index < bytes.len() && bytes[*index] != b'\n' {
        *index += 1;
    }
}

fn skip_block_comment(bytes: &[u8], index: &mut usize) -> Result<()> {
    *index += 2;
    while *index + 1 < bytes.len() {
        if bytes[*index] == b'*' && bytes[*index + 1] == b'/' {
            *index += 2;
            return Ok(());
        }
        *index += 1;
    }
    bail!("discovery query のコメントが閉じていません");
}

fn skip_quoted_sql(bytes: &[u8], index: &mut usize, quote: u8) -> Result<()> {
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
    bail!("discovery query のクォートが閉じていません");
}

fn skip_bracket_quoted_sql(bytes: &[u8], index: &mut usize) -> Result<()> {
    *index += 1;
    while *index < bytes.len() {
        if bytes[*index] == b']' {
            *index += 1;
            return Ok(());
        }
        *index += 1;
    }
    bail!("discovery query のクォートが閉じていません");
}
