use anyhow::{Context, Result};
use clap::Parser;
use shadowvpn_client::{api::Api, run, ClientConfig};
use shadowvpn_core::Invite;
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
    #[arg(long, env = "SHADOWVPN_SERVER")]
    server: Option<String>,

    /// Nome do grupo/rede.
    #[arg(long, env = "SHADOWVPN_GROUP")]
    group: Option<String>,

    /// Convite shadowvpn:// que preenche servidor e rede.
    #[arg(long, env = "SHADOWVPN_INVITE", value_name = "LINK")]
    invite: Option<String>,

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

    let (server, group) = if let Some(value) = args.invite {
        let invite = Invite::parse(&value)?;
        tracing::info!(invite = %invite.to_url(), "convite carregado");
        (invite.server, invite.network)
    } else {
        (
            args.server
                .unwrap_or_else(|| "http://127.0.0.1:8080".into()),
            args.group.unwrap_or_else(|| "minha-rede".into()),
        )
    };

    let config = ClientConfig::new(server, group, password).with_identity_path(args.identity);
    config.validate()?;

    if args.create {
        let api = Api::new(&config.server)?;
        api.create_network(&config.group_name, &config.group_password)
            .await?;
        let invite = Invite::new(&config.server, &config.group_name)?;
        tracing::info!(group = %config.group_name, invite = %invite.to_url(), "rede criada");
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
