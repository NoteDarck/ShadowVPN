use anyhow::{bail, Context, Result};
use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use shadowvpn_core::{
    CreateNetworkRequest, CreateNetworkResponse, ErrorResponse, PeersRequest, PeersResponse,
    RegisterRequest, RegisterResponse, UnregisterRequest,
};
use std::time::Duration;

#[derive(Clone)]
pub struct Api {
    client: Client,
    base: String,
}

impl Api {
    pub fn new(server: &str) -> Result<Self> {
        let base = server.trim().trim_end_matches('/').to_string();
        let url = reqwest::Url::parse(&base)
            .with_context(|| format!("URL do servidor inválida: {server}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("o servidor deve usar http:// ou https://");
        }

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("ShadowVPN/", env!("CARGO_PKG_VERSION")))
            .build()
            .context("não foi possível criar o cliente HTTP")?;

        Ok(Self { client, base })
    }

    pub async fn create_network(
        &self,
        group_name: &str,
        group_password: &str,
    ) -> Result<CreateNetworkResponse> {
        self.post_json(
            "/networks",
            &CreateNetworkRequest {
                group_name: group_name.to_string(),
                group_password: group_password.to_string(),
            },
        )
        .await
        .context("falha ao criar rede")
    }

    pub async fn register(
        &self,
        group_name: &str,
        group_password: &str,
        public_key: &str,
        hostname: &str,
    ) -> Result<RegisterResponse> {
        self.post_json(
            "/register",
            &RegisterRequest {
                group_name: group_name.to_string(),
                group_password: group_password.to_string(),
                public_key: public_key.to_string(),
                hostname: hostname.to_string(),
            },
        )
        .await
        .context("falha ao registrar no grupo")
    }

    pub async fn heartbeat(
        &self,
        group_name: &str,
        group_password: &str,
        session_id: u64,
    ) -> Result<PeersResponse> {
        self.post_json(
            "/peers",
            &PeersRequest {
                group_name: group_name.to_string(),
                group_password: group_password.to_string(),
                session_id,
            },
        )
        .await
        .context("falha ao atualizar lista de peers")
    }

    pub async fn unregister(
        &self,
        group_name: &str,
        group_password: &str,
        session_id: u64,
    ) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/unregister", self.base))
            .json(&UnregisterRequest {
                group_name: group_name.to_string(),
                group_password: group_password.to_string(),
                session_id,
            })
            .send()
            .await
            .context("falha ao enviar desconexão")?;

        let status = response.status();
        if !status.is_success() && status != StatusCode::NOT_FOUND {
            let body = response.text().await.unwrap_or_default();
            bail!("servidor respondeu {status}: {body}");
        }

        Ok(())
    }

    async fn post_json<B, T>(&self, path: &str, body: &B) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let response = self
            .client
            .post(format!("{}{}", self.base, path))
            .json(body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ErrorResponse>(&body)
                .map(|error| error.error)
                .unwrap_or(body);
            bail!("servidor respondeu {status}: {message}");
        }

        response
            .json::<T>()
            .await
            .context("resposta inválida do servidor")
    }
}
