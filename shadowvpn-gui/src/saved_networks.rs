use crate::app_paths;
use shadowvpn_core::Invite;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedNetwork {
    pub server: String,
    pub network: String,
}

impl SavedNetwork {
    pub fn invite_url(&self) -> Option<String> {
        Invite::new(&self.server, &self.network)
            .ok()
            .map(|invite| invite.to_url())
    }
}

pub struct SavedNetworks {
    path: PathBuf,
    items: Vec<SavedNetwork>,
    error: Option<String>,
}

impl SavedNetworks {
    pub fn load() -> Self {
        Self::load_from(app_paths::created_networks_path())
    }

    fn load_from(path: PathBuf) -> Self {
        let mut items = Vec::new();
        let mut error = None;

        match fs::read_to_string(&path) {
            Ok(content) => {
                for line in content.lines() {
                    let Ok(invite) = Invite::parse(line.trim()) else {
                        continue;
                    };
                    let network = SavedNetwork {
                        server: invite.server,
                        network: invite.network,
                    };
                    if !items.contains(&network) {
                        items.push(network);
                    }
                }
            }
            Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
            Err(failure) => {
                error = Some(format!(
                    "Não foi possível ler as redes salvas em {}: {failure}",
                    path.display()
                ));
            }
        }

        Self { path, items, error }
    }

    pub fn items(&self) -> &[SavedNetwork] {
        &self.items
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn remember(&mut self, invite: &Invite) -> Result<(), String> {
        let network = SavedNetwork {
            server: invite.server.clone(),
            network: invite.network.clone(),
        };
        self.items.retain(|saved| saved != &network);
        self.items.insert(0, network);
        self.persist()
    }

    pub fn remember_all(&mut self, invites: &[Invite]) -> Result<usize, String> {
        let mut added = 0;
        for invite in invites.iter().rev() {
            let network = SavedNetwork {
                server: invite.server.clone(),
                network: invite.network.clone(),
            };
            let is_new = !self.items.contains(&network);
            self.items.retain(|saved| saved != &network);
            self.items.insert(0, network);
            if is_new {
                added += 1;
            }
        }

        if added > 0 {
            self.persist()?;
        }
        Ok(added)
    }

    pub fn remove(&mut self, index: usize) -> Result<SavedNetwork, String> {
        if index >= self.items.len() {
            return Err("rede salva não encontrada".into());
        }
        let network = self.items.remove(index);
        self.persist()?;
        Ok(network)
    }

    fn persist(&mut self) -> Result<(), String> {
        let mut content = String::new();
        for network in &self.items {
            if let Some(url) = network.invite_url() {
                content.push_str(&url);
                content.push('\n');
            }
        }

        let result = (|| -> std::io::Result<()> {
            if let Some(parent) = self.path.parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent)?;
                }
            }

            let mut options = OpenOptions::new();
            options.create(true).write(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&self.path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
            }

            Ok(())
        })();

        match result {
            Ok(()) => {
                self.error = None;
                Ok(())
            }
            Err(failure) => {
                let message = format!(
                    "Não foi possível salvar as redes em {}: {failure}",
                    self.path.display()
                );
                self.error = Some(message.clone());
                Err(message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite(server: &str, network: &str) -> Invite {
        Invite::new(server, network).unwrap()
    }

    #[test]
    fn remembers_and_reloads_networks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("created-networks.txt");
        let mut saved = SavedNetworks::load_from(path.clone());

        saved
            .remember(&invite("http://192.168.1.10:8080", "minha-rede"))
            .unwrap();
        saved
            .remember(&invite("http://vpn.example.com", "trabalho"))
            .unwrap();

        let reloaded = SavedNetworks::load_from(path);
        assert_eq!(reloaded.items().len(), 2);
        assert_eq!(reloaded.items()[0].network, "trabalho");
        assert_eq!(reloaded.items()[1].network, "minha-rede");
        assert!(reloaded.error().is_none());
    }

    #[test]
    fn moves_duplicates_to_the_front() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("created-networks.txt");
        let mut saved = SavedNetworks::load_from(path);
        let first = invite("http://192.168.1.10:8080", "minha-rede");
        let second = invite("http://vpn.example.com", "trabalho");

        saved.remember(&first).unwrap();
        saved.remember(&second).unwrap();
        saved.remember(&first).unwrap();

        assert_eq!(saved.items().len(), 2);
        assert_eq!(saved.items()[0].network, "minha-rede");
        assert_eq!(saved.items()[1].network, "trabalho");
    }

    #[test]
    fn imports_multiple_networks_preserving_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("created-networks.txt");
        let mut saved = SavedNetworks::load_from(path.clone());
        let invites = vec![
            invite("http://192.168.1.10:8080", "primeira"),
            invite("http://192.168.1.10:8080", "segunda"),
            invite("http://192.168.1.10:8080", "terceira"),
        ];

        assert_eq!(saved.remember_all(&invites).unwrap(), 3);
        assert_eq!(
            saved
                .items()
                .iter()
                .map(|network| network.network.as_str())
                .collect::<Vec<_>>(),
            vec!["primeira", "segunda", "terceira"]
        );
        assert_eq!(SavedNetworks::load_from(path).items().len(), 3);
        assert_eq!(saved.remember_all(&invites).unwrap(), 0);
    }

    #[test]
    fn removes_a_saved_network() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("created-networks.txt");
        let mut saved = SavedNetworks::load_from(path.clone());
        saved
            .remember(&invite("http://192.168.1.10:8080", "minha-rede"))
            .unwrap();

        let removed = saved.remove(0).unwrap();
        assert_eq!(removed.network, "minha-rede");
        assert!(SavedNetworks::load_from(path).items().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn restricts_saved_network_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("created-networks.txt");
        let mut saved = SavedNetworks::load_from(path.clone());
        saved
            .remember(&invite("http://192.168.1.10:8080", "minha-rede"))
            .unwrap();

        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn ignores_invalid_lines() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("created-networks.txt");
        let valid = invite("http://192.168.1.10:8080", "minha-rede").to_url();
        fs::write(&path, format!("{valid}\nisto não é um convite\n\n")).unwrap();

        let saved = SavedNetworks::load_from(path);
        assert_eq!(saved.items().len(), 1);
        assert_eq!(saved.items()[0].network, "minha-rede");
    }
}
