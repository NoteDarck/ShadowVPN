use anyhow::{bail, Context, Result};
use shadowvpn_core::{valid_group_name, valid_password};
use std::path::PathBuf;

#[derive(Clone)]
pub struct ClientConfig {
    pub server: String,
    pub group_name: String,
    pub group_password: String,
    pub identity_path: Option<PathBuf>,
    pub admin_token: Option<String>,
}

impl ClientConfig {
    pub fn new(
        server: impl Into<String>,
        group_name: impl Into<String>,
        group_password: impl Into<String>,
    ) -> Self {
        Self {
            server: server.into(),
            group_name: group_name.into(),
            group_password: group_password.into(),
            identity_path: None,
            admin_token: None,
        }
    }

    pub fn with_identity_path(mut self, path: Option<PathBuf>) -> Self {
        self.identity_path = path;
        self
    }

    pub fn with_admin_token(mut self, token: Option<String>) -> Self {
        self.admin_token = token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self
    }

    pub fn validate(&self) -> Result<()> {
        if !valid_group_name(&self.group_name) {
            bail!("nome do grupo inválido: use até 64 caracteres alfanuméricos, '-' ou '_'");
        }
        if !valid_password(&self.group_password) {
            bail!("a senha do grupo precisa ter entre 8 e 128 caracteres");
        }

        let server = self.server.trim().trim_end_matches('/');
        let url = reqwest::Url::parse(server)
            .with_context(|| format!("URL do servidor inválida: {}", self.server))?;
        if !matches!(url.scheme(), "http" | "https") {
            bail!("o servidor deve usar http:// ou https://");
        }
        if url.host_str().is_none() {
            bail!("a URL do servidor precisa incluir um host");
        }
        if !url.username().is_empty() || url.password().is_some() {
            bail!("a URL do servidor não deve incluir usuário ou senha");
        }
        if url.query().is_some() || url.fragment().is_some() {
            bail!("a URL do servidor não deve incluir query string ou fragmento");
        }

        Ok(())
    }

    pub(crate) fn normalized(&self) -> Self {
        let mut config = self.clone();
        config.server = self.server.trim().trim_end_matches('/').to_string();
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_config() {
        let mut config = ClientConfig::new("http://127.0.0.1:8080/", "grupo-1", "senha-segura");
        assert!(config.validate().is_ok());
        assert_eq!(config.normalized().server, "http://127.0.0.1:8080");

        config.group_name = "grupo inválido".into();
        assert!(config.validate().is_err());

        config.group_name = "grupo".into();
        config.server = "ftp://example.com".into();
        assert!(config.validate().is_err());

        config.server = "http://user:pass@example.com".into();
        assert!(config.validate().is_err());
    }
}
