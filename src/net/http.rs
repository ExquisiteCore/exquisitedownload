use anyhow::{Context, Result};
use reqwest::Client;

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

/// Build a configured HTTP client with optional proxy
pub fn build_client(user_agent: &str, timeout_secs: u64, proxy: Option<&str>) -> Result<Client> {
    let mut builder = Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs(timeout_secs));

    if let Some(proxy_url) = proxy {
        let proxy = reqwest::Proxy::all(proxy_url)
            .context("invalid proxy URL")?;
        builder = builder.proxy(proxy);
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
fn extract_filename(
    headers: &reqwest::header::HeaderMap,
    url: &str,
) -> Option<String> {
    // Try Content-Disposition header first
    if let Some(cd) = headers.get(reqwest::header::CONTENT_DISPOSITION) {
        if let Ok(cd_str) = cd.to_str() {
            if let Some(name) = parse_content_disposition(cd_str) {
                return Some(name);
            }
        }
    }

    // Fall back to URL path
    let parsed = url::Url::parse(url).ok()?;
    let segments = parsed.path_segments()?;
    let last = segments.last()?;
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
}
