pub mod api;
pub mod config;
pub mod engine;
pub mod identity;
pub mod tunnel;

pub use api::ApiError;
pub use config::ClientConfig;
pub use engine::{run, ClientEvent, ClientHandle};

use anyhow::Result;

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
