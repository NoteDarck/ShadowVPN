use base64::{engine::general_purpose::STANDARD, Engine};
use boringtun::x25519::{PublicKey, StaticSecret};
use eframe::egui;
use reqwest::blocking::Client;
use shadowvpn_core::{PeerInfo, RegisterRequest, RegisterResponse};

struct ShadowVpnApp {
    server: String,
    group: String,
    password: String,
    endpoint: String,
    status: String,
    virtual_ip: String,
    peers: Vec<PeerInfo>,
    connected: bool,
}

impl Default for ShadowVpnApp {
    fn default() -> Self {
        Self {
            server: "http://127.0.0.1:8080".into(),
            group: "minha-rede".into(),
            password: String::new(),
            endpoint: "127.0.0.1:51820".into(),
            status: "Pronto para conectar".into(),
            virtual_ip: "-".into(),
            peers: Vec::new(),
            connected: false,
        }
    }
}

impl eframe::App for ShadowVpnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("ShadowVPN");
            ui.label("Cliente desktop para sua rede privada WireGuard.");
            ui.separator();
            egui::Grid::new("settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Servidor"); ui.text_edit_singleline(&mut self.server); ui.end_row();
                ui.label("Grupo"); ui.text_edit_singleline(&mut self.group); ui.end_row();
                ui.label("Senha"); ui.add(egui::TextEdit::singleline(&mut self.password).password(true)); ui.end_row();
                ui.label("Endpoint público"); ui.text_edit_singleline(&mut self.endpoint); ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.connected, egui::Button::new("Conectar")).clicked() { self.connect(); }
                if ui.add_enabled(self.connected, egui::Button::new("Desconectar")).clicked() { self.connected = false; self.status = "Desconectado".into(); }
            });
            ui.separator();
            ui.label(format!("Status: {}", self.status));
            ui.label(format!("IP virtual: {}", self.virtual_ip));
            ui.heading("Peers encontrados");
            if self.peers.is_empty() { ui.label("Nenhum peer no grupo ainda."); }
            for peer in &self.peers { ui.label(format!("{} — {}", peer.virtual_ip, peer.endpoint)); }
            ui.add_space(12.0);
            ui.small("Nota: a interface TUN exige privilégios de administrador e é iniciada pelo shadowvpn-client.");
        });
    }
}

impl ShadowVpnApp {
    fn connect(&mut self) {
        if self.password.len() < 8 {
            self.status = "A senha precisa ter pelo menos 8 caracteres".into();
            return;
        }
        let private = StaticSecret::random_from_rng(rand_core::OsRng);
        let public = PublicKey::from(&private);
        let body = RegisterRequest {
            group_name: self.group.clone(),
            group_password: self.password.clone(),
            public_key: STANDARD.encode(public.as_bytes()),
            endpoint: self.endpoint.clone(),
        };
        let url = format!("{}/register", self.server.trim_end_matches('/'));
        match Client::new()
            .post(url)
            .json(&body)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<RegisterResponse>())
        {
            Ok(response) => {
                self.virtual_ip = response.virtual_ip;
                self.peers = response.peers;
                self.connected = true;
                self.status = "Registrado no grupo (túnel disponível no cliente CLI)".into();
            }
            Err(error) => {
                self.status = format!("Falha ao conectar: {error}");
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "ShadowVPN",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(ShadowVpnApp::default()))),
    )
}
