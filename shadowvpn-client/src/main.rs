use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
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

    /// Token de administração para criar redes em servidor protegido.
    #[arg(
        long,
        env = "SHADOWVPN_ADMIN_TOKEN",
        hide_env_values = true,
        value_name = "TOKEN"
    )]
    admin_token: Option<String>,

    /// Arquivo com o token de administração.
    #[arg(long, env = "SHADOWVPN_ADMIN_TOKEN_FILE", value_name = "PATH")]
    admin_token_file: Option<PathBuf>,

    /// Caminho da chave de identidade persistente.
    #[arg(long, env = "SHADOWVPN_IDENTITY")]
    identity: Option<PathBuf>,

    /// Cria a rede no servidor antes de entrar. Falha se ela já existir.
    #[arg(long, env = "SHADOWVPN_CREATE")]
    create: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Cria uma rede sem conectar.
    CreateNetwork {
        /// Nome da rede.
        #[arg(long)]
        name: String,
        /// Senha da rede.
        #[arg(long, env = "SHADOWVPN_NEW_PASSWORD", hide_env_values = true)]
        password: String,
    },
    /// Lista as redes do servidor.
    ListNetworks,
    /// Remove uma rede e encerra suas sessões.
    DeleteNetwork {
        /// Nome da rede.
        name: String,
    },
    /// Troca a senha de uma rede e encerra suas sessões.
    RotatePassword {
        /// Nome da rede.
        name: String,
        /// Nova senha da rede.
        #[arg(long, env = "SHADOWVPN_NEW_PASSWORD", hide_env_values = true)]
        new_password: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let args = Args::parse();
    let (server, group) = if let Some(value) = &args.invite {
        let invite = Invite::parse(value)?;
        tracing::info!(invite = %invite.to_url(), "convite carregado");
        (invite.server, invite.network)
    } else {
        (
            args.server
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:8080".into()),
            args.group.clone().unwrap_or_else(|| "minha-rede".into()),
        )
    };
    let admin_token = if args.command.is_some() || args.create {
        resolve_admin_token(&args)?
    } else {
        None
    };

    if let Some(command) = args.command {
        run_admin_command(command, &server, admin_token).await?;
        return Ok(());
    }

    let password = args
        .password
        .context("defina --password ou a variável SHADOWVPN_PASSWORD")?;
    let config = ClientConfig::new(server, group, password)
        .with_identity_path(args.identity)
        .with_admin_token(admin_token);
    config.validate()?;

    if args.create {
        let api = Api::new(&config.server)?.with_admin_token(config.admin_token.clone());
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

fn resolve_admin_token(args: &Args) -> Result<Option<String>> {
    if let Some(token) = &args.admin_token {
        let token = token.trim();
        if !token.is_empty() {
            return Ok(Some(token.to_string()));
        }
    }

    let Some(path) = &args.admin_token_file else {
        return Ok(None);
    };
    let token = std::fs::read_to_string(path)
        .with_context(|| format!("não foi possível ler {}", path.display()))?
        .trim()
        .to_string();
    if token.is_empty() {
        bail!("o arquivo {} não contém um token", path.display());
    }
    Ok(Some(token))
}

async fn run_admin_command(
    command: Command,
    server: &str,
    admin_token: Option<String>,
) -> Result<()> {
    let token = admin_token
        .context("defina --admin-token, SHADOWVPN_ADMIN_TOKEN ou SHADOWVPN_ADMIN_TOKEN_FILE")?;
    let api = Api::new(server)?.with_admin_token(Some(token));

    match command {
        Command::CreateNetwork { name, password } => {
            api.create_network(&name, &password).await?;
            let invite = Invite::new(server, &name)?;
            println!("Rede \"{name}\" criada.");
            println!("Convite: {}", invite.to_url());
        }
        Command::ListNetworks => {
            let response = api.list_networks().await?;
            if response.networks.is_empty() {
                println!("Nenhuma rede cadastrada.");
            } else {
                println!("{:<32} {:>8} {:>8}", "REDE", "ONLINE", "MEMBROS");
                for network in response.networks {
                    println!(
                        "{:<32} {:>8} {:>8}",
                        network.name, network.online_peers, network.known_peers
                    );
                }
            }
        }
        Command::DeleteNetwork { name } => {
            api.delete_network(&name).await?;
            println!("Rede \"{name}\" removida.");
        }
        Command::RotatePassword { name, new_password } => {
            api.rotate_password(&name, &new_password).await?;
            println!("Senha da rede \"{name}\" alterada; sessões encerradas.");
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
