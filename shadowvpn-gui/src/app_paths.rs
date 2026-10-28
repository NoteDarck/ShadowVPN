use std::path::PathBuf;

pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| PathBuf::from(home).join(".local").join("share"))
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

pub fn created_networks_path() -> PathBuf {
    data_dir()
        .map(|base| base.join("shadowvpn").join("created-networks.txt"))
        .unwrap_or_else(|| PathBuf::from("data").join("created-networks.txt"))
}

pub fn server_networks_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SHADOWVPN_NETWORKS_FILE") {
        return PathBuf::from(path);
    }

    data_dir()
        .map(|base| base.join("shadowvpn").join("networks.json"))
        .unwrap_or_else(|| PathBuf::from("data").join("networks.json"))
}

pub fn server_admin_token_path() -> PathBuf {
    if let Some(path) = std::env::var_os("SHADOWVPN_ADMIN_TOKEN_FILE") {
        return PathBuf::from(path);
    }

    data_dir()
        .map(|base| base.join("shadowvpn").join("admin-token"))
        .unwrap_or_else(|| PathBuf::from("data").join("admin-token"))
}
