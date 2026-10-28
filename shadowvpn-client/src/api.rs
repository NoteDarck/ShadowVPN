use anyhow::{bail, Context, Result};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use shadowvpn_core::{
    valid_group_name, valid_public_key, CreateNetworkRequest, CreateNetworkResponse, ErrorResponse,
    MemberKeyRequest, MemberListResponse, NetworkListResponse, PeerIdentity, PeersRequest,
    PeersResponse, RegisterRequest, RegisterResponse, RotatePasswordRequest, UnregisterRequest,
};
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "servidor respondeu {}: {}",
            self.status, self.message
        )
    }
}

impl std::error::Error for ApiError {}

#[derive(Clone)]
pub struct Api {
    client: Client,
    base: String,
    admin_token: Option<String>,
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

        Ok(Self {
            client,
            base,
            admin_token: None,
        })
    }

    pub fn with_admin_token(mut self, token: Option<String>) -> Self {
        self.admin_token = token;
        self
    }

    pub async fn create_network(
        &self,
        group_name: &str,
        group_password: &str,
    ) -> Result<CreateNetworkResponse> {
        self.post_json_with_admin(
            "/networks",
            &CreateNetworkRequest {
                group_name: group_name.to_string(),
                group_password: group_password.to_string(),
            },
        )
        .await
        .context("falha ao criar rede")
    }

    pub async fn list_networks(&self) -> Result<NetworkListResponse> {
        let response = self
            .authorized(self.client.get(format!("{}/admin/networks", self.base)))
            .send()
            .await
            .context("falha ao consultar redes")?;
        decode_response(response).await
    }

    pub async fn delete_network(&self, group_name: &str) -> Result<()> {
        validate_group_name(group_name)?;
        let url = format!("{}/admin/networks/{group_name}", self.base);
        let response = self
            .authorized(self.client.delete(url))
            .send()
            .await
            .context("falha ao excluir rede")?;
        ensure_success(response).await
    }

    pub async fn list_members(&self, group_name: &str) -> Result<MemberListResponse> {
        validate_group_name(group_name)?;
        let url = format!("{}/admin/networks/{group_name}/members", self.base);
        let response = self
            .authorized(self.client.get(url))
            .send()
            .await
            .context("falha ao consultar membros da rede")?;
        decode_response(response).await
    }

    pub async fn revoke_member(&self, group_name: &str, public_key: &str) -> Result<()> {
        validate_group_name(group_name)?;
        validate_public_key(public_key)?;
        let url = format!("{}/admin/networks/{group_name}/members", self.base);
        let response = self
            .authorized(self.client.delete(url))
            .json(&MemberKeyRequest {
                public_key: public_key.to_string(),
            })
            .send()
            .await
            .context("falha ao revogar membro")?;
        ensure_success(response).await
    }

    pub async fn restore_member(&self, group_name: &str, public_key: &str) -> Result<()> {
        validate_group_name(group_name)?;
        validate_public_key(public_key)?;
        let url = format!("{}/admin/networks/{group_name}/members/restore", self.base);
        let response = self
            .authorized(self.client.post(url))
            .json(&MemberKeyRequest {
                public_key: public_key.to_string(),
            })
            .send()
            .await
            .context("falha ao liberar membro")?;
        ensure_success(response).await
    }

    pub async fn rotate_password(&self, group_name: &str, new_password: &str) -> Result<()> {
        validate_group_name(group_name)?;
        let url = format!("{}/admin/networks/{group_name}/password", self.base);
        let response = self
            .authorized(self.client.post(url))
            .json(&RotatePasswordRequest {
                new_password: new_password.to_string(),
            })
            .send()
            .await
            .context("falha ao alterar a senha da rede")?;
        ensure_success(response).await
    }

    pub async fn register(
        &self,
        group_name: &str,
        group_password: &str,
        public_key: &str,
        identity: &PeerIdentity,
        hostname: &str,
    ) -> Result<RegisterResponse> {
        self.post_json(
            "/register",
            &RegisterRequest {
                group_name: group_name.to_string(),
                group_password: group_password.to_string(),
                public_key: public_key.to_string(),
                identity: identity.clone(),
                hostname: hostname.to_string(),
            },
        )
        .await
        .context("falha ao registrar no grupo")
    }

    pub async fn heartbeat(
        &self,
        group_name: &str,
        session_token: &str,
        session_id: u64,
    ) -> Result<PeersResponse> {
        self.post_json(
            "/peers",
            &PeersRequest {
                group_name: group_name.to_string(),
                group_password: String::new(),
                session_id,
                session_token: session_token.to_string(),
            },
        )
        .await
        .context("falha ao atualizar lista de peers")
    }

    pub async fn unregister(
        &self,
        group_name: &str,
        session_token: &str,
        session_id: u64,
    ) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/unregister", self.base))
            .json(&UnregisterRequest {
                group_name: group_name.to_string(),
                group_password: String::new(),
                session_id,
                session_token: session_token.to_string(),
            })
            .send()
            .await
            .context("falha ao enviar desconexão")?;

        let status = response.status();
        if !status.is_success() && status != StatusCode::NOT_FOUND {
            let body = response.text().await.unwrap_or_default();
            return Err(ApiError {
                status,
                message: body,
            }
            .into());
        }

        Ok(())
    }

    async fn post_json<B, T>(&self, path: &str, body: &B) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        self.post_json_inner(path, body, false).await
    }

    async fn post_json_with_admin<B, T>(&self, path: &str, body: &B) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        self.post_json_inner(path, body, true).await
    }

    async fn post_json_inner<B, T>(&self, path: &str, body: &B, admin: bool) -> Result<T>
    where
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let request = self
            .client
            .post(format!("{}{}", self.base, path))
            .json(body);
        let request = if admin {
            self.authorized(request)
        } else {
            request
        };

        decode_response(request.send().await?).await
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.admin_token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

fn validate_group_name(group_name: &str) -> Result<()> {
    if valid_group_name(group_name) {
        Ok(())
    } else {
        bail!("nome de rede inválido")
    }
}

fn validate_public_key(public_key: &str) -> Result<()> {
    if valid_public_key(public_key) {
        Ok(())
    } else {
        bail!("chave pública inválida")
    }
}

async fn decode_response<T>(response: Response) -> Result<T>
where
    T: DeserializeOwned,
{
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorResponse>(&body)
            .map(|error| error.error)
            .unwrap_or(body);
        return Err(ApiError { status, message }.into());
    }

    response
        .json::<T>()
        .await
        .context("resposta inválida do servidor")
}

async fn ensure_success(response: Response) -> Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<ErrorResponse>(&body)
        .map(|error| error.error)
        .unwrap_or(body);
    Err(ApiError { status, message }.into())
}
