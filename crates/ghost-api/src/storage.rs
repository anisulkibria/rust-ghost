//! Native object storage. Uploads retain a local original for resizing and backup.
//! AWS Signature V4 is used directly; S3-compatible endpoints need no sidecar.
use crate::AppState;
use ghost_core::{GhostError, Result};
use sha2::{Digest, Sha256};
fn hmac(key: &[u8], message: &str) -> Vec<u8> {
    let mut key = key.to_vec();
    if key.len() > 64 {
        key = Sha256::digest(key).to_vec()
    }
    key.resize(64, 0);
    let mut inner = Sha256::new();
    inner.update(key.iter().map(|b| b ^ 0x36).collect::<Vec<_>>());
    inner.update(message.as_bytes());
    let mut outer = Sha256::new();
    outer.update(key.iter().map(|b| b ^ 0x5c).collect::<Vec<_>>());
    outer.update(inner.finalize());
    outer.finalize().to_vec()
}
fn signature(secret: &str, date: &str, region: &str, string_to_sign: &str) -> String {
    let date_key = hmac(format!("AWS4{secret}").as_bytes(), date);
    let region_key = hmac(&date_key, region);
    let service_key = hmac(&region_key, "s3");
    let signing_key = hmac(&service_key, "aws4_request");
    hex::encode(hmac(&signing_key, string_to_sign))
}
/// Contract for native storage adapters; implementations return the public asset URL.
#[allow(async_fn_in_trait)]
pub trait StorageAdapter {
    async fn publish(&self, key: &str, content_type: &str, data: &[u8]) -> Result<String>;
}
pub struct S3Storage {
    endpoint: reqwest::Url,
    bucket: String,
    region: String,
    access: String,
    secret: String,
    session_token: Option<String>,
    asset_host: Option<String>,
}
impl S3Storage {
    pub fn from_config(config: &serde_json::Value) -> Result<Self> {
        let read = |key: &str| config[key].as_str().unwrap_or("").trim().to_string();
        let region = match read("region") {
            v if !v.is_empty() => v,
            _ => "us-east-1".into(),
        };
        let endpoint = read("endpoint");
        let default_endpoint = format!("https://s3.{region}.amazonaws.com");
        let endpoint = reqwest::Url::parse(if endpoint.is_empty() {
            &default_endpoint
        } else {
            &endpoint
        })
        .map_err(|e| GhostError::ConfigError(e.to_string()))?;
        let bucket = read("bucket");
        let access = read("accessKeyId");
        let secret = read("secretAccessKey");
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.query().is_some()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || bucket.is_empty()
            || bucket.contains('/')
            || access.is_empty()
            || secret.is_empty()
        {
            return Err(GhostError::ConfigError(
                "S3 needs bucket, region, accessKeyId and secretAccessKey".into(),
            ));
        }
        let asset_host = config["assetHost"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if asset_host.as_ref().is_some_and(|url| {
            !reqwest::Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
        }) {
            return Err(GhostError::ConfigError("Invalid S3 assetHost".into()));
        }
        Ok(Self {
            endpoint,
            bucket,
            region,
            access,
            secret,
            session_token: config["sessionToken"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            asset_host,
        })
    }
}
impl StorageAdapter for S3Storage {
    async fn publish(&self, key: &str, content_type: &str, data: &[u8]) -> Result<String> {
        let mut url = self.endpoint.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| GhostError::ConfigError("Invalid S3 endpoint".into()))?;
            path.pop_if_empty().push(&self.bucket);
            for segment in key.split('/').filter(|s| !s.is_empty()) {
                path.push(segment);
            }
        }
        let now = chrono::Utc::now();
        let date = now.format("%Y%m%d").to_string();
        let timestamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let payload = hex::encode(Sha256::digest(data));
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or("")),
            None => url.host_str().unwrap_or("").into(),
        };
        let mut headers = std::collections::BTreeMap::from([
            ("host", host),
            ("x-amz-content-sha256", payload.clone()),
            ("x-amz-date", timestamp.clone()),
        ]);
        if let Some(token) = &self.session_token {
            headers.insert("x-amz-security-token", token.clone());
        }
        let signed_headers = headers.keys().copied().collect::<Vec<_>>().join(";");
        let canonical_headers = headers
            .iter()
            .map(|(key, value)| format!("{key}:{}\n", value.trim()))
            .collect::<String>();
        let canonical = format!(
            "PUT\n{}\n\n{canonical_headers}\n{signed_headers}\n{payload}",
            url.path()
        );
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
            hex::encode(Sha256::digest(canonical.as_bytes()))
        );
        let signature = signature(&self.secret, &date, &self.region, &string_to_sign);
        let authorization=format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",self.access);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| GhostError::BadRequest(e.to_string()))?;
        let mut request = client
            .put(url.clone())
            .header("authorization", authorization)
            .header("content-type", content_type)
            .header("x-amz-content-sha256", payload)
            .header("x-amz-date", timestamp);
        if let Some(token) = &self.session_token {
            request = request.header("x-amz-security-token", token)
        }
        let response =
            request.body(data.to_vec()).send().await.map_err(|_| {
                GhostError::BadRequest("Object storage could not be reached".into())
            })?;
        if !response.status().is_success() {
            return Err(GhostError::BadRequest(format!(
                "Object storage rejected the upload ({})",
                response.status()
            )));
        }
        Ok(if let Some(host) = &self.asset_host {
            format!("{}/{}", host.trim_end_matches('/'), key)
        } else {
            url.to_string()
        })
    }
}
pub async fn publish_upload(
    state: &AppState,
    key: &str,
    content_type: &str,
    data: &[u8],
    local_url: String,
) -> Result<String> {
    let storage = &state.config.storage;
    match storage["active"].as_str().unwrap_or("local") {
        "local" | "LocalFileStorage" => Ok(local_url),
        "s3" | "S3" => {
            S3Storage::from_config(&storage["s3"])?
                .publish(key, content_type, data)
                .await
        }
        _ => Err(GhostError::ConfigError(
            "Unknown native storage adapter".into(),
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hmac_matches_rfc4231_vector() {
        assert_eq!(
            hex::encode(hmac(&[0x0b; 20], "Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
    #[test]
    fn invalid_storage_settings_are_rejected() {
        assert!(S3Storage::from_config(&serde_json::json!({"bucket":"a"})).is_err());
        assert!(S3Storage::from_config(
            &serde_json::json!({"bucket":"a","accessKeyId":"fixture","secretAccessKey":"fixture"})
        )
        .is_ok());
    }
}
