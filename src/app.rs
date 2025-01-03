use std::{
    sync::{mpsc::Receiver, Arc},
    thread::JoinHandle,
    time::Duration,
    usize,
};

use egui::{
    ahash::HashMap, mutex::Mutex, vec2, Button, Color32, Context, Layout, ScrollArea,
    SelectableLabel, Ui, Window,
};
use egui_extras::{Column, TableBuilder};
use rumqttc::{
    tokio_rustls::rustls::pki_types::SignatureVerificationAlgorithm, Client, Event, Incoming,
    MqttOptions,
};
use serde::{Deserialize, Serialize};

use crate::mqtt_servermanager::{MqttServerManager, MqttServerManagerEvent};

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct TemplateApp {
    host: String,
    port: String,

    send_topic: String,
    #[serde(skip)]
    send_payload: String,

    #[serde(skip)]
    manager: MqttServerManager,
    #[serde(skip)]
    mqtt_client: Option<Client>,
    #[serde(skip)]
    mqtt_jh: Option<JoinHandle<()>>,
    #[serde(skip)]
    mqtt_receiver: Option<Receiver<Event>>,
    #[serde(skip)]
    incoming: Vec<MqttServerManagerEvent>,
    servers: MqttServers,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct MqttServer {
    edit_display: bool,
    name: String,
    host: String,
    port: String,
    new_subscription: String,
    subscriptions: Vec<String>,
}

impl MqttServer {
    fn new() -> Self {
        Self {
            edit_display: true,
            ..Self::default()
        }
    }
    fn name(&self) -> String {
        if !self.name.is_empty() {
            return self.name.to_owned();
        }
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct MqttServers(HashMap<u16, MqttServer>);

impl MqttServers {
    fn push(&mut self, server: MqttServer) {
        let id = fastrand::u16(0..u16::MAX);
        self.0.insert(id, server);
    }
    fn get_by_index(&self, id: &u16) -> Option<&MqttServer> {
        self.0.get(id)
    }
    fn selectable_list(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.with_layout(Layout::top_down_justified(egui::Align::Center), |ui| {
                for (_, server) in self.0.iter_mut() {
                    if ui.button(server.name()).clicked() {
                        server.edit_display = true;
                    }
                }
            });
        });
    }

    fn edit_windows(&mut self, ctx: &Context) {
        let mut delete_ids = vec![];
        for (id, server) in self.0.iter_mut() {
            egui::Window::new(server.name())
                .id(id.to_string().into())
                .open(&mut server.edit_display)
                .show(ctx, |ui| {
                    ui.label(id.to_string());
                    ui.separator();
                    ui.heading("Connection");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut server.host).hint_text("127.0.0.1"));
                        ui.label("Host");
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut server.port).hint_text("1883"));
                        ui.label("Port");
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut server.name).hint_text("alias"));
                        ui.label("Alias");
                    });
                    ui.separator();
                    ui.heading("Subscriptions");
                    let mut delete_subs = vec![];
                    for sub in server.subscriptions.iter_mut() {
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(sub).interactive(false));
                            if ui.button("Del").clicked() {
                                delete_subs.push(sub.clone());
                            }
                        });
                    }
                    server
                        .subscriptions
                        .retain(|sub| !delete_subs.contains(sub));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut server.new_subscription).hint_text("#"),
                        );
                        if ui.button("Add").clicked() {
                            server.subscriptions.push(server.new_subscription.clone());
                            server.new_subscription.clear();
                        }
                    });
                    ui.separator();
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("Delete").fill(Color32::DARK_RED))
                            .clicked()
                        {
                            delete_ids.push(*id);
                        }
                    });
                });
        }
        for id in delete_ids {
            self.0.remove(&id);
        }
    }
}

impl Default for TemplateApp {
    fn default() -> Self {
        Self {
            // Example stuff:
            host: "".to_owned(),
            port: "".to_owned(),
            send_payload: "".to_owned(),
            send_topic: "".to_owned(),
            manager: MqttServerManager::new(),
            mqtt_client: None,
            mqtt_jh: None,
            mqtt_receiver: None,
            incoming: vec![],
            servers: MqttServers::default(),
        }
    }
}

impl TemplateApp {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        if let Some(storage) = cc.storage {
            return eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
        }

        Default::default()
    }
}

impl eframe::App for TemplateApp {
    /// Called by the frame work to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for event in self.manager.pull_events() {
            self.incoming.push(event);
        }
        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // The top panel is often a good place for a menu bar:

            egui::menu::bar(ui, |ui| {
                // NOTE: no File->Quit on web pages!
                let is_web = cfg!(target_arch = "wasm32");
                if !is_web {
                    ui.menu_button("File", |ui| {
                        if ui.button("Reset App Settings").clicked() {
                            *self = Self::default();
                        }
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.add_space(0.0);
                }
                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        egui::SidePanel::left("mqtt_servers")
            .max_width(450.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("MQTT Servers");
                    if ui.button("New").clicked() {
                        self.servers.push(MqttServer::new());
                    }
                });
                ScrollArea::vertical().show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        self.servers.selectable_list(ui);
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.servers.edit_windows(ctx);
        });

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                powered_by_egui_and_eframe(ui);
                egui::warn_if_debug_build(ui);
            });
        });
    }
}

fn powered_by_egui_and_eframe(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label("Powered by ");
        ui.hyperlink_to("egui", "https://github.com/emilk/egui");
        ui.label(" and ");
        ui.hyperlink_to(
            "eframe",
            "https://github.com/emilk/egui/tree/master/crates/eframe",
        );
        ui.label(".");
    });
}
