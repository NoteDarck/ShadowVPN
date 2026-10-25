use anyhow::Result;
use std::net::Ipv4Addr;
use tokio_util::codec::Framed;

pub type TunFramed = Framed<tun::AsyncDevice, tun::TunPacketCodec>;

pub const TUN_MTU: u16 = 1380;

pub fn create(virtual_ip: Ipv4Addr) -> Result<TunFramed> {
    let mut config = tun::Configuration::default();
    config
        .address(virtual_ip)
        .netmask(Ipv4Addr::new(255, 255, 255, 0))
        .mtu(TUN_MTU)
        .up();

    #[cfg(target_os = "linux")]
    {
        config.tun_name("shadowvpn0");
        config.platform_config(|platform| {
            platform.ensure_root_privileges(true);
        });
    }

    #[cfg(target_os = "macos")]
    {
        config.destination(Ipv4Addr::new(26, 26, 26, 1));
    }

    #[cfg(target_os = "windows")]
    {
        config.tun_name("ShadowVPN");
        let wintun = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.join("wintun.dll")))
            .ok_or_else(|| anyhow::anyhow!("não foi possível localizar a pasta do executável"))?;
        config.platform_config(|platform| {
            platform.wintun_file(wintun);
            platform.wait_for_interfaces(true, false, std::time::Duration::from_secs(5));
        });
    }

    let device = tun::create_as_async(&config).map_err(tun_error)?;
    Ok(device.into_framed())
}

fn tun_error(error: tun::Error) -> anyhow::Error {
    #[cfg(target_os = "windows")]
    let hint = "Execute como Administrador e mantenha wintun.dll ao lado do executável.";
    #[cfg(target_os = "macos")]
    let hint = "Execute com sudo para permitir a criação da interface utun.";
    #[cfg(target_os = "linux")]
    let hint = "Execute como root (ou conceda CAP_NET_ADMIN) e verifique se /dev/net/tun existe.";
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let hint = "A criação da interface TUN exige privilégios administrativos.";

    anyhow::anyhow!("falha ao criar a interface TUN: {error}. {hint}")
}
