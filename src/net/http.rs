use anyhow::{Context, Result};
use reqwest::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

/// File metadata obtained from a HEAD request
#[derive(Debug)]
pub struct FileMetadata {
    pub content_length: Option<u64>,
    pub supports_range: bool,
    pub filename: Option<String>,
    pub etag: Option<String>,
    /// The final URL after following redirects
    pub final_url: Option<String>,
}

/// Parse "Key: Value" header strings into a HeaderMap
pub fn parse_headers(headers: &[String], cookie: Option<&str>) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    for h in headers {
        let (key, value) = h.split_once(':').ok_or_else(|| {
            anyhow::anyhow!("invalid header format: '{}' (expected 'Key: Value')", h)
        })?;
        let name = HeaderName::from_bytes(key.trim().as_bytes())
            .with_context(|| format!("invalid header name: '{}'", key.trim()))?;
        let val = HeaderValue::from_str(value.trim())
            .with_context(|| format!("invalid header value: '{}'", value.trim()))?;
        map.append(name, val);
    }
    if let Some(cookie_str) = cookie {
        let val = HeaderValue::from_str(cookie_str).context("invalid cookie value")?;
        map.insert(reqwest::header::COOKIE, val);
    }
    Ok(map)
}

/// Build a configured HTTP client with optional proxy and custom headers
pub fn build_client(
    user_agent: &str,
    timeout_secs: u64,
    proxy: Option<&str>,
    default_headers: Option<HeaderMap>,
) -> Result<Client> {
    let mut builder = Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs(timeout_secs));

    if let Some(proxy_url) = proxy {
        let proxy = reqwest::Proxy::all(proxy_url).context("invalid proxy URL")?;
        builder = builder.proxy(proxy);
    }

    if let Some(headers) = default_headers {
        builder = builder.default_headers(headers);
    }

    builder.build().context("failed to build HTTP client")
}

/// Send a HEAD request to get file metadata
pub async fn fetch_metadata(client: &Client, url: &str) -> Result<FileMetadata> {
    let resp = client
        .head(url)
        .send()
        .await
        .context("HEAD request failed")?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("HEAD request returned status {}", status);
    }

    // Capture the final URL after redirects
    let final_url = {
        let resp_url = resp.url().as_str();
        if resp_url != url {
            Some(resp_url.to_string())
        } else {
            None
        }
    };

    let headers = resp.headers();

    let content_length = headers
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let supports_range = headers
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("bytes"));

    let filename = extract_filename(headers, resp.url().as_str());

    let etag = headers
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    Ok(FileMetadata {
        content_length,
        supports_range,
        filename,
        etag,
        final_url,
    })
}

/// Extract filename from Content-Disposition header or URL path
fn extract_filename(headers: &reqwest::header::HeaderMap, url: &str) -> Option<String> {
    // Try Content-Disposition header first
    if let Some(cd) = headers.get(reqwest::header::CONTENT_DISPOSITION)
        && let Ok(cd_str) = cd.to_str()
        && let Some(name) = parse_content_disposition(cd_str)
    {
        return Some(name);
    }

    // Fall back to URL path
    let parsed = url::Url::parse(url).ok()?;
    let mut segments = parsed.path_segments()?;
    let last = segments.next_back()?;
    if last.is_empty() {
        return None;
    }
    Some(
        urlencoding::decode(last)
            .unwrap_or(last.into())
            .into_owned(),
    )
}

/// Parse filename from Content-Disposition header value
fn parse_content_disposition(value: &str) -> Option<String> {
    // Look for filename*= (RFC 5987) first, then filename=
    for part in value.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename*=") {
            // Format: UTF-8''encoded_name
            if let Some(name) = rest.split("''").nth(1) {
                return Some(
                    urlencoding::decode(name)
                        .unwrap_or(name.into())
                        .into_owned(),
                );
            }
        }
    }
    for part in value.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename=") {
            let name = rest.trim_matches('"');
            return Some(name.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_content_disposition() {
        assert_eq!(
            parse_content_disposition("attachment; filename=\"test.zip\""),
            Some("test.zip".to_string())
        );
        assert_eq!(
            parse_content_disposition("attachment; filename*=UTF-8''%E6%B5%8B%E8%AF%95.zip"),
            Some("测试.zip".to_string())
        );
    }

    #[test]
    fn test_parse_content_disposition_no_filename() {
        assert_eq!(parse_content_disposition("inline"), None);
    }

    #[test]
    fn test_parse_content_disposition_unquoted() {
        assert_eq!(
            parse_content_disposition("attachment; filename=test.zip"),
            Some("test.zip".to_string())
        );
    }

    #[test]
    fn test_parse_headers_basic() {
        let headers = vec![
            "Authorization: Bearer token123".to_string(),
            "Accept: application/json".to_string(),
        ];
        let map = parse_headers(&headers, None).unwrap();
        assert_eq!(map.get("authorization").unwrap(), "Bearer token123");
        assert_eq!(map.get("accept").unwrap(), "application/json");
    }

    #[test]
    fn test_parse_headers_with_cookie() {
        let headers = vec![];
        let map = parse_headers(&headers, Some("sid=abc; token=xyz")).unwrap();
        assert_eq!(map.get("cookie").unwrap(), "sid=abc; token=xyz");
    }

    #[test]
    fn test_parse_headers_cookie_and_custom() {
        let headers = vec!["X-Custom: value".to_string()];
        let map = parse_headers(&headers, Some("session=123")).unwrap();
        assert_eq!(map.get("x-custom").unwrap(), "value");
        assert_eq!(map.get("cookie").unwrap(), "session=123");
    }

    #[test]
    fn test_parse_headers_invalid_format() {
        let headers = vec!["no-colon-here".to_string()];
        assert!(parse_headers(&headers, None).is_err());
    }

    #[test]
    fn test_parse_headers_empty() {
        let map = parse_headers(&[], None).unwrap();
        assert!(map.is_empty());
    }
}
