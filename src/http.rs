use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str = "achievement-hunter/0.1 (+https://github.com/Zsweezzy/achievement-hunter)";
const MAX_BODY: u64 = 16 * 1024 * 1024;

fn request_failed(error: reqwest::Error) -> String {
    format!("request failed: {}", error.without_url())
}

fn read_body(response: reqwest::blocking::Response) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    response
        .take(MAX_BODY + 1)
        .read_to_end(&mut buffer)
        .map_err(|e| format!("body read failed: {e}"))?;
    if buffer.len() as u64 > MAX_BODY {
        return Err(format!("response exceeds {MAX_BODY} bytes"));
    }
    Ok(buffer)
}

fn read_text(response: reqwest::blocking::Response) -> Result<String, String> {
    let bytes = read_body(response)?;
    String::from_utf8(bytes).map_err(|e| format!("body not utf-8: {e}"))
}

static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

pub fn client() -> &'static reqwest::blocking::Client {
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(25))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

pub fn encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub fn get_text(url: &str) -> Result<String, String> {
    let response = client().get(url).send().map_err(request_failed)?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    read_text(response)
}

pub fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let response = client().get(url).send().map_err(request_failed)?;
    let status = response.status();
    let text = read_text(response)?;
    if status.is_success() {
        return serde_json::from_str(&text).map_err(|e| format!("invalid json: {e}"));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(ps) = v.get("playerstats")
            && let Some(err) = ps.get("error").and_then(|x| x.as_str())
        {
            return Err(err.to_owned());
        }
        if let Some(ps) = v.get("playerstats")
            && let Some(msg) = ps.get("message").and_then(|x| x.as_str())
        {
            return Err(msg.to_owned());
        }
        if let Some(resp) = v.get("response") {
            if let Some(err) = resp.get("error") {
                if let Some(m) = err.get("message").and_then(|x| x.as_str()) {
                    return Err(m.to_owned());
                }
                if let Some(m) = err.as_str() {
                    return Err(m.to_owned());
                }
            }
            if let Some(m) = resp.get("message").and_then(|x| x.as_str()) {
                return Err(m.to_owned());
            }
            if let Some(m) = resp.get("errorMsg").and_then(|x| x.as_str()) {
                return Err(m.to_owned());
            }
        }
        if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
            return Err(err.to_owned());
        }
        if let Some(err) = v.get("error_description").and_then(|x| x.as_str()) {
            return Err(err.to_owned());
        }
        if let Some(msg) = v.get("message").and_then(|x| x.as_str()) {
            return Err(msg.to_owned());
        }
    }
    Err(format!("http {status}"))
}

pub fn get_json_with_headers(
    url: &str,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    let mut request = client().get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().map_err(request_failed)?;
    let status = response.status();
    let text = read_text(response)?;
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("invalid json: {e}"))
}

pub fn post_json(
    url: &str,
    body: &serde_json::Value,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    let mut request = client().post(url).json(body);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().map_err(request_failed)?;
    let status = response.status();
    let text = read_text(response)?;
    if status.is_success() {
        return serde_json::from_str(&text).map_err(|e| format!("invalid json: {e}"));
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(ps) = v.get("playerstats")
            && let Some(err) = ps.get("error").and_then(|x| x.as_str())
        {
            return Err(err.to_owned());
        }
        if let Some(resp) = v.get("response") {
            if let Some(err) = resp.get("error") {
                if let Some(m) = err.get("message").and_then(|x| x.as_str()) {
                    return Err(m.to_owned());
                }
                if let Some(m) = err.as_str() {
                    return Err(m.to_owned());
                }
            }
            if let Some(m) = resp.get("message").and_then(|x| x.as_str()) {
                return Err(m.to_owned());
            }
        }
        if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
            return Err(err.to_owned());
        }
        if let Some(msg) = v.get("message").and_then(|x| x.as_str()) {
            return Err(msg.to_owned());
        }
    }
    Err(format!("http {status}"))
}

pub fn download(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    let response = client().get(url).send().map_err(request_failed)?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    let bytes = read_body(response)?;
    if bytes.is_empty() {
        return Err("empty body".to_owned());
    }
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, &bytes).map_err(|e| format!("write failed: {e}"))?;
    std::fs::rename(&tmp, dest).map_err(|e| format!("rename failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crucial_query_encoding_escapes_reserved_chars() {
        assert_eq!(encode_query("Portal 2"), "Portal+2");
        assert_eq!(encode_query("a&b=c"), "a%26b%3Dc");
        assert_eq!(encode_query("Tom Clancy's"), "Tom+Clancy%27s");
    }

    #[test]
    fn crucial_query_encoding_keeps_unreserved() {
        assert_eq!(encode_query("abcXYZ019-_.~"), "abcXYZ019-_.~");
    }
}
