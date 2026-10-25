use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use boringtun::x25519::{PublicKey, StaticSecret};
use rand_core::OsRng;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn default_identity_path() -> Result<PathBuf> {
    let base = config_dir().context(
        "não foi possível determinar a pasta de configuração; defina SHADOWVPN_IDENTITY",
    )?;
    Ok(base.join("shadowvpn").join("identity.key"))
}

pub fn load_or_create(path: &Path) -> Result<StaticSecret> {
    if path.exists() {
        return read_identity(path);
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("não foi possível criar {}", parent.display()))?;
        }
    }

    let secret = StaticSecret::random_from_rng(OsRng);
    let encoded = BASE64.encode(secret.as_bytes());

    match write_identity(path, &encoded) {
        Ok(()) => Ok(secret),
        Err(error) if path.exists() => {
            tracing::debug!(%error, path = %path.display(), "identidade criada por outra instância");
            read_identity(path)
        }
        Err(error) => Err(error),
    }
}

pub fn public_key_b64(secret: &StaticSecret) -> String {
    BASE64.encode(PublicKey::from(secret).as_bytes())
}

fn read_identity(path: &Path) -> Result<StaticSecret> {
    let encoded = fs::read_to_string(path)
        .with_context(|| format!("não foi possível ler {}", path.display()))?;
    let bytes = BASE64
        .decode(encoded.trim())
        .with_context(|| format!("identidade inválida em {}", path.display()))?;
    let key: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("identidade inválida em {}", path.display()))?;

    restrict_permissions(path);

    Ok(StaticSecret::from(key))
}

fn write_identity(path: &Path, encoded: &str) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(path)
        .with_context(|| format!("não foi possível criar {}", path.display()))?;
    file.write_all(encoded.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;

    restrict_permissions(path);
    Ok(())
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn config_dir() -> Option<PathBuf> {
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
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

pub fn validate_identity(path: &Path) -> Result<()> {
    if path.exists() {
        let _ = read_identity(path)?;
        Ok(())
    } else {
        bail!("identidade não encontrada em {}", path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_reloads_identity() {
        let dir = tempfile::tempdir().expect("diretório temporário");
        let path = dir.path().join("identity.key");

        let first = load_or_create(&path).expect("cria identidade");
        let second = load_or_create(&path).expect("reabre identidade");

        assert_eq!(first.as_bytes(), second.as_bytes());
        assert_eq!(public_key_b64(&first), public_key_b64(&second));
        assert!(path.exists());
    }
}
