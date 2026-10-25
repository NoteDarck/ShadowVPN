#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use shadowvpn_client::{ClientConfig, ClientEvent, ClientHandle};
use shadowvpn_core::PeerInfo;
use std::path::PathBuf;
use std::time::Duration;

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([760.0, 620.0])
        .with_min_inner_size([620.0, 480.0]);
    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "ShadowVPN",
        options,
        Box::new(|_cc| Ok(Box::new(ShadowVpnApp::default()))),
    )
}

struct ShadowVpnApp {
    server: String,
    group: String,
    password: String,
    identity_path: String,
    show_password: bool,
    logo: Option<egui::TextureHandle>,
    status: String,
    notice: Option<String>,
    error: Option<String>,
    hostname: String,
    public_key: String,
    virtual_ip: String,
    relay: String,
    peers: Vec<PeerInfo>,
    handle: Option<ClientHandle>,
    connecting: bool,
    stopping: bool,
    connected: bool,
}

impl Default for ShadowVpnApp {
    fn default() -> Self {
        Self {
            server: "http://127.0.0.1:8080".into(),
            group: "minha-rede".into(),
            password: String::new(),
            identity_path: String::new(),
            show_password: false,
            logo: None,
            status: "Pronto para conectar".into(),
            notice: None,
            error: None,
            hostname: String::new(),
            public_key: String::new(),
            virtual_ip: String::new(),
            relay: String::new(),
            peers: Vec::new(),
            handle: None,
            connecting: false,
            stopping: false,
            connected: false,
        }
    }
}

impl eframe::App for ShadowVpnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();
        self.ensure_logo(ctx);
        ctx.request_repaint_after(Duration::from_millis(250));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(logo) = &self.logo {
                    ui.add(egui::Image::from_texture(egui::load::SizedTexture::new(
                        logo.id(),
                        egui::vec2(72.0, 72.0),
                    )));
                }
                ui.vertical(|ui| {
                    ui.heading("ShadowVPN");
                    ui.label("Rede privada com WireGuard e relay próprio.");
                });
            });
            ui.separator();

            let busy = self.handle.is_some();
            egui::Grid::new("config")
                .num_columns(2)
                .spacing([14.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Servidor");
                    ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.server)
                            .desired_width(420.0)
                            .hint_text("http://vpn.exemplo.com:8080"),
                    );
                    ui.end_row();

                    ui.label("Grupo");
                    ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.group)
                            .desired_width(220.0)
                            .hint_text("minha-rede"),
                    );
                    ui.end_row();

                    ui.label("Senha");
                    ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.password)
                            .password(!self.show_password)
                            .desired_width(220.0),
                    );
                    ui.end_row();

                    ui.label("Identidade");
                    ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.identity_path)
                            .desired_width(420.0)
                            .hint_text("opcional: caminho da chave persistente"),
                    );
                    ui.end_row();
                });

            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_password, "Mostrar senha");
                if ui
                    .add_enabled(
                        !busy,
                        egui::Button::new(if self.connecting {
                            "Conectando..."
                        } else {
                            "Conectar"
                        }),
                    )
                    .clicked()
                {
                    self.connect();
                }

                if ui
                    .add_enabled(busy && !self.stopping, egui::Button::new("Desconectar"))
                    .clicked()
                {
                    self.disconnect();
                }
            });

            ui.separator();

            match &self.error {
                Some(error) => {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 80, 80),
                        format!("Erro: {error}"),
                    );
                }
                None => {
                    let color = if self.connected {
                        egui::Color32::from_rgb(70, 170, 90)
                    } else {
                        ui.visuals().text_color()
                    };
                    ui.colored_label(color, format!("Status: {}", self.status));
                }
            }

            if let Some(notice) = &self.notice {
                ui.colored_label(egui::Color32::from_rgb(190, 150, 40), notice);
            }

            if !self.virtual_ip.is_empty() {
                ui.add_space(6.0);
                egui::Grid::new("session")
                    .num_columns(2)
                    .spacing([14.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Nome");
                        ui.monospace(&self.hostname);
                        ui.end_row();

                        ui.label("IP virtual");
                        ui.monospace(&self.virtual_ip);
                        ui.end_row();

                        ui.label("Relay");
                        ui.monospace(&self.relay);
                        ui.end_row();

                        ui.label("Chave pública");
                        ui.add(
                            egui::Label::new(egui::RichText::new(&self.public_key).monospace())
                                .selectable(true),
                        );
                        ui.end_row();
                    });
            }

            ui.add_space(10.0);
            ui.heading("Peers");
            if !self.connected {
                ui.label("Conecte-se para listar os outros computadores do grupo.");
            } else if self.peers.is_empty() {
                ui.label("Nenhum outro peer conectado. Aguardando...");
            } else {
                egui::ScrollArea::vertical()
                    .max_height(190.0)
                    .show(ui, |ui| {
                        egui::Grid::new("peers")
                            .num_columns(4)
                            .striped(true)
                            .spacing([18.0, 4.0])
                            .show(ui, |ui| {
                                ui.strong("Nome");
                                ui.strong("IP virtual");
                                ui.strong("Sessão");
                                ui.strong("Chave pública");
                                ui.end_row();

                                for peer in &self.peers {
                                    let name = if peer.hostname.is_empty() {
                                        "Peer"
                                    } else {
                                        peer.hostname.as_str()
                                    };
                                    ui.label(name);
                                    ui.monospace(&peer.virtual_ip);
                                    ui.monospace(format!("{:016x}", peer.session_id));
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(short_key(&peer.public_key))
                                                .monospace(),
                                        )
                                        .selectable(true),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
            }

            ui.add_space(12.0);
            ui.separator();
            ui.small(
                "A GUI cria a interface TUN ao conectar. No Windows, execute como Administrador \
                 e mantenha wintun.dll ao lado do executável. No Linux/macOS, execute com sudo \
                 ou conceda CAP_NET_ADMIN.",
            );
            ui.small(
                "O tráfego entre peers é cifrado com WireGuard. O servidor de relay não vê o \
                 conteúdo dos pacotes.",
            );
        });
    }
}

impl ShadowVpnApp {
    fn ensure_logo(&mut self, ctx: &egui::Context) {
        if self.logo.is_some() {
            return;
        }

        let Ok(image) = image::load_from_memory(include_bytes!("../../assets/icon.png")) else {
            return;
        };
        let image = image.into_rgba8();
        let (width, height) = image.dimensions();
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            image.as_raw(),
        );
        self.logo = Some(ctx.load_texture("shadowvpn-logo", color, egui::TextureOptions::LINEAR));
    }

    fn connect(&mut self) {
        self.error = None;
        self.notice = None;
        self.connecting = true;
        self.stopping = false;
        self.status = "Conectando...".into();
        self.clear_session();

        let identity_path = {
            let value = self.identity_path.trim();
            if value.is_empty() {
                None
            } else {
                Some(PathBuf::from(value))
            }
        };

        let config =
            ClientConfig::new(self.server.trim(), self.group.trim(), self.password.clone())
                .with_identity_path(identity_path);

        match ClientHandle::spawn(config) {
            Ok(handle) => {
                self.handle = Some(handle);
            }
            Err(error) => {
                self.connecting = false;
                self.connected = false;
                self.error = Some(format!("{error:#}"));
                self.status = "Falha ao conectar".into();
            }
        }
    }

    fn disconnect(&mut self) {
        if self.stopping {
            return;
        }
        self.stopping = true;
        self.status = "Desconectando...".into();
        if let Some(handle) = &mut self.handle {
            handle.stop();
        }
    }

    fn poll_events(&mut self) {
        let mut events = Vec::new();
        if let Some(handle) = &self.handle {
            while let Some(event) = handle.try_recv() {
                events.push(event);
            }
        }

        let mut finished = false;
        for event in events {
            match event {
                ClientEvent::Registered {
                    hostname,
                    virtual_ip,
                    relay,
                    public_key,
                    peers,
                } => {
                    self.connected = true;
                    self.connecting = false;
                    self.stopping = false;
                    self.error = None;
                    self.notice = None;
                    self.hostname = hostname;
                    self.virtual_ip = virtual_ip;
                    self.relay = relay;
                    self.public_key = public_key;
                    self.peers = peers;
                    self.status = "Conectado".into();
                }
                ClientEvent::Peers(peers) => {
                    self.peers = peers;
                    self.notice = None;
                }
                ClientEvent::Status(status) => {
                    if self.connected {
                        self.notice = Some(status);
                    } else {
                        self.status = status;
                    }
                }
                ClientEvent::Error(error) => {
                    self.connected = false;
                    self.connecting = false;
                    self.stopping = false;
                    self.notice = None;
                    self.error = Some(error);
                    self.status = "Erro".into();
                    self.clear_session();
                }
                ClientEvent::Finished => {
                    finished = true;
                }
            }
        }

        if finished {
            self.handle = None;
            self.connected = false;
            self.connecting = false;
            self.stopping = false;
            self.clear_session();
            if self.error.is_none() {
                self.status = "Desconectado".into();
            }
        }
    }

    fn clear_session(&mut self) {
        self.hostname.clear();
        self.public_key.clear();
        self.virtual_ip.clear();
        self.relay.clear();
        self.peers.clear();
    }
}

fn load_window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../../assets/icon.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

fn short_key(key: &str) -> String {
    if key.len() <= 20 {
        key.to_string()
    } else {
        format!("{}…{}", &key[..10], &key[key.len() - 6..])
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_icon_is_valid() {
        let image = image::load_from_memory(include_bytes!("../../assets/icon.png"))
            .expect("ícone PNG válido")
            .into_rgba8();
        assert_eq!(image.dimensions(), (512, 512));
        assert!(image.pixels().any(|pixel| pixel[3] > 0));
    }
}
