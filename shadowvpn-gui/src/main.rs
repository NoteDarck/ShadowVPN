#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{
    self, Align, Color32, Frame, Label, Layout, Margin, RichText, Rounding, Sense, Stroke,
};
use local_server::{is_loopback_url, relay_port, LocalServer, ToggleOutcome};
use saved_networks::{SavedNetwork, SavedNetworks};
use shadowvpn_client::{ClientConfig, ClientEvent, ClientHandle};
use shadowvpn_core::{Invite, PeerInfo};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::mpsc::{self, TryRecvError};
use std::time::Duration;

mod app_paths;
mod local_server;
mod saved_networks;

const BG: Color32 = Color32::from_rgb(8, 15, 26);
const SIDEBAR_BG: Color32 = Color32::from_rgb(11, 21, 35);
const CARD: Color32 = Color32::from_rgb(17, 30, 48);
const CARD_ALT: Color32 = Color32::from_rgb(22, 38, 59);
const CARD_HOVER: Color32 = Color32::from_rgb(27, 47, 71);
const INPUT: Color32 = Color32::from_rgb(10, 20, 34);
const BORDER: Color32 = Color32::from_rgb(32, 52, 76);
const TEXT: Color32 = Color32::from_rgb(231, 240, 247);
const MUTED: Color32 = Color32::from_rgb(139, 160, 179);
const ACCENT: Color32 = Color32::from_rgb(73, 226, 197);
const ACCENT_DARK: Color32 = Color32::from_rgb(20, 92, 82);
const GREEN: Color32 = Color32::from_rgb(78, 210, 139);
const AMBER: Color32 = Color32::from_rgb(238, 184, 88);
const DANGER: Color32 = Color32::from_rgb(236, 103, 108);
const SERVER_ON: Color32 = Color32::from_rgb(70, 158, 255);

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryMode {
    Create,
    Join,
    Saved,
}

fn import_local_networks_into(saved: &mut SavedNetworks) -> Result<usize, String> {
    let names = local_server::local_network_names()?;
    if names.is_empty() {
        return Ok(0);
    }

    let server = local_server::local_invite_server_url();
    let invites = names
        .iter()
        .filter_map(|name| Invite::new(&server, name).ok())
        .collect::<Vec<_>>();
    if invites.is_empty() {
        return Err("Nenhum nome de rede do servidor local pôde ser importado.".into());
    }
    saved.remember_all(&invites)
}

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([980.0, 700.0])
        .with_min_inner_size([820.0, 580.0]);
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
    confirm_password: String,
    identity_path: String,
    invite_input: String,
    identity_fingerprint: String,
    entry_mode: EntryMode,
    creating_network: bool,
    create_result: Option<mpsc::Receiver<Result<(), String>>>,
    invite_applied: bool,
    show_password: bool,
    show_settings: bool,
    styled: bool,
    logo: Option<egui::TextureHandle>,
    status: String,
    notice: Option<String>,
    error: Option<String>,
    hostname: String,
    public_key: String,
    virtual_ip: String,
    relay: String,
    latency_ms: Option<u64>,
    peers: Vec<PeerInfo>,
    handle: Option<ClientHandle>,
    connecting: bool,
    stopping: bool,
    connected: bool,
    local_server: LocalServer,
    saved_networks: SavedNetworks,
}

impl Default for ShadowVpnApp {
    fn default() -> Self {
        let mut saved_networks = SavedNetworks::load();
        let mut notice = None;
        let mut error = None;
        if saved_networks.items().is_empty() && !saved_networks.path().exists() {
            match import_local_networks_into(&mut saved_networks) {
                Ok(0) => {}
                Ok(added) => {
                    notice = Some(format!(
                        "{added} rede(s) do servidor local adicionada(s) a Minhas redes."
                    ));
                }
                Err(failure) => error = Some(failure),
            }
        }
        let latest = saved_networks.items().first();
        let (server, group) = latest
            .map(|network| (network.server.clone(), network.network.clone()))
            .unwrap_or_else(|| ("http://127.0.0.1:8080".into(), "minha-rede".into()));
        let entry_mode = if latest.is_some() {
            EntryMode::Saved
        } else {
            EntryMode::Join
        };

        Self {
            server,
            group,
            password: String::new(),
            confirm_password: String::new(),
            identity_path: String::new(),
            invite_input: String::new(),
            identity_fingerprint: String::new(),
            entry_mode,
            creating_network: false,
            create_result: None,
            invite_applied: false,
            show_password: false,
            show_settings: false,
            styled: false,
            logo: None,
            status: "Pronto para conectar".into(),
            notice,
            error,
            hostname: String::new(),
            public_key: String::new(),
            virtual_ip: String::new(),
            relay: String::new(),
            latency_ms: None,
            peers: Vec::new(),
            handle: None,
            connecting: false,
            stopping: false,
            connected: false,
            local_server: LocalServer::new(),
            saved_networks,
        }
    }
}

impl eframe::App for ShadowVpnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.styled {
            configure_style(ctx);
            self.styled = true;
        }

        self.poll_events();
        self.local_server.refresh();
        self.ensure_logo(ctx);
        ctx.request_repaint_after(Duration::from_millis(200));

        self.top_bar(ctx);
        self.sidebar(ctx);

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(24.0, 20.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.main_content(ui);
                    });
            });

        self.settings_window(ctx);
    }
}

impl ShadowVpnApp {
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .exact_height(64.0)
            .frame(
                Frame::none()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(18.0, 10.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(logo) = &self.logo {
                        ui.add(
                            egui::Image::from_texture(egui::load::SizedTexture::new(
                                logo.id(),
                                egui::vec2(34.0, 34.0),
                            ))
                            .rounding(Rounding::same(9.0)),
                        );
                    }
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.add_space(1.0);
                        ui.label(RichText::new("ShadowVPN").size(19.0).strong().color(TEXT));
                        ui.label(
                            RichText::new("Sua rede privada, em qualquer lugar")
                                .size(10.5)
                                .color(MUTED),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (label, color) = self.connection_status();
                        pill(ui, label, color);
                        ui.add_space(6.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Configurações").size(12.0).color(TEXT),
                                )
                                .fill(CARD_ALT)
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .rounding(Rounding::same(8.0)),
                            )
                            .clicked()
                        {
                            self.show_settings = true;
                        }
                    });
                });
                let rect = ui.max_rect();
                ui.painter().hline(
                    rect.x_range(),
                    rect.bottom() - 1.0,
                    Stroke::new(1.0_f32, BORDER),
                );
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let animation_time = ctx.input(|input| input.time);
        egui::SidePanel::left("networks")
            .exact_width(232.0)
            .resizable(false)
            .frame(
                Frame::none()
                    .fill(SIDEBAR_BG)
                    .inner_margin(Margin::symmetric(14.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("REDES").size(10.0).strong().color(MUTED));
                ui.add_space(7.0);

                let (_, state_color) = self.connection_status();
                let group = self.group_name();
                let created = self.active_saved_network().is_some();
                Frame::none()
                    .fill(if self.connected { CARD_ALT } else { CARD })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if self.connected {
                            ACCENT.gamma_multiply(0.35)
                        } else {
                            BORDER
                        },
                    ))
                    .rounding(Rounding::same(12.0))
                    .inner_margin(Margin::same(12.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            status_dot(ui, state_color, 9.0);
                            ui.add_space(2.0);
                            ui.add(
                                Label::new(RichText::new(group).size(14.5).strong().color(TEXT))
                                    .truncate(),
                            );
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(if self.connected {
                                "Rede privada • conectada"
                            } else if created {
                                "Rede criada • convite disponível"
                            } else {
                                "Rede privada"
                            })
                            .size(11.0)
                            .color(if created && !self.connected {
                                ACCENT
                            } else {
                                MUTED
                            }),
                        );
                        if self.connected && !self.virtual_ip.is_empty() {
                            ui.add_space(3.0);
                            ui.label(
                                RichText::new(&self.virtual_ip)
                                    .monospace()
                                    .size(12.5)
                                    .color(ACCENT),
                            );
                        }
                    });

                ui.add_space(10.0);
                if ui
                    .add_sized(
                        [ui.available_width(), 32.0],
                        egui::Button::new(
                            RichText::new("Configurações da conexão")
                                .size(11.5)
                                .color(TEXT),
                        )
                        .fill(CARD)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .rounding(Rounding::same(8.0)),
                    )
                    .clicked()
                {
                    self.show_settings = true;
                }

                ui.add_space(12.0);
                self.local_server_card(ui, animation_time);

                ui.add_space(18.0);
                ui.label(RichText::new("SESSÃO").size(10.0).strong().color(MUTED));
                ui.add_space(6.0);
                let (state_label, state_color) = self.connection_status();
                sidebar_stat(ui, "Status", state_label, state_color);
                sidebar_stat(ui, "Identidade", "XEdDSA", TEXT);
                if self.connected {
                    sidebar_stat(
                        ui,
                        "Relay",
                        if self.relay.is_empty() {
                            "—"
                        } else {
                            &self.relay
                        },
                        TEXT,
                    );
                    let latency = self
                        .latency_ms
                        .map(|ms| format!("{ms} ms"))
                        .unwrap_or_else(|| "—".into());
                    sidebar_stat(ui, "Latência", &latency, TEXT);
                } else {
                    sidebar_stat(ui, "Computadores", "—", TEXT);
                }

                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("ShadowVPN v{}", env!("CARGO_PKG_VERSION")))
                            .size(10.0)
                            .color(MUTED),
                    );
                    ui.label(
                        RichText::new("XEdDSA + WireGuard + relay privado")
                            .size(10.0)
                            .color(MUTED),
                    );
                    ui.add_space(6.0);
                    ui.separator();
                });
            });
    }

    fn local_server_card(&mut self, ui: &mut egui::Ui, animation_time: f64) {
        let snapshot = self.local_server.snapshot();
        let active = snapshot.reachable || snapshot.starting;
        let color = if active { SERVER_ON } else { DANGER };
        let pulse = ((animation_time * 2.8).sin() as f32 * 0.5 + 0.5).clamp(0.0, 1.0);
        let mut toggle = false;

        Frame::none()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, with_alpha(color, 80)))
            .rounding(Rounding::same(12.0))
            .inner_margin(Margin::same(10.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let response = server_power_button(ui, active, pulse);
                    let tooltip = if snapshot.external {
                        "Servidor ativo fora desta janela"
                    } else if active {
                        "Clique para desligar o servidor local"
                    } else {
                        "Clique para ligar o servidor local"
                    };
                    if response.clicked() {
                        toggle = true;
                    }
                    response.on_hover_text(tooltip);

                    ui.add_space(5.0);
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("SERVIDOR LOCAL")
                                .size(9.0)
                                .strong()
                                .color(MUTED),
                        );
                        let status = if snapshot.reachable {
                            if snapshot.external {
                                "Ligado por fora"
                            } else {
                                "Ligado"
                            }
                        } else if snapshot.starting {
                            "Iniciando..."
                        } else {
                            "Desligado"
                        };
                        ui.label(RichText::new(status).size(12.0).strong().color(color));
                        let detail = if snapshot.reachable || snapshot.starting {
                            snapshot.address.clone()
                        } else {
                            "Clique para ligar".into()
                        };
                        ui.add(Label::new(RichText::new(detail).size(9.5).color(MUTED)).truncate());
                    });
                });

                if let Some(error) = &snapshot.error {
                    ui.add_space(3.0);
                    ui.label(RichText::new("Ver detalhes").size(9.0).color(DANGER))
                        .on_hover_text(error);
                }
            });

        ui.ctx().request_repaint_after(Duration::from_millis(50));
        if toggle {
            self.toggle_local_server();
        }
    }

    fn toggle_local_server(&mut self) {
        let use_lan_url = is_loopback_url(&self.server);
        match self.local_server.toggle() {
            ToggleOutcome::Started => {
                if use_lan_url {
                    self.server = self.local_server.url();
                }
                let address = self.local_server.snapshot().address;
                self.notice = Some(format!(
                    "Servidor local ligado em {address}. Libere as portas {}/tcp e {}/udp no firewall.",
                    self.local_server.port(),
                    relay_port()
                ));
                self.status = "Servidor local ligado".into();
                self.error = None;
            }
            ToggleOutcome::Stopped => {
                self.notice = Some("Servidor local desligado.".into());
                self.status = "Servidor local desligado".into();
            }
            ToggleOutcome::External => {
                let address = self.local_server.snapshot().address;
                self.error = Some(format!(
                    "Já existe um servidor ativo em {address}, iniciado fora desta janela. Encerre-o pelo terminal para controlá-lo aqui."
                ));
                self.status = "Servidor externo ativo".into();
            }
            ToggleOutcome::Failed(error) => {
                self.error = Some(error);
                self.status = "Falha ao ligar o servidor".into();
            }
        }
    }

    fn main_content(&mut self, ui: &mut egui::Ui) {
        if self.connected {
            self.network_view(ui);
        } else if self.connecting && self.error.is_none() {
            self.connecting_view(ui);
        } else {
            self.connect_view(ui);
        }
    }

    fn connect_view(&mut self, ui: &mut egui::Ui) {
        let server_before = self.server.clone();
        let group_before = self.group.clone();

        ui.add_space(8.0);
        self.entry_mode_selector(ui);
        ui.add_space(18.0);

        if self.entry_mode == EntryMode::Saved {
            self.saved_networks_view(ui);
            return;
        }

        let creating = self.entry_mode == EntryMode::Create;
        eyebrow(
            ui,
            if creating {
                "CRIAR REDE"
            } else {
                "ENTRAR EM REDE"
            },
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new(if creating {
                "Crie sua rede privada"
            } else {
                "Entre em uma rede existente"
            })
            .size(28.0)
            .strong()
            .color(TEXT),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(if creating {
                "Escolha um nome e uma senha. O convite da rede será gerado automaticamente."
            } else {
                "Cole o convite recebido e informe a senha. O servidor é preenchido sozinho."
            })
            .size(12.5)
            .color(MUTED),
        );
        ui.add_space(16.0);

        if let Some(error) = &self.error {
            banner(ui, error, DANGER);
            ui.add_space(10.0);
        }
        if let Some(notice) = &self.notice {
            banner(ui, notice, ACCENT);
            ui.add_space(10.0);
        }

        let busy = self.handle.is_some() || self.creating_network || self.connecting;
        Frame::none()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(Rounding::same(14.0))
            .inner_margin(Margin::same(18.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !creating {
                    self.invite_fields(ui, busy);
                    ui.add_space(14.0);
                }
                self.connection_fields(ui, busy, creating);
                ui.add_space(10.0);
                self.advanced_server(ui, busy);
            });

        if (self.server != server_before || self.group != group_before) && !self.invite_applied {
            self.notice = None;
        }
        self.invite_applied = false;

        ui.add_space(14.0);
        let label = if self.creating_network {
            "Criando rede..."
        } else if self.connecting {
            "Entrando..."
        } else if creating {
            "Criar rede"
        } else {
            "Entrar na rede"
        };
        if primary_button(ui, label, !busy) {
            if creating {
                self.create_network();
            } else {
                self.connect();
            }
        }

        if !creating && self.active_saved_network().is_some() {
            ui.add_space(14.0);
            self.invite_card(ui);
        }

        ui.add_space(16.0);
        if creating {
            ui.label(
                RichText::new(
                    "Depois de criar, a rede fica salva em Minhas redes com o convite pronto \
                     para copiar. Envie o link e a senha por canais separados.",
                )
                .size(10.5)
                .color(MUTED),
            );
            ui.add_space(3.0);
            ui.label(
                RichText::new(
                    "Criar a rede não exige privilégios de administrador; conectar exige.",
                )
                .size(10.5)
                .color(MUTED),
            );
        } else {
            ui.label(
                RichText::new(
                    "A interface TUN exige privilégios de administrador. No Windows, mantenha \
                     wintun.dll ao lado do executável.",
                )
                .size(10.5)
                .color(MUTED),
            );
            ui.add_space(3.0);
            ui.label(
                RichText::new(
                    "O link de convite contém apenas servidor e rede. A senha nunca é incluída.",
                )
                .size(10.5)
                .color(MUTED),
            );
        }
    }

    fn entry_mode_selector(&mut self, ui: &mut egui::Ui) {
        let mut selected = self.entry_mode;
        Frame::none()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(Rounding::same(10.0))
            .inner_margin(Margin::same(4.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if mode_button(ui, "Criar rede", selected == EntryMode::Create) {
                        selected = EntryMode::Create;
                    }
                    if mode_button(ui, "Entrar em rede", selected == EntryMode::Join) {
                        selected = EntryMode::Join;
                    }
                    if mode_button(ui, "Minhas redes", selected == EntryMode::Saved) {
                        selected = EntryMode::Saved;
                    }
                });
            });

        self.set_entry_mode(selected);
    }

    fn set_entry_mode(&mut self, selected: EntryMode) {
        if selected == self.entry_mode {
            return;
        }
        self.entry_mode = selected;
        self.error = None;
        self.notice = None;
        self.status = match selected {
            EntryMode::Create => "Preencha os dados da nova rede".into(),
            EntryMode::Join => "Preencha os dados da rede".into(),
            EntryMode::Saved => "Escolha uma rede criada por você".into(),
        };
    }

    fn saved_networks_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(2.0);
        eyebrow(ui, "MINHAS REDES");
        ui.add_space(4.0);
        ui.label(
            RichText::new("Redes que você criou")
                .size(28.0)
                .strong()
                .color(TEXT),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "Cada rede criada fica salva neste computador. Abra uma delas para recuperar o \
                 convite. A senha não é armazenada.",
            )
            .size(12.5)
            .color(MUTED),
        );
        ui.add_space(16.0);

        if let Some(error) = &self.error {
            banner(ui, error, DANGER);
            ui.add_space(10.0);
        }
        let saved_error = self.saved_networks.error().map(str::to_owned);
        if let Some(error) = saved_error.as_deref() {
            if self.error.as_deref() != Some(error) {
                banner(ui, error, DANGER);
                ui.add_space(10.0);
            }
        }
        if let Some(notice) = &self.notice {
            banner(ui, notice, ACCENT);
            ui.add_space(10.0);
        }

        if self.saved_networks.items().is_empty() {
            Frame::none()
                .fill(CARD)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .rounding(Rounding::same(14.0))
                .inner_margin(Margin::same(20.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Nenhuma rede criada ainda")
                            .size(16.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Ao criar uma rede, ela aparecerá aqui com o convite pronto para \
                             copiar, mesmo depois de fechar o ShadowVPN.",
                        )
                        .size(11.5)
                        .color(MUTED),
                    );
                    ui.add_space(12.0);
                    if primary_button(ui, "Criar minha primeira rede", true) {
                        self.set_entry_mode(EntryMode::Create);
                    }
                    ui.add_space(6.0);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Importar redes do servidor local")
                                    .size(11.5)
                                    .color(ACCENT),
                            )
                            .frame(false),
                        )
                        .on_hover_text(
                            "Adiciona à lista as redes que já existem no networks.json local.",
                        )
                        .clicked()
                    {
                        self.import_local_networks();
                    }
                });
            return;
        }

        let networks = self.saved_networks.items().to_vec();
        let mut open_index = None;
        let mut copy_index = None;
        let mut remove_index = None;

        for (index, network) in networks.iter().enumerate() {
            let active = self
                .active_saved_network()
                .map(|saved| saved == network)
                .unwrap_or(false);
            let invite = network.invite_url().unwrap_or_default();

            Frame::none()
                .fill(if active { CARD_ALT } else { CARD })
                .stroke(Stroke::new(
                    1.0_f32,
                    if active {
                        ACCENT.gamma_multiply(0.35)
                    } else {
                        BORDER
                    },
                ))
                .rounding(Rounding::same(12.0))
                .inner_margin(Margin::same(14.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        status_dot(ui, if active { ACCENT } else { MUTED }, 8.0);
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.add(
                                Label::new(
                                    RichText::new(&network.network)
                                        .size(15.0)
                                        .strong()
                                        .color(TEXT),
                                )
                                .truncate(),
                            );
                            ui.add(
                                Label::new(RichText::new(&network.server).size(10.5).color(MUTED))
                                    .truncate(),
                            );
                        });

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Remover").size(10.5).color(DANGER),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(
                                    "Remove apenas desta lista; a rede continua no servidor.",
                                )
                                .clicked()
                            {
                                remove_index = Some(index);
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Copiar").size(10.5).color(TEXT),
                                    )
                                    .fill(CARD)
                                    .stroke(Stroke::new(1.0_f32, BORDER))
                                    .rounding(Rounding::same(7.0)),
                                )
                                .clicked()
                            {
                                copy_index = Some(index);
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Abrir").size(10.5).color(TEXT),
                                    )
                                    .fill(ACCENT_DARK)
                                    .stroke(Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.5)))
                                    .rounding(Rounding::same(7.0)),
                                )
                                .clicked()
                            {
                                open_index = Some(index);
                            }
                        });
                    });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(if active { "SELECIONADA" } else { "CONVITE" })
                                .size(8.5)
                                .strong()
                                .color(if active { ACCENT } else { MUTED }),
                        );
                        if invite.is_empty() {
                            ui.label(RichText::new("convite inválido").size(10.0).color(DANGER));
                        } else {
                            ui.add(
                                Label::new(
                                    RichText::new(&invite).monospace().size(10.0).color(MUTED),
                                )
                                .truncate(),
                            )
                            .on_hover_text(&invite);
                        }
                    });
                });
            ui.add_space(10.0);
        }

        if let Some(index) = copy_index {
            if let Some(url) = networks.get(index).and_then(SavedNetwork::invite_url) {
                ui.ctx().copy_text(url);
                self.notice = Some("Convite copiado. Envie o link e a senha separadamente.".into());
            }
        }
        if let Some(index) = remove_index {
            self.remove_saved_network(index);
        }
        if let Some(index) = open_index {
            self.open_saved_network(index);
        }

        ui.add_space(4.0);
        let path = self.saved_networks.path().display().to_string();
        ui.add(
            Label::new(
                RichText::new(format!("Arquivo: {path}"))
                    .monospace()
                    .size(9.0)
                    .color(MUTED),
            )
            .truncate(),
        )
        .on_hover_text(path);
    }

    fn open_saved_network(&mut self, index: usize) {
        let Some(network) = self.saved_networks.items().get(index).cloned() else {
            return;
        };
        if self.server != network.server || self.group.trim() != network.network {
            self.password.clear();
            self.confirm_password.clear();
        }
        self.server = network.server;
        self.group = network.network.clone();
        self.entry_mode = EntryMode::Join;
        self.error = None;
        self.notice = Some(format!(
            "Rede \"{}\" carregada. O convite está pronto para ser copiado.",
            network.network
        ));
        self.status = "Rede carregada".into();
    }

    fn remove_saved_network(&mut self, index: usize) {
        match self.saved_networks.remove(index) {
            Ok(network) => {
                self.error = None;
                self.notice = Some(format!(
                    "Rede \"{}\" removida da lista. A rede continua no servidor.",
                    network.network
                ));
                self.status = "Rede removida da lista".into();
            }
            Err(error) => {
                self.error = Some(error);
                self.status = "Falha ao atualizar as redes salvas".into();
            }
        }
    }

    fn import_local_networks(&mut self) {
        match import_local_networks_into(&mut self.saved_networks) {
            Ok(0) => {
                self.error = None;
                self.notice = Some(
                    "Nenhuma rede nova encontrada no servidor local. Crie uma rede primeiro."
                        .into(),
                );
                self.status = "Nenhuma rede local para importar".into();
            }
            Ok(added) => {
                self.error = None;
                self.notice = Some(format!(
                    "{added} rede(s) do servidor local adicionada(s) a Minhas redes."
                ));
                if let Some(latest) = self.saved_networks.items().first().cloned() {
                    if self.server != latest.server || self.group.trim() != latest.network {
                        self.password.clear();
                        self.confirm_password.clear();
                    }
                    self.server = latest.server;
                    self.group = latest.network;
                }
                self.status = "Redes do servidor local importadas".into();
            }
            Err(failure) => {
                self.notice = None;
                self.error = Some(failure);
                self.status = "Falha ao importar as redes locais".into();
            }
        }
    }

    fn connecting_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(54.0);
        ui.vertical_centered(|ui| {
            ui.add(egui::Spinner::new().size(30.0).color(ACCENT));
            ui.add_space(14.0);
            ui.label(
                RichText::new("Conectando à rede")
                    .size(25.0)
                    .strong()
                    .color(TEXT),
            );
            ui.add_space(3.0);
            ui.label(RichText::new(self.group_name()).size(14.0).color(ACCENT));
            ui.add_space(8.0);
            ui.label(RichText::new(&self.status).size(12.0).color(MUTED));
            ui.add_space(22.0);
            if ui
                .add_enabled(
                    !self.stopping,
                    egui::Button::new(
                        RichText::new(if self.stopping {
                            "Cancelando..."
                        } else {
                            "Cancelar"
                        })
                        .color(TEXT),
                    )
                    .fill(CARD_ALT)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .rounding(Rounding::same(8.0))
                    .min_size(egui::vec2(120.0, 34.0)),
                )
                .clicked()
            {
                self.disconnect();
            }
        });
    }

    fn network_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                eyebrow(ui, "REDE PRIVADA");
                ui.add_space(3.0);
                ui.label(
                    RichText::new(self.group_name())
                        .size(27.0)
                        .strong()
                        .color(TEXT),
                );
                ui.add_space(1.0);
                ui.label(
                    RichText::new(format!(
                        "Rede virtual • {} de 253 computadores",
                        self.peers.len() + 1
                    ))
                    .size(11.5)
                    .color(MUTED),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !self.stopping,
                        egui::Button::new(
                            RichText::new(if self.stopping {
                                "Desconectando..."
                            } else {
                                "Desconectar"
                            })
                            .size(12.0)
                            .color(Color32::from_rgb(255, 192, 194)),
                        )
                        .fill(Color32::from_rgb(83, 35, 45))
                        .stroke(Stroke::new(1.0_f32, DANGER.gamma_multiply(0.65)))
                        .rounding(Rounding::same(8.0))
                        .min_size(egui::vec2(124.0, 34.0)),
                    )
                    .clicked()
                {
                    self.disconnect();
                }
                if ui
                    .add_enabled(
                        !self.stopping,
                        egui::Button::new(RichText::new("Copiar convite").size(11.5).color(TEXT))
                            .fill(CARD)
                            .stroke(Stroke::new(1.0_f32, BORDER))
                            .rounding(Rounding::same(8.0))
                            .min_size(egui::vec2(120.0, 34.0)),
                    )
                    .clicked()
                {
                    if let Some(url) = self.invite_url() {
                        ui.ctx().copy_text(url);
                        self.notice =
                            Some("Convite copiado. Envie o link e a senha separadamente.".into());
                    }
                }
            });
        });

        ui.add_space(14.0);
        if let Some(error) = &self.error {
            banner(ui, error, DANGER);
            ui.add_space(10.0);
        }
        if let Some(notice) = &self.notice {
            banner(ui, notice, AMBER);
            ui.add_space(10.0);
        }

        let mut copied_ip = false;
        let latency = self.latency_ms.map(|ms| format!("{ms} ms"));
        let latency_color = match self.latency_ms {
            Some(ms) if ms < 90 => GREEN,
            Some(_) => AMBER,
            None => MUTED,
        };
        Frame::none()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(Rounding::same(14.0))
            .inner_margin(Margin::same(18.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.columns(4, |cols| {
                    if metric(
                        &mut cols[0],
                        "SEU IP VIRTUAL",
                        &self.virtual_ip,
                        ACCENT,
                        19.0,
                        true,
                        true,
                    ) {
                        copied_ip = true;
                    }
                    metric(
                        &mut cols[1],
                        "RELAY",
                        if self.relay.is_empty() {
                            "—"
                        } else {
                            &self.relay
                        },
                        TEXT,
                        12.0,
                        true,
                        false,
                    );
                    metric(
                        &mut cols[2],
                        "LATÊNCIA DO SERVIDOR",
                        latency.as_deref().unwrap_or("—"),
                        latency_color,
                        16.0,
                        false,
                        false,
                    );
                    metric(
                        &mut cols[3],
                        "COMPUTADORES",
                        &format!("{} / 253", self.peers.len() + 1),
                        TEXT,
                        16.0,
                        false,
                        false,
                    );
                });
            });
        if copied_ip {
            self.notice = Some("IP virtual copiado para a área de transferência.".into());
        }

        ui.add_space(14.0);
        Frame::none()
            .fill(CARD)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(Rounding::same(14.0))
            .inner_margin(Margin::same(18.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Computadores")
                            .size(16.0)
                            .strong()
                            .color(TEXT),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("{}", self.peers.len() + 1))
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new("Atualização automática")
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
                });
                ui.add_space(8.0);

                if self.peers.is_empty() {
                    Frame::none()
                        .fill(INPUT)
                        .rounding(Rounding::same(10.0))
                        .inner_margin(Margin::symmetric(18.0, 20.0))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("Aguardando outros computadores")
                                        .size(13.0)
                                        .strong()
                                        .color(TEXT),
                                );
                                ui.add_space(3.0);
                                ui.label(
                                    RichText::new(
                                        "Quando alguém entrar nesta rede, ele aparecerá aqui.",
                                    )
                                    .size(11.0)
                                    .color(MUTED),
                                );
                            });
                        });
                } else {
                    let mut peers: Vec<&PeerInfo> = self.peers.iter().collect();
                    peers.sort_by_key(|peer| {
                        peer.virtual_ip
                            .parse::<Ipv4Addr>()
                            .unwrap_or(Ipv4Addr::UNSPECIFIED)
                    });
                    egui::Grid::new("peer_grid")
                        .num_columns(4)
                        .spacing([16.0, 9.0])
                        .show(ui, |ui| {
                            table_header(ui, "COMPUTADOR");
                            table_header(ui, "IP VIRTUAL");
                            table_header(ui, "ESTADO");
                            table_header(ui, "SESSÃO");
                            ui.end_row();

                            for peer in peers {
                                let name = if peer.hostname.is_empty() {
                                    "Peer"
                                } else {
                                    peer.hostname.as_str()
                                };
                                ui.horizontal(|ui| {
                                    avatar(ui, name, &peer.public_key);
                                    ui.add_sized(
                                        [150.0, 20.0],
                                        Label::new(
                                            RichText::new(name).size(12.5).strong().color(TEXT),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(format!(
                                        "{name}\nChave: {}\nIdentidade: {}",
                                        short_key(&peer.public_key),
                                        peer.fingerprint
                                    ));
                                });
                                ui.label(
                                    RichText::new(&peer.virtual_ip)
                                        .monospace()
                                        .size(12.0)
                                        .color(TEXT),
                                );
                                ui.horizontal(|ui| {
                                    status_dot(ui, GREEN, 7.0);
                                    ui.label(RichText::new("Online").size(11.0).color(GREEN));
                                });
                                let short_session = peer.session_id as u32;
                                ui.add(
                                    Label::new(
                                        RichText::new(format!("{short_session:08x}"))
                                            .monospace()
                                            .size(11.0)
                                            .color(MUTED),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(format!("Sessão {:016x}", peer.session_id));
                                ui.end_row();
                            }
                        });
                }
            });

        ui.add_space(12.0);
        egui::CollapsingHeader::new(
            RichText::new("Detalhes da conexão")
                .size(12.0)
                .strong()
                .color(MUTED),
        )
        .default_open(false)
        .show(ui, |ui| {
            Frame::none()
                .fill(CARD)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .rounding(Rounding::same(12.0))
                .inner_margin(Margin::same(14.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::Grid::new("connection_details")
                        .num_columns(2)
                        .spacing([20.0, 8.0])
                        .show(ui, |ui| {
                            detail_label(ui, "Nome");
                            ui.label(
                                RichText::new(if self.hostname.is_empty() {
                                    "—"
                                } else {
                                    &self.hostname
                                })
                                .size(11.5)
                                .color(TEXT),
                            );
                            ui.end_row();

                            detail_label(ui, "Servidor");
                            ui.label(
                                RichText::new(&self.server)
                                    .monospace()
                                    .size(11.0)
                                    .color(TEXT),
                            );
                            ui.end_row();

                            detail_label(ui, "Fingerprint");
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if self.identity_fingerprint.is_empty() {
                                        "—"
                                    } else {
                                        &self.identity_fingerprint
                                    })
                                    .monospace()
                                    .size(10.5)
                                    .color(TEXT),
                                )
                                .on_hover_text("Compare este código com o outro computador.");
                                if !self.identity_fingerprint.is_empty()
                                    && ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("copiar").size(9.5).color(MUTED),
                                            )
                                            .frame(false),
                                        )
                                        .clicked()
                                {
                                    ui.ctx().copy_text(self.identity_fingerprint.clone());
                                }
                            });
                            ui.end_row();

                            detail_label(ui, "Chave pública");
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(short_key(&self.public_key))
                                        .monospace()
                                        .size(11.0)
                                        .color(TEXT),
                                );
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("copiar").size(9.5).color(MUTED),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    ui.ctx().copy_text(self.public_key.clone());
                                }
                            });
                            ui.end_row();

                            detail_label(ui, "Identidade");
                            ui.label(
                                RichText::new(if self.identity_path.trim().is_empty() {
                                    "padrão do sistema"
                                } else {
                                    self.identity_path.trim()
                                })
                                .size(11.0)
                                .color(MUTED),
                            );
                            ui.end_row();
                        });
                });
        });

        ui.add_space(10.0);
        ui.label(
            RichText::new(
                "O relay encaminha apenas pacotes WireGuard cifrados entre os membros da rede.",
            )
            .size(10.0)
            .color(MUTED),
        );
    }

    fn invite_fields(&mut self, ui: &mut egui::Ui, busy: bool) {
        field_label(ui, "CONVITE DE REDE");
        ui.horizontal(|ui| {
            let response = ui.add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.invite_input)
                    .desired_width(ui.available_width() - 84.0)
                    .hint_text("cole o link shadowvpn://"),
            );
            if ui
                .add_enabled(
                    !busy && !self.invite_input.trim().is_empty(),
                    egui::Button::new(RichText::new("Usar").size(11.5).color(TEXT))
                        .fill(CARD_ALT)
                        .stroke(Stroke::new(1.0_f32, BORDER))
                        .rounding(Rounding::same(8.0)),
                )
                .clicked()
                || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)))
            {
                self.apply_invite();
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new("O convite não contém a senha; peça a senha separadamente.")
                .size(10.0)
                .color(MUTED),
        );
    }

    fn server_field(&mut self, ui: &mut egui::Ui, busy: bool) {
        field_label(ui, "SERVIDOR DE COORDENAÇÃO");
        ui.add_enabled(
            !busy,
            egui::TextEdit::singleline(&mut self.server)
                .desired_width(f32::INFINITY)
                .hint_text("http://vpn.exemplo.com:8080"),
        );
        ui.add_space(4.0);
        ui.label(
            RichText::new("Normalmente preenchido pelo convite. Use apenas ao auto-hospedar.")
                .size(10.0)
                .color(MUTED),
        );
    }

    fn advanced_server(&mut self, ui: &mut egui::Ui, busy: bool) {
        egui::CollapsingHeader::new(
            RichText::new("Servidor avançado")
                .size(11.0)
                .strong()
                .color(MUTED),
        )
        .default_open(false)
        .show(ui, |ui| {
            self.server_field(ui, busy);
        });
    }

    fn invite_card(&mut self, ui: &mut egui::Ui) {
        let Some(url) = self.invite_url() else {
            return;
        };
        Frame::none()
            .fill(INPUT)
            .stroke(Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.3)))
            .rounding(Rounding::same(10.0))
            .inner_margin(Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    RichText::new("CONVITE DESTA REDE")
                        .size(9.5)
                        .strong()
                        .color(ACCENT),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(
                        Label::new(RichText::new(&url).monospace().size(10.5).color(TEXT))
                            .truncate(),
                    )
                    .on_hover_text(&url);
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Copiar").size(10.0).color(ACCENT))
                                .fill(CARD)
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .rounding(Rounding::same(7.0)),
                        )
                        .clicked()
                    {
                        ui.ctx().copy_text(url.clone());
                        self.notice =
                            Some("Convite copiado. Envie o link e a senha separadamente.".into());
                    }
                });
            });
    }

    fn invite_url(&self) -> Option<String> {
        Invite::new(self.server.trim(), self.group.trim())
            .ok()
            .map(|invite| invite.to_url())
    }

    fn apply_invite(&mut self) {
        let value = self.invite_input.trim().to_string();
        if value.is_empty() {
            self.error = Some("Cole um link de convite.".into());
            return;
        }

        match Invite::parse(&value) {
            Ok(invite) => {
                self.server = invite.server;
                self.group = invite.network;
                self.entry_mode = EntryMode::Join;
                self.invite_input.clear();
                self.invite_applied = true;
                self.error = None;
                self.notice = Some("Convite aplicado. Informe a senha da rede.".into());
                self.status = "Convite carregado".into();
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.status = "Convite inválido".into();
            }
        }
    }

    fn connection_fields(&mut self, ui: &mut egui::Ui, busy: bool, creating: bool) {
        let enabled = !busy;
        field_label(ui, "REDE / GRUPO");
        ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(&mut self.group)
                .desired_width(f32::INFINITY)
                .hint_text("minha-rede"),
        );

        ui.add_space(10.0);
        field_label(ui, "SENHA DA REDE");
        ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(&mut self.password)
                .desired_width(f32::INFINITY)
                .password(!self.show_password)
                .hint_text(if creating {
                    "crie uma senha com pelo menos 8 caracteres"
                } else {
                    "senha compartilhada por quem criou a rede"
                }),
        );

        if creating {
            ui.add_space(10.0);
            field_label(ui, "CONFIRMAR SENHA");
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(&mut self.confirm_password)
                    .desired_width(f32::INFINITY)
                    .password(!self.show_password)
                    .hint_text("repita a senha da rede"),
            );
        }

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_password, "Mostrar senha");
        });

        if !creating {
            ui.add_space(10.0);
            field_label(ui, "IDENTIDADE (OPCIONAL)");
            ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(&mut self.identity_path)
                    .desired_width(f32::INFINITY)
                    .hint_text("caminho da chave persistente"),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new("Se vazio, a chave é salva no diretório de configuração do sistema.")
                    .size(10.0)
                    .color(MUTED),
            );
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }

        let mut open = self.show_settings;
        let mut close_requested = false;
        egui::Window::new("Configurações")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(540.0)
            .show(ctx, |ui| {
                let busy = self.handle.is_some() || self.creating_network || self.connecting;
                ui.label(
                    RichText::new("Dados da conexão")
                        .size(12.0)
                        .strong()
                        .color(TEXT),
                );
                ui.add_space(8.0);
                self.server_field(ui, busy);
                ui.add_space(10.0);
                self.connection_fields(ui, busy, false);
                if busy {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Desconecte para alterar estes dados.")
                            .size(10.5)
                            .color(AMBER),
                    );
                }
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("ShadowVPN v{}", env!("CARGO_PKG_VERSION")))
                        .size(11.0)
                        .color(MUTED),
                );
                ui.label(
                    RichText::new("XEdDSA para identidade • WireGuard para cifra")
                        .size(10.5)
                        .color(MUTED),
                );
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Fechar").color(TEXT))
                                .fill(CARD_ALT)
                                .stroke(Stroke::new(1.0_f32, BORDER))
                                .rounding(Rounding::same(8.0)),
                        )
                        .clicked()
                    {
                        close_requested = true;
                    }
                });
            });
        if close_requested {
            open = false;
        }
        self.show_settings = open;
    }

    fn connection_status(&self) -> (&'static str, Color32) {
        if self.stopping {
            ("Desconectando", AMBER)
        } else if self.connected {
            ("Conectado", GREEN)
        } else if self.creating_network {
            ("Criando rede", AMBER)
        } else if self.connecting {
            ("Conectando", AMBER)
        } else if self.active_saved_network().is_some() {
            ("Rede criada", ACCENT)
        } else if self.error.is_some() {
            ("Erro", DANGER)
        } else {
            ("Desconectado", MUTED)
        }
    }

    fn active_saved_network(&self) -> Option<&SavedNetwork> {
        let invite = Invite::new(self.server.trim(), self.group.trim()).ok()?;
        self.saved_networks
            .items()
            .iter()
            .find(|saved| saved.server == invite.server && saved.network == invite.network)
    }

    fn group_name(&self) -> &str {
        if self.group.trim().is_empty() {
            "Minha rede"
        } else {
            self.group.trim()
        }
    }

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

    fn create_network(&mut self) {
        self.error = None;
        self.notice = None;

        if self.password != self.confirm_password {
            self.error = Some("As senhas não coincidem.".into());
            self.status = "Confira a confirmação da senha".into();
            return;
        }

        let config =
            ClientConfig::new(self.server.trim(), self.group.trim(), self.password.clone());
        if let Err(error) = config.validate() {
            self.error = Some(format!("{error:#}"));
            self.status = "Dados da rede inválidos".into();
            return;
        }

        let server = self.server.clone();
        let group = self.group.clone();
        let password = self.password.clone();
        let (sender, receiver) = mpsc::channel();
        self.creating_network = true;
        self.create_result = Some(receiver);
        self.status = "Criando rede...".into();

        std::thread::spawn(move || {
            let result = shadowvpn_client::create_network(&server, &group, &password)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
    }

    fn poll_create_result(&mut self) {
        let received = self
            .create_result
            .as_ref()
            .map(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(
                    "a tarefa de criação da rede terminou sem resposta".into(),
                )),
            });

        let Some(Some(result)) = received else {
            return;
        };

        self.create_result = None;
        self.creating_network = false;
        match result {
            Ok(()) => {
                let network = self.group.trim().to_string();
                self.entry_mode = EntryMode::Join;
                match Invite::new(self.server.trim(), &network) {
                    Ok(invite) => match self.saved_networks.remember(&invite) {
                        Ok(()) => {
                            self.notice = Some(format!(
                                "Rede \"{network}\" criada e salva em Minhas redes. O convite está pronto para copiar."
                            ));
                            self.status = "Rede criada e salva".into();
                        }
                        Err(error) => {
                            self.error = Some(format!(
                                "Rede \"{network}\" criada, mas não foi possível salvá-la na lista: {error}"
                            ));
                            self.status = "Rede criada; falha ao salvar convite".into();
                        }
                    },
                    Err(error) => {
                        self.error = Some(format!(
                            "Rede \"{network}\" criada, mas o convite não pôde ser gerado: {error}"
                        ));
                        self.status = "Rede criada; convite inválido".into();
                    }
                }
            }
            Err(error) => {
                self.error = Some(error);
                self.status = "Falha ao criar a rede".into();
            }
        }
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
        self.poll_create_result();

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
                    identity_fingerprint,
                    latency_ms,
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
                    self.identity_fingerprint = identity_fingerprint;
                    self.latency_ms = Some(latency_ms);
                    self.peers = peers;
                    self.status = "Conectado".into();
                }
                ClientEvent::Peers(peers) => {
                    self.peers = peers;
                    self.notice = None;
                }
                ClientEvent::Latency(latency_ms) => {
                    self.latency_ms = Some(latency_ms);
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
        self.identity_fingerprint.clear();
        self.virtual_ip.clear();
        self.relay.clear();
        self.latency_ms = None;
        self.peers.clear();
    }
}

fn server_power_button(ui: &mut egui::Ui, on: bool, pulse: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(52.0, 52.0), Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let center = rect.center();
    let color = if on { SERVER_ON } else { DANGER };
    let hovered = response.hovered();
    let glow = if hovered { 0.42 } else { 0.18 + pulse * 0.20 };
    let painter = ui.painter();

    painter.circle_filled(
        center,
        25.0 + pulse * 1.5,
        with_alpha(color, (18.0 + glow * 40.0) as u8),
    );
    painter.circle_filled(center, 21.5, with_alpha(color, (22.0 + glow * 34.0) as u8));
    painter.circle_filled(center, 19.0, Color32::from_rgb(13, 25, 41));
    painter.circle_stroke(
        center,
        19.5,
        Stroke::new(if hovered { 2.5_f32 } else { 2.0_f32 }, color),
    );

    let stroke = Stroke::new(2.0_f32, color);
    painter.line_segment(
        [
            center + egui::vec2(0.0, -9.0),
            center + egui::vec2(0.0, -1.5),
        ],
        stroke,
    );
    let radius = 8.0;
    let start = -std::f32::consts::FRAC_PI_2 + 0.55;
    let end = 3.0 * std::f32::consts::FRAC_PI_2 - 0.55;
    let points: Vec<egui::Pos2> = (0..=28)
        .map(|index| {
            let angle = start + (end - start) * index as f32 / 28.0;
            center + egui::vec2(angle.cos() * radius, angle.sin() * radius)
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));

    response
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn configure_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SIDEBAR_BG;
    style.visuals.extreme_bg_color = INPUT;
    style.visuals.faint_bg_color = CARD_ALT;
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    style.visuals.window_rounding = Rounding::same(14.0);
    style.visuals.menu_rounding = Rounding::same(10.0);
    style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.fg_stroke.color = TEXT;
    style.visuals.widgets.inactive.bg_fill = CARD_ALT;
    style.visuals.widgets.inactive.weak_bg_fill = CARD_ALT;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.inactive.rounding = Rounding::same(8.0);
    style.visuals.widgets.hovered.fg_stroke.color = TEXT;
    style.visuals.widgets.hovered.bg_fill = CARD_HOVER;
    style.visuals.widgets.hovered.weak_bg_fill = CARD_HOVER;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.55));
    style.visuals.widgets.hovered.rounding = Rounding::same(8.0);
    style.visuals.widgets.active.fg_stroke.color = TEXT;
    style.visuals.widgets.active.bg_fill = ACCENT_DARK;
    style.visuals.widgets.active.weak_bg_fill = ACCENT_DARK;
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals.widgets.active.rounding = Rounding::same(8.0);
    style.visuals.widgets.open.bg_fill = CARD_ALT;
    style.visuals.widgets.open.fg_stroke.color = TEXT;
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 30.0;
    ctx.set_style(style);
}

fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(9.5)
            .strong()
            .color(ACCENT.gamma_multiply(0.9)),
    );
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(9.5).strong().color(MUTED));
    ui.add_space(2.0);
}

fn table_header(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(9.0).strong().color(MUTED));
}

fn detail_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(10.5).color(MUTED));
}

fn sidebar_stat(ui: &mut egui::Ui, label: &str, value: &str, color: Color32) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(10.5).color(MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(Label::new(RichText::new(value).size(10.5).color(color)).truncate());
        });
    });
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::none()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.35)))
        .rounding(Rounding::same(20.0))
        .inner_margin(Margin::symmetric(9.0, 4.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                status_dot(ui, color, 7.0);
                ui.add_space(1.0);
                ui.label(RichText::new(text).size(10.5).color(color));
            });
        });
}

fn status_dot(ui: &mut egui::Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    ui.painter().circle_filled(rect.center(), size / 2.0, color);
}

fn banner(ui: &mut egui::Ui, text: &str, color: Color32) {
    Frame::none()
        .fill(color.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.38)))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::symmetric(12.0, 9.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).size(11.0).color(color));
        });
}

fn mode_button(ui: &mut egui::Ui, text: &str, selected: bool) -> bool {
    ui.add(
        egui::Button::new(RichText::new(text).size(12.0).color(if selected {
            TEXT
        } else {
            MUTED
        }))
        .fill(if selected {
            ACCENT_DARK
        } else {
            Color32::TRANSPARENT
        })
        .stroke(Stroke::new(
            1.0_f32,
            if selected {
                ACCENT.gamma_multiply(0.65)
            } else {
                Color32::TRANSPARENT
            },
        ))
        .rounding(Rounding::same(8.0))
        .min_size(egui::vec2(132.0, 32.0)),
    )
    .clicked()
}

fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> bool {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(text)
                .size(13.0)
                .strong()
                .color(Color32::from_rgb(5, 25, 23)),
        )
        .fill(ACCENT)
        .stroke(Stroke::new(1.0_f32, ACCENT))
        .rounding(Rounding::same(9.0))
        .min_size(egui::vec2(150.0, 38.0)),
    )
    .clicked()
}

fn metric(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    color: Color32,
    size: f32,
    monospace: bool,
    copyable: bool,
) -> bool {
    ui.label(RichText::new(label).size(8.5).strong().color(MUTED));
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let mut text = RichText::new(value).size(size).strong().color(color);
        if monospace {
            text = text.monospace();
        }
        let copy_width = if copyable { 42.0 } else { 0.0 };
        let value_width = (ui.available_width() - copy_width).max(24.0);
        ui.add_sized([value_width, size + 8.0], Label::new(text).truncate())
            .on_hover_text(value.to_string());
        let mut copied = false;
        if copyable
            && ui
                .add(egui::Button::new(RichText::new("copiar").size(9.0).color(MUTED)).frame(false))
                .clicked()
        {
            ui.ctx().copy_text(value.to_string());
            copied = true;
        }
        copied
    })
    .inner
}

fn avatar(ui: &mut egui::Ui, name: &str, key: &str) {
    let size = 26.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let color = avatar_color(key);
    ui.painter()
        .circle_filled(rect.center(), size / 2.0, color.gamma_multiply(0.22));
    let initial = name
        .chars()
        .find(|character| character.is_alphanumeric())
        .unwrap_or('P')
        .to_uppercase()
        .to_string();
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(12.0),
        color,
    );
}

fn avatar_color(key: &str) -> Color32 {
    const PALETTE: [Color32; 6] = [
        Color32::from_rgb(73, 226, 197),
        Color32::from_rgb(104, 170, 255),
        Color32::from_rgb(232, 160, 102),
        Color32::from_rgb(210, 140, 235),
        Color32::from_rgb(120, 214, 135),
        Color32::from_rgb(238, 184, 88),
    ];
    let hash = key.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u32)
    });
    PALETTE[hash as usize % PALETTE.len()]
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
