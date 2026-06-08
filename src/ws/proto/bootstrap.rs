use serde::Deserialize;
use super::error::WsError;

const GEN_ENDPOINT_URI: &str = "/callback/ws/endpoint";

#[derive(Debug, Deserialize)]
pub struct EndpointResp {
    pub code: i32,
    pub msg: Option<String>,
    pub data: Option<Endpoint>,
}

#[derive(Debug, Deserialize)]
pub struct Endpoint {
    #[serde(rename = "URL")]
    pub url: String,
    #[serde(rename = "ClientConfig")]
    pub client_config: Option<ClientConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientConfig {
    #[serde(rename = "ReconnectCount")]
    pub reconnect_count: i32,
    #[serde(rename = "ReconnectInterval")]
    pub reconnect_interval: i32,
    #[serde(rename = "ReconnectNonce")]
    pub reconnect_nonce: i32,
    #[serde(rename = "PingInterval")]
    pub ping_interval: i32,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            reconnect_count: -1,
            reconnect_interval: 120,
            reconnect_nonce: 30,
            ping_interval: 120,
        }
    }
}

pub struct BootstrapResult {
    pub ws_url: String,
    pub service_id: i32,
    pub config: ClientConfig,
}

/// POST /callback/ws/endpoint to get WebSocket URL and config.
pub async fn bootstrap(
    domain: &str,
    app_id: &str,
    app_secret: &str,
) -> Result<BootstrapResult, WsError> {
    let url = format!("{}{}", domain.trim_end_matches('/'), GEN_ENDPOINT_URI);

    let body = serde_json::json!({
        "AppID": app_id,
        "AppSecret": app_secret,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| WsError::Bootstrap(format!("HTTP request failed: {}", e)))?;

    let status = resp.status();
    if !status.is_success() {
        let msg = resp.text().await.unwrap_or_default();
        return Err(WsError::Bootstrap(format!("HTTP {}: {}", status.as_u16(), msg)));
    }

    let endpoint_resp: EndpointResp = resp
        .json()
        .await
        .map_err(|e| WsError::Bootstrap(format!("JSON parse: {}", e)))?;

    match endpoint_resp.code {
        super::headers::ERR_OK => {}
        super::headers::ERR_SYSTEM_BUSY | super::headers::ERR_INTERNAL => {
            return Err(WsError::Bootstrap(format!(
                "server error {}: {}",
                endpoint_resp.code,
                endpoint_resp.msg.unwrap_or_default()
            )));
        }
        other => {
            return Err(WsError::Bootstrap(format!(
                "client error {}: {}",
                other,
                endpoint_resp.msg.unwrap_or_default()
            )));
        }
    }

    let endpoint = endpoint_resp
        .data
        .ok_or_else(|| WsError::Bootstrap("no endpoint data".into()))?;

    if endpoint.url.is_empty() {
        return Err(WsError::Bootstrap("empty URL".into()));
    }

    let service_id = endpoint
        .url
        .split("service_id=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let config = endpoint.client_config.unwrap_or_default();

    Ok(BootstrapResult {
        ws_url: endpoint.url,
        service_id,
        config,
    })
}
