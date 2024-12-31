use std::{sync::mpsc::Receiver, thread::JoinHandle, time::Duration};

use egui_extras::{Column, TableBuilder};
use rumqttc::{Client, Event, Incoming, MqttOptions};

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
    mqtt_client: Option<Client>,
    #[serde(skip)]
    mqtt_jh: Option<JoinHandle<()>>,
    #[serde(skip)]
    mqtt_receiver: Option<Receiver<Event>>,
    #[serde(skip)]
    incoming: Vec<Event>,
}

impl Default for TemplateApp {
    fn default() -> Self {
        Self {
            // Example stuff:
            host: "".to_owned(),
            port: "".to_owned(),
            send_payload: "".to_owned(),
            send_topic: "".to_owned(),
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
                if self.mqtt_jh.is_none() && ui.button("connect").clicked() {
                    let port = self.port.parse::<u16>().unwrap_or(1883);
                    let rand = fastrand::u16(0..u16::MAX);
                    let options =
                        MqttOptions::new(format!("oldqtt_{}", rand), self.host.as_str(), port);
                    log::debug!("clicked connect");
                    let (client, mut connection) = Client::new(options, 20);
                    self.mqtt_client = Some(client);
                    log::debug!("Connected to {}", self.host);
                    let (tx, rx) = std::sync::mpsc::channel::<rumqttc::Event>();
                    let ctx_c = ctx.clone();
                    self.mqtt_jh = Some(std::thread::spawn(move || {
                        // loop over notifications
                        for notification in connection.iter() {
                            match notification {
                                Err(error) => {
                                    log::error!("MQTT Error: {:?}", error);
                                    return ();
                                }
                                Ok(event) => {
                                    if let Err(e) = tx.send(event) {
                                        log::error!("Channel Error: {}", e);
                                    };
                                    ctx_c.request_repaint_after(Duration::from_millis(10));
                                }
                            }
                        }
                    }));
                    self.mqtt_receiver = Some(rx);
                    self.mqtt_client
                        .as_ref()
                        .unwrap()
                        .subscribe("#", rumqttc::QoS::ExactlyOnce)
                        .unwrap();
                } else if self.mqtt_jh.is_some() {
                    if ui.button("disconnect").clicked() {
                        let handle = self.mqtt_jh.take();
                        let client = self.mqtt_client.take();
                        if let (Some(client), Some(handle)) = (client, handle) {
                            client.disconnect().unwrap();
                            handle.join().unwrap();
                            self.incoming.clear();
                        }
                    }
                }
            });
            if let Some(receiver) = &self.mqtt_receiver {
                while let Some(event) = receiver.try_recv().ok() {
                    if let Event::Incoming(_) = event {
                        log::debug!("Add event to table: {:?}", event);
                        self.incoming.push(event);
                    }
                }
            }

            ui.separator();

            if self.mqtt_jh.is_some() {
                ui.horizontal(|ui| {
                    ui.label("Topic: ");
                    ui.text_edit_singleline(&mut self.send_topic);
                    ui.label("Payload: ");
                    ui.text_edit_singleline(&mut self.send_payload);
                    if ui.button("send").clicked() {
                        if let Some(client) = &self.mqtt_client {
                            client
                                .publish(
                                    &self.send_topic,
                                    rumqttc::QoS::ExactlyOnce,
                                    false,
                                    self.send_payload.as_str(),
                                )
                                .unwrap();
                        }
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
                            if let Event::Incoming(Incoming::Publish(publish)) = event {
                                let topic = publish.topic.clone();
                                let payload = String::from_utf8(publish.payload.to_vec()).unwrap();
                                body.row(30.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(topic);
                                    });
                                    row.col(|ui| {
                                        ui.label(payload);
                                    });
                                });
                            }
                        }
                    });
            }
        });

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                powered_by_egui_and_eframe(ui);
                egui::warn_if_debug_build(ui);
                ui.separator();
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
