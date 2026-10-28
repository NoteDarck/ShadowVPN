pub mod api;
pub mod config;
pub mod engine;
pub mod identity;
pub mod tunnel;

pub use api::ApiError;
pub use config::ClientConfig;
pub use engine::{run, ClientEvent, ClientHandle};

use anyhow::{bail, Result};
use shadowvpn_core::{MemberListResponse, NetworkListResponse};
use std::future::Future;

pub fn create_network(
    server: &str,
    group_name: &str,
    group_password: &str,
    admin_token: Option<&str>,
) -> Result<()> {
    let config = ClientConfig::new(server, group_name, group_password)
        .with_admin_token(admin_token.map(str::to_owned));
    config.validate()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let api = api::Api::new(&config.server)?.with_admin_token(config.admin_token.clone());
        api.create_network(&config.group_name, &config.group_password)
            .await
            .map(|_| ())
    })
}

pub fn list_networks(server: &str, admin_token: &str) -> Result<NetworkListResponse> {
    let api = admin_api(server, admin_token)?;
    block_on(api.list_networks())
}

pub fn list_members(
    server: &str,
    admin_token: &str,
    group_name: &str,
) -> Result<MemberListResponse> {
    let api = admin_api(server, admin_token)?;
    block_on(api.list_members(group_name))
}

pub fn revoke_member(
    server: &str,
    admin_token: &str,
    group_name: &str,
    public_key: &str,
) -> Result<()> {
    let api = admin_api(server, admin_token)?;
    block_on(api.revoke_member(group_name, public_key))
}

pub fn restore_member(
    server: &str,
    admin_token: &str,
    group_name: &str,
    public_key: &str,
) -> Result<()> {
    let api = admin_api(server, admin_token)?;
    block_on(api.restore_member(group_name, public_key))
}

pub fn delete_network(server: &str, admin_token: &str, group_name: &str) -> Result<()> {
    let api = admin_api(server, admin_token)?;
    block_on(api.delete_network(group_name))
}

pub fn rotate_password(
    server: &str,
    admin_token: &str,
    group_name: &str,
    new_password: &str,
) -> Result<()> {
    let api = admin_api(server, admin_token)?;
    block_on(api.rotate_password(group_name, new_password))
}

fn admin_api(server: &str, admin_token: &str) -> Result<api::Api> {
    if admin_token.trim().is_empty() {
        bail!("token de administração vazio");
    }
    Ok(api::Api::new(server)?.with_admin_token(Some(admin_token.trim().to_owned())))
}

fn block_on<T>(future: impl Future<Output = Result<T>>) -> Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(future)
}
