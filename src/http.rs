use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str = "achievement-hunter/0.1 (+https://github.com/Zsweezzy/ZSW-AchievementHunter)";
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

fn error_from_body(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let pick = |node: &serde_json::Value, keys: &[&str]| {
        keys.iter()
            .find_map(|key| node.get(*key).and_then(|v| v.as_str()).map(str::to_owned))
    };
    if let Some(playerstats) = value.get("playerstats")
        && let Some(message) = pick(playerstats, &["error", "message"])
    {
        return Some(message);
    }
    if let Some(response) = value.get("response") {
        if let Some(error) = response.get("error")
            && let Some(message) = error
                .as_str()
                .map(str::to_owned)
                .or_else(|| pick(error, &["message"]))
        {
            return Some(message);
        }
        if let Some(message) = pick(response, &["message", "errorMsg"]) {
            return Some(message);
        }
    }
    pick(&value, &["error", "error_description", "message"])
}

fn decode_json(status: reqwest::StatusCode, text: String) -> Result<serde_json::Value, String> {
    if status.is_success() {
        return serde_json::from_str(&text).map_err(|e| format!("invalid json: {e}"));
    }
    Err(error_from_body(&text).unwrap_or_else(|| format!("http {status}")))
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
    decode_json(status, text)
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
    decode_json(status, text)
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
    decode_json(status, text)
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

    #[test]
    fn crucial_error_from_body_reads_steam_and_generic_shapes() {
        assert_eq!(
            error_from_body(
                r#"{"playerstats":{"success":false,"error":"Requested app has no stats"}}"#
            )
            .as_deref(),
            Some("Requested app has no stats")
        );
        assert_eq!(
            error_from_body(r#"{"response":{"error":{"message":"rate limited"}}}"#).as_deref(),
            Some("rate limited")
        );
        assert_eq!(
            error_from_body(r#"{"error_description":"bad key"}"#).as_deref(),
            Some("bad key")
        );
        assert_eq!(error_from_body("<html>400</html>"), None);
        assert_eq!(
            error_from_body(r#"{"playerstats":{"success":false}}"#),
            None
        );
    }

    #[test]
    fn crucial_decode_json_uses_body_error_then_falls_back_to_status() {
        let bad = reqwest::StatusCode::BAD_REQUEST;
        assert_eq!(
            decode_json(bad, r#"{"playerstats":{"error":"no stats"}}"#.to_owned()).unwrap_err(),
            "no stats"
        );
        assert_eq!(
            decode_json(bad, "<html>".to_owned()).unwrap_err(),
            "http 400 Bad Request"
        );
        assert_eq!(
            decode_json(reqwest::StatusCode::OK, r#"{"ok":true}"#.to_owned()).unwrap()["ok"],
            true
        );
        assert!(
            decode_json(reqwest::StatusCode::OK, "not json".to_owned())
                .unwrap_err()
                .starts_with("invalid json:")
        );
    }
}
