fn parse_request_line(line: &str) -> Result<(&str, &str)> {
    if line.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("request line に制御文字は指定できません");
    }
    let mut parts = line.split(' ');
    let Some(method) = parts.next() else {
        bail!("request line にmethodが必要です");
    };
    let Some(target) = parts.next() else {
        bail!("request line にtargetが必要です");
    };
    let Some(version) = parts.next() else {
        bail!("request line にHTTP versionが必要です");
    };
    if parts.next().is_some() {
        bail!("request line の要素が多すぎます");
    }
    if !matches!(method, "GET" | "POST") {
        bail!("HTTP method はGETまたはPOSTだけ許可されます");
    }
    if !target.starts_with('/') {
        bail!("request target はabsolute pathである必要があります");
    }
    if target.starts_with("//") {
        bail!("request target はorigin-form pathである必要があります");
    }
    if target.contains('#') {
        bail!("request target にfragmentは指定できません");
    }
    if target_path_contains_percent_encoding(target) {
        bail!("request target のpathにpercent encodingは指定できません");
    }
    if target.contains('\\') {
        bail!("request target にambiguous path separatorは指定できません");
    }
    if target.bytes().any(|byte| byte.is_ascii_control()) {
        bail!("request target に制御文字は指定できません");
    }
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        bail!("HTTP version はHTTP/1.0またはHTTP/1.1だけ許可されます");
    }
    Ok((method, target))
}

fn read_http_request(stream: &mut TcpStream) -> Result<Option<HttpRequest>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let size = stream.read(&mut buffer)?;
        if size == 0 {
            if request.is_empty() {
                return Ok(None);
            }
            bail!("HTTP request header が完了していません");
        }
        request.extend_from_slice(&buffer[..size]);
        if let Some(header_end) = http_header_end(&request) {
            if header_end > MAX_HTTP_HEADER_BYTES {
                bail!("HTTP request header が大きすぎます");
            }
            let head = String::from_utf8(request[..header_end].to_vec())
                .context("HTTP request header はUTF-8である必要があります")
                .map(Some)?;
            return Ok(head.map(|head| HttpRequest {
                head,
                initial_body: request[header_end..].to_vec(),
            }));
        }
        if request.len() > MAX_HTTP_HEADER_BYTES {
            bail!("HTTP request header が大きすぎます");
        }
    }
}

fn http_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn validate_crlf_lines(request: &str) -> Result<()> {
    let bytes = request.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        match *byte {
            b'\n' if index == 0 || bytes[index - 1] != b'\r' => {
                bail!("HTTP header の行区切りはCRLFである必要があります");
            }
            b'\r' if bytes.get(index + 1) != Some(&b'\n') => {
                bail!("HTTP header の行区切りはCRLFである必要があります");
            }
            _ => {}
        }
    }
    Ok(())
}

