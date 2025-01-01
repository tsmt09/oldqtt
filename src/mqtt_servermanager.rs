use std::{
    error::Error,
    sync::{
        mpsc::{Receiver, Sender},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

use rumqttc::{Client, ConnectionError, Event, Incoming, Iter, MqttOptions, Outgoing, Publish};

pub struct MqttServerManagerEvent {
    pub event: Publish,
    pub server: u16,
}

pub struct MqttServerManager {
    servers: Arc<Mutex<Vec<Server>>>,
    channel_tx: Sender<MqttServerManagerEvent>,
    channel_rx: Receiver<MqttServerManagerEvent>,
}

impl MqttServerManager {
    pub fn new() -> Self {
        let servers = Arc::new(Mutex::new(vec![]));
        let (channel_tx, channel_rx) = std::sync::mpsc::channel::<MqttServerManagerEvent>();
        MqttServerManager {
            servers,
            channel_tx,
            channel_rx,
        }
    }

    pub fn connect<S>(&self, host: S, port: u16) -> Result<(), Box<dyn Error + '_>>
    where
        S: Into<String> + Clone,
    {
        let server = Server::connect(host, port, self.channel_tx.clone());
        self.servers.lock()?.push(server);
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.servers
            .lock()
            .and_then(|servers| Ok(!servers.is_empty()))
            .unwrap_or(false)
    }

    pub fn disconnect(&self) {
        if let Ok(mut servers) = self.servers.lock() {
            for server in servers.iter() {
                server.client().disconnect().unwrap();
            }
            servers.clear();
        }
    }

    pub fn pull_events(&self) -> Vec<MqttServerManagerEvent> {
        let mut events = vec![];
        while let Ok(event) = self.channel_rx.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn subscribe(&self) {
        if let Ok(servers) = self.servers.lock() {
            for server in servers.iter() {
                server.client().subscribe("#", rumqttc::QoS::ExactlyOnce);
            }
        }
    }
    pub fn publish(&self, topic: String, payload: String) {
        if let Ok(servers) = self.servers.lock() {
            for server in servers.iter() {
                server.client().publish(
                    topic.clone(),
                    rumqttc::QoS::ExactlyOnce,
                    false,
                    payload.clone(),
                );
            }
        }
    }
}

struct Server {
    id: u16,
    host: String,
    port: u16,
    client: rumqttc::Client,
    handle: JoinHandle<()>,
    channel: Sender<MqttServerManagerEvent>,
}

impl Server {
    fn connect<S>(host: S, port: u16, channel: Sender<MqttServerManagerEvent>) -> Self
    where
        S: Into<String> + Clone,
    {
        let rand = fastrand::u16(0..u16::MAX);
        let options = MqttOptions::new(format!("oldqtt_{}", rand), host.clone(), port);
        let (client, connection) = Client::new(options, 20);
        let client_c = client.clone();
        let channel_c = channel.clone();
        let handle = std::thread::spawn(move || {
            Self::poll_iter(connection, client_c, channel_c);
        });
        Server {
            id: rand,
            host: host.into(),
            port,
            client,
            channel,
            handle,
        }
    }

    fn poll_iter(
        mut connection: rumqttc::Connection,
        client: rumqttc::Client,
        channel: Sender<MqttServerManagerEvent>,
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
                            server: 0,
                        }) {
                            log::error!("Error sending event to channel: {}", error)
                        };
                    }
                    Err(e) => {
                        log::error!("connection error: {}", e);
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

    fn id(&self) -> u16 {
        self.id
    }
}
