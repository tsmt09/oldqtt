use std::{collections::VecDeque, sync::mpsc::Receiver, thread::JoinHandle};

use egui::{ahash::HashMap, epaint::ColorMode, Color32, Context, Layout, ScrollArea, Stroke, Ui};
use egui_extras::Column;
use rumqttc::{Client, Event};
use serde::{Deserialize, Serialize};

use crate::mqtt_servermanager::{MqttServerManager, MqttServerManagerEvent, Server};

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
pub struct MqttServer {
    display: bool,
    edit_display: bool,
    name: String,
    host: String,
    port: String,
    #[serde(skip)]
    new_subscription: String,
    #[serde(skip)]
    new_pub_topic: String,
    #[serde(skip)]
    new_pub_payload: String,
    subscriptions: Vec<String>,
    #[serde(skip)]
    messages: VecDeque<MqttServerManagerEvent>,
    max_messages: usize,
    table_messages: usize,
}

impl MqttServer {
    pub fn new() -> Self {
        Self {
            edit_display: true,
            ..Self::default()
        }
    }
    pub fn name(&self) -> String {
        if !self.name.is_empty() {
            return self.name.to_owned();
        }
        format!("{}:{}", self.host(), self.port())
    }
    pub fn port(&self) -> u16 {
        self.port.parse().unwrap_or(1883)
    }
    pub fn host(&self) -> String {
        if self.host.is_empty() {
            String::from("127.0.0.1")
        } else {
            self.host.clone()
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct MqttServers {
    servers: HashMap<u32, MqttServer>,
}

impl MqttServers {
    fn add_server(&mut self, server: MqttServer) {
        let id = fastrand::u32(0..u32::MAX);
        self.servers.insert(id, server);
    }
    fn push(&mut self, event: MqttServerManagerEvent) {
        if let Some(server) = self.servers.get_mut(&event.client) {
            server.messages.push_back(event);
            if server.messages.len() > server.max_messages {
                server.messages.pop_front();
            }
        }
    }
    fn _get(&self, id: &u32) -> Option<&MqttServer> {
        self.servers.get(id)
    }
    fn selectable_list(&mut self, ui: &mut Ui, manager: &mut MqttServerManager) {
        ui.vertical(|ui| {
            for (id, server) in self.servers.iter_mut() {
                let connected = manager.servers().contains_key(id);
                ui.with_layout(Layout::top_down_justified(egui::Align::Center), |ui| {
                    ui.horizontal(|ui| {
                        let mut button = egui::Button::new(server.name());
                        if connected {
                            button = button.stroke(Stroke::new(1.0, Color32::GREEN));
                        }
                        if ui.add(button).clicked() {
                            server.display = true;
                        }
                        if ui.button("edit").clicked() {
                            server.edit_display = true;
                        }
                        if !connected {
                            if ui
                                .add(egui::Button::new("c").fill(Color32::GREEN))
                                .clicked()
                            {
                                let ctx = ui.ctx().clone();
                                manager.connect(*id, server, ctx);
                            }
                        } else {
                            if ui.add(egui::Button::new("d")).clicked() {
                                manager.disconnect(*id);
                            }
                        }
                    });
                });
            }
        });
    }

    fn windows(&mut self, ctx: &Context, manager: &mut MqttServerManager) {
        for (id, server) in self.servers.iter_mut() {
            let connected = manager.servers().contains_key(id);
            egui::Window::new(format!("MQTT: {}", server.name()))
                .id(format!("mqtt_{}", id).into())
                .open(&mut server.display)
                .default_width(150.0)
                .default_height(150.0)
                .show(ctx, |ui| {
                    ui.small(format!("'{:x}' connected: {}", id, connected));
                    ui.small(format!("'{:x}' messages: {}", id, server.messages.len()));
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(
                            &mut server.table_messages,
                            (0 as usize)..=(10_000 as usize),
                        ));
                        ui.label("max rendered messages in table");
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut server.new_pub_topic)
                                .hint_text("topic")
                                .interactive(connected),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut server.new_pub_payload)
                                .hint_text("payload")
                                .interactive(connected),
                        );
                        if ui.button("publish").clicked() {
                            if let Some(connected_client) = manager.servers().get(id) {
                                connected_client.publish(
                                    server.new_pub_topic.clone(),
                                    server.new_pub_payload.clone(),
                                );
                            }
                        };
                    });
                    ui.separator();
                    egui_extras::TableBuilder::new(ui)
                        .column(Column::auto().at_least(20.0))
                        .column(Column::remainder())
                        .resizable(true)
                        .striped(true)
                        .header(20.0, |mut header| {
                            header.col(|ui| {
                                ui.strong("Topic");
                            });
                            header.col(|ui| {
                                ui.strong("Payload");
                            });
                        })
                        .body(|mut body| {
                            let mut exit_count = 0;
                            for message in server.messages.iter().rev() {
                                if exit_count >= server.table_messages {
                                    return;
                                }
                                let event = &message.event;
                                body.row(10.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(event.topic.clone());
                                    });
                                    row.col(|ui| {
                                        ui.label(
                                            String::from_utf8(event.payload.to_vec())
                                                .unwrap_or(String::from("ERROR PARSING UTF8")),
                                        );
                                    });
                                    exit_count += 1;
                                });
                            }
                        });
                });
        }
    }

    fn edit_windows(&mut self, ctx: &Context, manager: &mut MqttServerManager) {
        let mut delete_ids = vec![];
        for (id, server) in self.servers.iter_mut() {
            let connected = manager.servers().contains_key(id);
            egui::Window::new(format!("Edit {}", server.name()))
                .id(id.to_string().into())
                .open(&mut server.edit_display)
                .show(ctx, |ui| {
                    ui.small(format!("random id: {:0x}", id));
                    ui.separator();
                    ui.heading("Connection");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut server.host)
                                .hint_text("127.0.0.1")
                                .interactive(!connected),
                        );
                        ui.label("Host");
                    });
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut server.port)
                                .hint_text("1883")
                                .interactive(!connected),
                        );
                        ui.label("Port");
                    });
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut server.name)
                                .hint_text("alias")
                                .interactive(!connected),
                        );
                        ui.label("Alias");
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::Slider::new(
                            &mut server.max_messages,
                            (0 as usize)..=(1_000_000 as usize),
                        ));
                        ui.label("max stored messages");
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
                    // if !manager.servers().contains_key(id) {
                    //     if ui
                    //         .add(egui::Button::new("Connect").fill(Color32::GREEN))
                    //         .clicked()
                    //     {
                    //         let ctx = ui.ctx().clone();
                    //         manager.connect(*id, server, ctx);
                    //     }
                    // } else {
                    //     if ui.add(egui::Button::new("Disconnect")).clicked() {
                    //         manager.disconnect(*id);
                    //         server.display = false;
                    //     }
                    // }
                    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(egui::Button::new("Delete").fill(Color32::RED))
                            .clicked()
                        {
                            delete_ids.push(*id);
                        }
                    });
                });
            if let Some(mqtt_server) = manager.servers_mut().get_mut(id) {
                mqtt_server.sync_subs(&server.subscriptions.clone());
            }
        }
        for id in delete_ids {
            manager.disconnect(id);
            self.servers.remove(&id);
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
            self.servers.push(event);
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

        egui::SidePanel::left("mqtt_clients")
            .max_width(450.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("MQTT Clients");
                    if ui.button("New").clicked() {
                        self.servers.add_server(MqttServer::new());
                    }
                });
                ScrollArea::vertical().show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        self.servers.selectable_list(ui, &mut self.manager);
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |_| {
            self.servers.edit_windows(ctx, &mut self.manager);
            self.servers.windows(ctx, &mut self.manager);
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
