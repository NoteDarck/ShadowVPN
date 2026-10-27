use anyhow::{Context, Result};
use clap::Parser;
use shadowvpn_client::{api::Api, run, ClientConfig};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "shadowvpn-client",
    version,
    about = "Cliente de rede privada com relay, inspirado no Radmin VPN"
)]
struct Args {
    /// URL do servidor de coordenação.
    #[arg(
        long,
        env = "SHADOWVPN_SERVER",
        default_value = "http://127.0.0.1:8080"
    )]
    server: String,

    /// Nome do grupo/rede.
    #[arg(long, env = "SHADOWVPN_GROUP", default_value = "minha-rede")]
    group: String,

    /// Senha do grupo (recomendado usar a variável SHADOWVPN_PASSWORD).
    #[arg(long, env = "SHADOWVPN_PASSWORD", hide_env_values = true)]
    password: Option<String>,

    /// Caminho da chave de identidade persistente.
    #[arg(long, env = "SHADOWVPN_IDENTITY")]
    identity: Option<PathBuf>,

    /// Cria a rede no servidor antes de entrar. Falha se ela já existir.
    #[arg(long, env = "SHADOWVPN_CREATE")]
    create: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let args = Args::parse();
    let password = args
        .password
        .context("defina --password ou a variável SHADOWVPN_PASSWORD")?;

    let config =
        ClientConfig::new(args.server, args.group, password).with_identity_path(args.identity);
    config.validate()?;

    if args.create {
        let api = Api::new(&config.server)?;
        api.create_network(&config.group_name, &config.group_password)
            .await?;
        tracing::info!(group = %config.group_name, "rede criada");
    }

    let (stop_tx, stop_rx) = watch::channel(false);
    let mut session = Box::pin(run(config, stop_rx, None));

    tokio::select! {
        result = &mut session => result?,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("encerrando; removendo peer do grupo...");
            let _ = stop_tx.send(true);
            match tokio::time::timeout(Duration::from_secs(10), &mut session).await {
                Ok(result) => result?,
                Err(_) => tracing::warn!("o servidor não respondeu ao encerramento"),
            }
        }
    }

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("SHADOWVPN_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
