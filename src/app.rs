use std::{sync::mpsc::Receiver, thread::JoinHandle, time::Duration};

use egui_extras::{Column, TableBuilder};
use rumqttc::{Client, Event, Incoming, MqttOptions};

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
        // Put your widgets into a `SidePanel`, `TopBottomPanel`, `CentralPanel`, `Window` or `Area`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // The top panel is often a good place for a menu bar:

            egui::menu::bar(ui, |ui| {
                // NOTE: no File->Quit on web pages!
                let is_web = cfg!(target_arch = "wasm32");
                if !is_web {
                    ui.menu_button("File", |ui| {
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.add_space(0.0);
                }

                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            // The central panel the region left after adding TopPanel's and SidePanel's
            let heading = if self.mqtt_client.is_none() {
                "Connect to MQTT"
            } else {
                "MQTT Connected!"
            };
            ui.heading(heading);

            ui.horizontal(|ui| {
                ui.label("Host: ");
                ui.text_edit_singleline(&mut self.host);
            });

            ui.horizontal(|ui| {
                ui.label("Port: ");
                ui.text_edit_singleline(&mut self.port);
                if !self.manager.is_connected() && ui.button("connect").clicked() {
                    let port = self.port.parse::<u16>().unwrap_or(1883);
                    if let Err(e) = self.manager.connect(&self.host, port) {
                        log::error!("Error connecting: {}", e.to_string())
                    } else {
                        self.manager.subscribe();
                    };
                } else if self.manager.is_connected() {
                    if ui.button("disconnect").clicked() {
                        self.manager.disconnect();
                    }
                }
            });
            for event in self.manager.pull_events() {
                self.incoming.push(event);
            }

            ui.separator();

            if self.manager.is_connected() {
                ui.horizontal(|ui| {
                    ui.label("Topic: ");
                    ui.text_edit_singleline(&mut self.send_topic);
                    ui.label("Payload: ");
                    ui.text_edit_singleline(&mut self.send_payload);
                    if ui.button("send").clicked() {
                        self.manager
                            .publish(self.send_topic.clone(), self.send_payload.clone());
                    }
                });
                TableBuilder::new(ui)
                    .column(Column::auto().at_least(50.0).resizable(true))
                    .column(Column::remainder())
                    .striped(true)
                    .resizable(true)
                    .header(20.0, |mut header| {
                        header.col(|ui| {
                            ui.heading("Topic");
                        });
                        header.col(|ui| {
                            ui.heading("Payload");
                        });
                    })
                    .body(|mut body| {
                        for event in &self.incoming {
                            let topic = event.event.topic.clone();
                            let payload = String::from_utf8(event.event.payload.to_vec()).unwrap();
                            body.row(30.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(topic);
                                });
                                row.col(|ui| {
                                    ui.label(payload);
                                });
                            });
                        }
                    });
            }
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
