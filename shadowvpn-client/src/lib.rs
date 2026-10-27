pub mod api;
pub mod config;
pub mod engine;
pub mod identity;
pub mod tunnel;

pub use config::ClientConfig;
pub use engine::{run, ClientEvent, ClientHandle};

use anyhow::Result;

pub fn create_network(server: &str, group_name: &str, group_password: &str) -> Result<()> {
    let config = ClientConfig::new(server, group_name, group_password);
    config.validate()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let api = api::Api::new(&config.server)?;
        api.create_network(&config.group_name, &config.group_password)
            .await
            .map(|_| ())
    })
}
