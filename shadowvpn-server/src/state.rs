use dashmap::DashMap;
use shadowvpn_core::PeerInfo;
use std::sync::Arc;

/// Estado global do servidor: grupos e seus peers
#[derive(Clone)]
pub struct AppState {
    /// group_name -> Vec<PeerInfo>
    pub groups: Arc<DashMap<String, Vec<PeerInfo>>>,
    /// Senhas dos grupos (group_name -> password)
    pub group_passwords: Arc<DashMap<String, String>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            groups: Arc::new(DashMap::new()),
            group_passwords: Arc::new(DashMap::new()),
        }
    }

    /// Atribui o próximo IP virtual disponível no grupo (10.0.0.2, 10.0.0.3, ...)
    pub fn allocate_ip(&self, group: &str) -> String {
        let peers = self.groups.get(group);
        let next = match peers {
            Some(p) => p.len() + 2,
            None => 2,
        };
        format!("10.0.0.{}", next)
    }
}