use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str = "achievement-hunter/0.1 (+https://github.com/Zsweezzy/achievement-hunter)";

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
    let response = client()
        .get(url)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    response
        .text()
        .map_err(|e| format!("body read failed: {e}"))
}

pub fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let text = get_text(url)?;
    serde_json::from_str(&text).map_err(|e| format!("invalid json: {e}"))
}

pub fn get_json_with_headers(
    url: &str,
    headers: &[(&str, &str)],
) -> Result<serde_json::Value, String> {
    let mut request = client().get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().map_err(|e| format!("request failed: {e}"))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|e| format!("body read failed: {e}"))?;
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
    let response = request.send().map_err(|e| format!("request failed: {e}"))?;
    let status = response.status();
    let text = response
        .text()
        .map_err(|e| format!("body read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("invalid json: {e}"))
}

pub fn download(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    let response = client()
        .get(url)
        .send()
        .map_err(|e| format!("request failed: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("http {status}"));
    }
    let bytes = response
        .bytes()
        .map_err(|e| format!("body read failed: {e}"))?;
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
