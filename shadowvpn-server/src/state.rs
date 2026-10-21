use dashmap::DashMap;
use shadowvpn_core::PeerInfo;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub groups: Arc<DashMap<String, Vec<PeerInfo>>>,
    pub group_passwords: Arc<DashMap<String, String>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            groups: Arc::new(DashMap::new()),
            group_passwords: Arc::new(DashMap::new()),
        }
    }

    pub fn register_peer(&self, group: &str, peer: PeerInfo) -> (String, Vec<PeerInfo>) {
        let mut peers = self.groups.entry(group.to_owned()).or_default();
        if let Some(existing) = peers.iter().find(|p| p.public_key == peer.public_key) {
            return (
                existing.virtual_ip.clone(),
                peers
                    .iter()
                    .filter(|p| p.public_key != peer.public_key)
                    .cloned()
                    .collect(),
            );
        }
        let virtual_ip = format!("10.0.0.{}", peers.len() + 2);
        let mut assigned = peer;
        assigned.virtual_ip = virtual_ip.clone();
        let others = peers.clone();
        peers.push(assigned);
        (virtual_ip, others)
    }

    pub fn peers(&self, group: &str) -> Vec<PeerInfo> {
        self.groups
            .get(group)
            .map(|p| p.clone())
            .unwrap_or_default()
    }
}
