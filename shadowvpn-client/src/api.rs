use anyhow::Result;
use shadowvpn_core::{PeerInfo, PeersRequest, RegisterRequest, RegisterResponse};

pub async fn register(
    server_url: &str,
    group_name: &str,
    group_password: &str,
    public_key: &str,
    endpoint: &str,
) -> Result<RegisterResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/register", server_url))
        .json(&RegisterRequest {
            group_name: group_name.to_string(),
            group_password: group_password.to_string(),
            public_key: public_key.to_string(),
            endpoint: endpoint.to_string(),
        })
        .send()
        .await?
        .error_for_status()?
        .json::<RegisterResponse>()
        .await?;
    Ok(resp)
}

#[allow(dead_code)]
pub async fn fetch_peers(
    server_url: &str,
    group_name: &str,
    group_password: &str,
) -> Result<Vec<PeerInfo>> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/peers", server_url))
        .json(&PeersRequest {
            group_name: group_name.to_string(),
            group_password: group_password.to_string(),
        })
        .send()
        .await?
        .error_for_status()?
        .json::<Vec<PeerInfo>>()
        .await?;
    Ok(resp)
}
