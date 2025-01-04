use std::{
    collections::{HashMap, HashSet},
    error::Error,
    sync::mpsc::{Receiver, Sender},
    thread::JoinHandle,
};

use egui::Context;
use rumqttc::{Client, Event, Incoming, MqttOptions, Outgoing, Publish};

use crate::app::MqttServer;

#[derive(Clone)]
pub struct MqttServerManagerEvent {
    pub event: Publish,
    pub client: u32,
}

pub struct MqttServerManager {
    servers: HashMap<u32, Server>,
    channel_tx: Sender<MqttServerManagerEvent>,
    channel_rx: Receiver<MqttServerManagerEvent>,
}

impl MqttServerManager {
    pub fn new() -> Self {
        let servers = HashMap::new();
        let (channel_tx, channel_rx) = std::sync::mpsc::channel::<MqttServerManagerEvent>();
        MqttServerManager {
            servers,
            channel_tx,
            channel_rx,
        }
    }

    pub fn servers(&self) -> &HashMap<u32, Server> {
        &self.servers
    }

    pub fn channel(&self) -> Sender<MqttServerManagerEvent> {
        self.channel_tx.clone()
    }

    pub fn servers_mut(&mut self) -> &mut HashMap<u32, Server> {
        &mut self.servers
    }

    pub fn pull_events(&self) -> Vec<MqttServerManagerEvent> {
        let mut events = vec![];
        while let Ok(event) = self.channel_rx.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn connect(&mut self, id: u32, server: &MqttServer, ctx: Context) {
        let port: u16 = server.port();
        let host: String = server.host();
        let mqtt_server = Server::connect(host, port, self.channel(), id, ctx);
        self.servers_mut().insert(id, mqtt_server);
    }

    pub fn disconnect(&mut self, id: u32) {
        if let Some(server) = self.servers_mut().remove(&id) {
            if let Err(e) = server.client().disconnect() {
                log::error!("Cannot disconnect client '{}': '{}'", id, e);
            }
        }
    }
}

pub struct Server {
    id: u32,
    host: String,
    port: u16,
    client: rumqttc::Client,
    handle: JoinHandle<()>,
    channel: Sender<MqttServerManagerEvent>,
    current_subs: HashSet<String>,
}

impl Server {
    pub fn connect<S>(
        host: S,
        port: u16,
        channel: Sender<MqttServerManagerEvent>,
        id: u32,
        ctx: Context,
    ) -> Self
    where
        S: Into<String> + Clone,
    {
        let options = MqttOptions::new(format!("oldqtt_{}", id), host.clone(), port);
        let (client, connection) = Client::new(options, 20);
        let channel_c = channel.clone();
        let handle = std::thread::spawn(move || {
            log::info!("MQTT Event Loop started.");
            Self::poll_iter(connection, channel_c, id, ctx);
            log::info!("MQTT Event Loop ended.");
        });
        Server {
            id,
            host: host.into(),
            port,
            client,
            channel,
            handle,
            current_subs: HashSet::new(),
        }
    }

    fn poll_iter(
        mut connection: rumqttc::Connection,
        channel: Sender<MqttServerManagerEvent>,
        id: u32,
        ctx: Context,
    ) {
        loop {
            for event in connection.iter() {
                match event {
                    Ok(Event::Outgoing(Outgoing::Disconnect)) => {
                        log::info!("disconnect happening, exiting!");
                        return;
                    }
                    Ok(Event::Incoming(Incoming::Publish(message))) => {
                        log::debug!(
                            "published: {} - {}",
                            message.topic,
                            String::from_utf8(message.payload.to_vec())
                                .unwrap_or(String::default())
                        );
                        if let Err(error) = channel.send(MqttServerManagerEvent {
                            event: message,
                            client: id,
                        }) {
                            log::error!("Error sending event to channel: {}", error)
                        };
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        log::error!("mqtt error: {}", e);
                    }
                    _ => {
                        log::debug!("incoming: {:?}", event);
                    }
                }
            }
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn sync_subs(&mut self, subs: &Vec<String>) -> Result<(), Box<dyn Error>> {
        let to_sub: Vec<String> = subs
            .iter()
            .filter(|sub| !self.current_subs.contains(*sub))
            .cloned()
            .collect();
        let to_unsub: Vec<String> = self
            .current_subs
            .iter()
            .filter(|sub| !subs.contains(sub))
            .cloned()
            .collect();
        for sub in to_sub {
            self.subscribe(sub)?;
        }
        for sub in to_unsub {
            self.unsubscribe(sub)?;
        }
        Ok(())
    }

    fn subscribe(&mut self, topic: String) -> Result<(), Box<dyn Error>> {
        if self.current_subs.insert(topic.clone()) {
            log::debug!("Client '{}' subscribing '{}'", self.id, &topic);
            if let Err(e) = self.client().subscribe(topic, rumqttc::QoS::ExactlyOnce) {
                return Err(Box::new(e));
            };
        }
        Ok(())
    }

    fn unsubscribe(&mut self, topic: String) -> Result<(), Box<dyn Error>> {
        if let Some(topic) = self.current_subs.take(&topic) {
            log::debug!("Client '{}' unsubscribing '{}'", self.id, &topic);
            if let Err(e) = self.client().unsubscribe(topic) {
                return Err(Box::new(e));
            };
        }
        Ok(())
    }

    pub fn publish<S, V>(&self, topic: S, payload: V)
    where
        S: Into<String> + std::fmt::Debug,
        V: Into<Vec<u8>> + std::fmt::Debug,
    {
        log::debug!(
            "Client '{}' publishing '{:?}' on topic '{:?}'",
            self.id,
            &payload,
            &topic
        );
        if let Err(e) = self
            .client()
            .publish(topic, rumqttc::QoS::ExactlyOnce, false, payload)
        {
            log::error!("Error publishing: {:?}", e);
        }
    }
}
