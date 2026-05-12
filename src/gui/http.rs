fn write_json_result<T: Serialize>(stream: &mut TcpStream, result: Result<T>) -> Result<()> {
    match result {
        Ok(data) => write_json(
            stream,
            200,
            &ApiEnvelope {
                ok: true,
                data: Some(data),
                error: None,
            },
        ),
        Err(error) => write_json(
            stream,
            200,
            &ApiEnvelope::<()> {
                ok: false,
                data: None,
                error: Some(error.to_string()),
            },
        ),
    }
}

fn utf8_byte_len(value: &str) -> usize {
    value.len()
}

fn write_json<T: Serialize>(stream: &mut TcpStream, status: u16, value: &T) -> Result<()> {
    let body = serde_json::to_string(value)?;
    write_response(
        stream,
        status,
        "application/json; charset=utf-8",
        &body,
        None,
        None,
    )
}

fn write_json_error(stream: &mut TcpStream, status: u16, error: anyhow::Error) -> Result<()> {
    write_json(
        stream,
        status,
        &ApiEnvelope::<()> {
            ok: false,
            data: None,
            error: Some(error.to_string()),
        },
    )
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
    script_nonce: Option<&str>,
    style_nonce: Option<&str>,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "OK",
    };
    let script_src = script_nonce
        .map(|nonce| format!("'nonce-{nonce}'"))
        .unwrap_or_else(|| "'none'".to_string());
    let style_src = style_nonce
        .map(|nonce| format!("'nonce-{nonce}'"))
        .unwrap_or_else(|| "'none'".to_string());
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src {script_src}; style-src {style_src}; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    Ok(())
}

fn split_target(target: &str) -> (&str, &str) {
    target.split_once('?').unwrap_or((target, ""))
}

fn is_api_path(path: &str) -> bool {
    path == "/api" || path.starts_with("/api/")
}

fn requires_json_body(path: &str) -> bool {
    path == "/api/sql" || path.starts_with("/api/admin/")
}

fn target_path_contains_percent_encoding(target: &str) -> bool {
    split_target(target).0.contains('%')
}

fn parse_headers(request: &str) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();
    for line in request.lines().skip(1).take_while(|line| !line.is_empty()) {
        if line.starts_with(' ') || line.starts_with('\t') {
            bail!("HTTP header の折り返し形式は許可されません");
        }
        let Some((key, value)) = line.split_once(':') else {
            bail!("HTTP header の形式が不正です");
        };
        if key.trim() != key {
            bail!("HTTP header 名の前後に空白は使用できません");
        }
        let key = key.to_ascii_lowercase();
        if key.is_empty() {
            bail!("HTTP header 名が空です");
        }
        if !is_valid_header_name(&key) {
            bail!("HTTP header 名が不正です: {key}");
        }
        let value = value.trim();
        if value.bytes().any(|byte| byte.is_ascii_control()) {
            bail!("HTTP header 値に制御文字は指定できません: {key}");
        }
        if matches!(
            key.as_str(),
            "host"
                | "x-sqlite-fleet-token"
                | "content-length"
                | "content-type"
                | "transfer-encoding"
        ) && headers.contains_key(&key)
        {
            bail!("重複したHTTP header は許可されません: {key}");
        }
        headers.insert(key, value.to_string());
    }
    Ok(headers)
}

fn is_valid_header_name(name: &str) -> bool {
    name.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'-'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
            )
    })
}

fn validate_api_token(headers: &HashMap<String, String>, expected: &str) -> Result<()> {
    match headers.get("x-sqlite-fleet-token") {
        Some(actual) if constant_time_eq(actual, expected) => Ok(()),
        _ => bail!("GUI API token が不正です。画面を更新して再実行してください"),
    }
}

fn constant_time_eq(actual: &str, expected: &str) -> bool {
    let actual = actual.as_bytes();
    let expected = expected.as_bytes();
    let mut diff = actual.len() ^ expected.len();
    for (index, expected_byte) in expected.iter().enumerate() {
        let actual_byte = actual.get(index).copied().unwrap_or(0);
        diff |= usize::from(actual_byte ^ expected_byte);
    }
    diff == 0
}

fn validate_no_request_body(
    headers: &HashMap<String, String>,
    has_initial_body: bool,
) -> Result<()> {
    if headers.contains_key("transfer-encoding") {
        bail!("GUI API request body は使用できません");
    }
    if has_initial_body {
        bail!("GUI API request body は使用できません");
    }
    match headers.get("content-length").map(String::as_str) {
        Some("0") | None => Ok(()),
        Some(_) => bail!("GUI API request body は使用できません"),
    }
}

fn validate_json_content_type(headers: &HashMap<String, String>) -> Result<()> {
    let Some(content_type) = headers.get("content-type") else {
        bail!("Content-Type は application/json が必要です");
    };
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if media_type == "application/json" {
        Ok(())
    } else {
        bail!("Content-Type は application/json が必要です");
    }
}

fn read_request_body(
    stream: &mut TcpStream,
    headers: &HashMap<String, String>,
    mut body: Vec<u8>,
) -> Result<Vec<u8>> {
    if headers.contains_key("transfer-encoding") {
        bail!("Transfer-Encoding は使用できません");
    }
    let Some(length) = headers.get("content-length") else {
        bail!("Content-Length が必要です");
    };
    let length = parse_content_length(length)?;
    if length > MAX_HTTP_BODY_BYTES {
        bail!("HTTP request body が大きすぎます");
    }
    if body.len() > length {
        bail!("HTTP request body がContent-Lengthを超えています");
    }
    while body.len() < length {
        let remaining = length - body.len();
        let mut buffer = vec![0_u8; remaining.min(8192)];
        let size = stream.read(&mut buffer)?;
        if size == 0 {
            bail!("HTTP request body が完了していません");
        }
        body.extend_from_slice(&buffer[..size]);
    }
    Ok(body)
}

fn parse_content_length(value: &str) -> Result<usize> {
    if value.is_empty() {
        bail!("Content-Length が空です");
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("Content-Length はASCII数字だけ指定できます: {value}");
    }
    if value.len() > 1 && value.starts_with('0') {
        bail!("Content-Length に先頭ゼロは使用できません: {value}");
    }
    value
        .parse()
        .with_context(|| format!("Content-Length が不正です: {value}"))
}

fn validate_host_header(
    headers: &HashMap<String, String>,
    bind_ip: IpAddr,
    port: u16,
) -> Result<()> {
    let Some(host) = headers.get("host") else {
        bail!("Host header が必要です");
    };
    let (hostname, header_port) = parse_host_header(host)?;
    match header_port {
        Some(header_port) if header_port == port => {}
        Some(_) => bail!("Host header のportが不正です"),
        None if port == 80 => {}
        None => bail!("Host header にはGUI serverのportが必要です"),
    }
    if is_localhost_alias(&hostname) && is_default_loopback(bind_ip) {
        return Ok(());
    }
    match hostname.parse::<IpAddr>() {
        Ok(ip) if ip.is_loopback() && ip == bind_ip => Ok(()),
        Ok(ip) if ip.is_loopback() => {
            bail!("Host header のIPがGUI serverのbind addressと一致しません")
        }
        _ => bail!("Host header はループバックホストのみ許可されます"),
    }
}

fn is_localhost_alias(hostname: &str) -> bool {
    matches!(hostname, "localhost" | "localhost.")
}

fn is_default_loopback(ip: IpAddr) -> bool {
    matches!(
        ip,
        IpAddr::V4(ip) if ip == Ipv4Addr::LOCALHOST
    ) || matches!(
        ip,
        IpAddr::V6(ip) if ip == Ipv6Addr::LOCALHOST
    )
}

fn parse_host_header(host: &str) -> Result<(String, Option<u16>)> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        bail!("Host header が空です");
    }
    if let Some(rest) = host.strip_prefix('[') {
        let Some((hostname, suffix)) = rest.split_once(']') else {
            bail!("Host header のIPv6形式が不正です");
        };
        if hostname.is_empty() {
            bail!("Host header のhostnameが空です");
        }
        if hostname.parse::<Ipv6Addr>().is_err() {
            bail!("Host header のIPv6 literalが不正です");
        }
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(parse_host_port(port)?)
        } else {
            bail!("Host header のIPv6形式が不正です");
        };
        return Ok((hostname.to_string(), port));
    }

    if host.matches(':').count() == 1 {
        let Some((hostname, port)) = host.rsplit_once(':') else {
            bail!("Host header の形式が不正です");
        };
        if hostname.is_empty() {
            bail!("Host header のhostnameが空です");
        }
        Ok((hostname.to_string(), Some(parse_host_port(port)?)))
    } else {
        if host.contains(':') {
            bail!("Host header のIPv6 literalには角括弧が必要です");
        }
        if host.is_empty() {
            bail!("Host header のhostnameが空です");
        }
        Ok((host.to_string(), None))
    }
}

fn parse_host_port(port: &str) -> Result<u16> {
    if port.is_empty() {
        bail!("Host header のportが空です");
    }
    if !port.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("Host header のportはASCII数字だけ指定できます: {port}");
    }
    if port.len() > 1 && port.starts_with('0') {
        bail!("Host header のportに先頭ゼロは使用できません: {port}");
    }
    port.parse()
        .with_context(|| format!("Host header のportが不正です: {port}"))
}

fn parse_query(query: &str) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    if query.is_empty() {
        return Ok(values);
    }
    for part in query.split('&') {
        if part.is_empty() {
            bail!("query parameter が空です");
        }
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        let key = percent_decode(key)?;
        let value = percent_decode(value)?;
        if key.is_empty() {
            bail!("query parameter 名が空です");
        }
        if key.bytes().any(|byte| byte.is_ascii_control())
            || value.bytes().any(|byte| byte.is_ascii_control())
        {
            bail!("query parameter に制御文字は指定できません");
        }
        if values.insert(key.clone(), value).is_some() {
            bail!("重複したquery parameter は許可されません: {key}");
        }
    }
    Ok(values)
}

fn required_bool_query(query: &HashMap<String, String>, name: &str) -> Result<bool> {
    match query.get(name).map(String::as_str) {
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(value) => bail!("query parameter {name} はtrueまたはfalseが必要です: {value}"),
        None => bail!("query parameter {name} が必要です"),
    }
}

fn optional_nonempty_query<'a>(
    query: &'a HashMap<String, String>,
    name: &str,
) -> Result<Option<&'a str>> {
    match query.get(name).map(String::as_str) {
        Some("") => bail!("query parameter {name} は空にできません"),
        Some(value) => Ok(Some(value)),
        None => Ok(None),
    }
}

fn optional_usize_query(query: &HashMap<String, String>, name: &str) -> Result<Option<usize>> {
    match query.get(name).map(String::as_str) {
        Some("") => bail!("query parameter {name} は空にできません"),
        Some(value) => {
            let limit = value
                .parse::<usize>()
                .with_context(|| format!("query parameter {name} は正の整数が必要です"))?;
            if limit == 0 {
                bail!("query parameter {name} は1以上が必要です");
            }
            Ok(Some(limit))
        }
        None => Ok(None),
    }
}

fn validate_query_keys(query: &HashMap<String, String>, allowed: &[&str]) -> Result<()> {
    for key in query.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!("未知のquery parameterです: {key}");
        }
    }
    Ok(())
}

fn validate_no_query(query: &str) -> Result<()> {
    if query.is_empty() {
        Ok(())
    } else {
        bail!("このAPI endpointにquery parameterは指定できません");
    }
}

fn percent_decode(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .context("query parameter のpercent encodingが不正です")?;
                decoded.push(
                    u8::from_str_radix(hex, 16)
                        .context("query parameter のpercent encodingが不正です")?,
                );
                i += 3;
            }
            b'%' => bail!("query parameter のpercent encodingが不正です"),
            byte => {
                decoded.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(decoded).context("query parameter はUTF-8である必要があります")
}

fn validate_gui_host(host: &str) -> Result<()> {
    let addrs = (host, 0)
        .to_socket_addrs()
        .with_context(|| format!("GUI host を解決できません: {host}"))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        bail!("GUI host を解決できません: {host}");
    }
    if addrs.iter().all(|addr| addr.ip().is_loopback()) {
        Ok(())
    } else {
        bail!("GUI host はループバックアドレスのみ指定できます: {host}");
    }
}

fn generate_csrf_token() -> Result<String> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random).context("GUI API token を生成できません")?;
    Ok(hex_encode(random))
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
