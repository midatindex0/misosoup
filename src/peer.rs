use std::collections::HashMap;

use actix::{Actor, ActorContext, AsyncContext, Handler, StreamHandler};
use actix_web_actors::ws;
use event_listener_primitives::HandlerId;
use mediasoup::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{message::*, vc::Vc};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd, Deserialize, Serialize)]
pub struct PeerId(String);

struct Transports {
    consumer: WebRtcTransport,
    producer: WebRtcTransport,
}

pub struct PeerConnection {
    id: PeerId,
    client_rtp_capabilities: Option<RtpCapabilities>,
    consumers: HashMap<ConsumerId, Consumer>,
    producers: Vec<Producer>,
    transports: Transports,
    vc: Vc,
    attached_handlers: Vec<HandlerId>,
}

impl Drop for PeerConnection {
    fn drop(&mut self) {
        self.vc.remove_peer(&self.id);
    }
}

impl PeerConnection {
    pub async fn new(vc: Vc, peer_id: impl Into<String>) -> Result<Self, String> {
        let transport_options =
            WebRtcTransportOptions::new(WebRtcTransportListenInfos::new(ListenInfo {
                protocol: Protocol::Udp,
                ip: std::env::var("IP")
                    .expect("IP environment variable not set")
                    .parse()
                    .expect("Invalid ip"),
                port: None,
                announced_ip: std::env::var("ANNOUNCED_IP")
                    .ok()
                    .map(|x| x.parse().expect("Invalid announced ip")),
                send_buffer_size: None,
                recv_buffer_size: None,
            }));
        let producer_transport = vc
            .router()
            .create_webrtc_transport(transport_options.clone())
            .await
            .map_err(|error| format!("Failed to create producer transport: {error}"))?;

        let consumer_transport = vc
            .router()
            .create_webrtc_transport(transport_options)
            .await
            .map_err(|error| format!("Failed to create consumer transport: {error}"))?;

        Ok(Self {
            id: PeerId(peer_id.into()),
            client_rtp_capabilities: None,
            consumers: HashMap::new(),
            producers: vec![],
            transports: Transports {
                consumer: consumer_transport,
                producer: producer_transport,
            },
            vc,
            attached_handlers: Vec::new(),
        })
    }
}

impl Actor for PeerConnection {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let server_init_message = S2C::Init {
            vc_id: self.vc.id(),
            consumer_transport_options: TransportOptions {
                id: self.transports.consumer.id(),
                dtls_parameters: self.transports.consumer.dtls_parameters(),
                ice_candidates: self.transports.consumer.ice_candidates().clone(),
                ice_parameters: self.transports.consumer.ice_parameters().clone(),
            },
            producer_transport_options: TransportOptions {
                id: self.transports.producer.id(),
                dtls_parameters: self.transports.producer.dtls_parameters(),
                ice_candidates: self.transports.producer.ice_candidates().clone(),
                ice_parameters: self.transports.producer.ice_parameters().clone(),
            },
            router_rtp_capabilities: self.vc.router().rtp_capabilities().clone(),
        };
        let address = ctx.address();
        address.do_send(server_init_message);

        for peer_id in self.vc.get_all_peers() {
            address.do_send(S2C::Notification(Notification::PeerJoin { peer_id }));
        }

        self.vc.add_peer(self.id.clone());

        self.attached_handlers.push(self.vc.on_notification({
            let own_peer_id = self.id.clone();
            let address = address.clone();

            move |notification| {
                if let Some(peer_id) = notification.associated_peer_id() {
                    if peer_id == &own_peer_id {
                        return;
                    }
                }
                address.do_send(S2C::Notification(notification.clone()));
            }
        }));

        self.attached_handlers.push(self.vc.on_echo({
            let own_peer_id = self.id.clone();
            let address = address.clone();

            move |peer_id, text| {
                if &own_peer_id == peer_id {
                    return;
                }
                address.do_send(S2C::Echo {
                    peer_id: peer_id.clone(),
                    text: text.clone(),
                });
            }
        }));

        self.attached_handlers.push(self.vc.on_producer_add({
            let own_peer_id = self.id.clone();
            let address = address.clone();

            move |peer_id, producer| {
                if &own_peer_id == peer_id {
                    return;
                }
                address.do_send(S2C::ProducerAdd {
                    peer_id: peer_id.clone(),
                    producer_id: producer.id(),
                });
            }
        }));

        self.attached_handlers.push(self.vc.on_producer_remove({
            let own_peer_id = self.id.clone();
            let address = address.clone();

            move |peer_id, producer_id| {
                if &own_peer_id == peer_id {
                    return;
                }
                address.do_send(S2C::ProducerRemove {
                    peer_id: peer_id.clone(),
                    producer_id: *producer_id,
                });
            }
        }));

        for (peer_id, producer_id) in self.vc.get_all_producers() {
            address.do_send(S2C::ProducerAdd {
                peer_id,
                producer_id,
            });
        }
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        dbg!(
            "[peer_id {:?}] WebSocket connection closed",
            self.id.clone()
        );
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for PeerConnection {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {}
            Ok(ws::Message::Text(text)) => match serde_json::from_str::<C2S>(&text) {
                Ok(message) => {
                    ctx.address().do_send(message);
                }
                Err(error) => {
                    eprintln!("Failed to parse client message: {error}\n{text}");
                }
            },
            Ok(ws::Message::Binary(bin)) => {
                eprintln!("Unexpected binary message: {bin:?}");
            }
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

impl Handler<C2S> for PeerConnection {
    type Result = ();

    fn handle(&mut self, message: C2S, ctx: &mut Self::Context) -> Self::Result {
        match message {
            C2S::Init { rtp_capabilities } => {
                self.client_rtp_capabilities.replace(rtp_capabilities);
            }
            C2S::ConnectProducerTransport { dtls_parameters } => {
                let address = ctx.address();
                let transport = self.transports.producer.clone();

                actix::spawn(async move {
                    match transport
                        .connect(WebRtcTransportRemoteParameters { dtls_parameters })
                        .await
                    {
                        Ok(_) => {
                            address.do_send(S2C::ConnectedProducerTransport);
                        }
                        Err(error) => {
                            eprintln!("Failed to connect producer transport: {error}");
                            address.do_send(InternalMessage::Stop);
                        }
                    }
                });
            }
            C2S::Produce {
                kind,
                rtp_parameters,
            } => {
                let peer_id = self.id.clone();
                let address = ctx.address();
                let transport = self.transports.producer.clone();
                let vc = self.vc.clone();
                actix::spawn(async move {
                    match transport
                        .produce(ProducerOptions::new(kind, rtp_parameters))
                        .await
                    {
                        Ok(producer) => {
                            let id = producer.id();
                            address.do_send(S2C::ProducerCreated { id });
                            vc.add_producer(peer_id, producer.clone());
                            address.do_send(InternalMessage::SaveProducer(producer));
                        }
                        Err(error) => {
                            eprintln!("{}", error);
                            address.do_send(InternalMessage::Stop);
                        }
                    }
                });
            }
            C2S::ProducerRemove { producer_id } => self.vc.remove_producer(&self.id, &producer_id),

            C2S::ConnectConsumerTransport { dtls_parameters } => {
                let peer_id = self.id.clone();
                let address = ctx.address();
                let transport = self.transports.consumer.clone();

                actix::spawn(async move {
                    match transport
                        .connect(WebRtcTransportRemoteParameters { dtls_parameters })
                        .await
                    {
                        Ok(_) => {
                            address.do_send(S2C::ConnectedConsumerTransport);
                            println!("[peer_id {peer_id:?}] Consumer transport connected");
                        }
                        Err(error) => {
                            eprintln!(
                                "[peer_id {peer_id:?}] Failed to connect consumer transport: {error}"
                            );
                            address.do_send(InternalMessage::Stop);
                        }
                    }
                });
            }
            C2S::Consume { producer_id } => {
                let peer_id = self.id.clone();
                let address = ctx.address();
                let transport = self.transports.consumer.clone();
                let rtp_capabilities = match self.client_rtp_capabilities.clone() {
                    Some(rtp_capabilities) => rtp_capabilities,
                    None => {
                        eprintln!(
                            "[peer_id {peer_id:?}] Client should send RTP capabilities before \
                            consuming"
                        );
                        return;
                    }
                };
                actix::spawn(async move {
                    let mut options = ConsumerOptions::new(producer_id, rtp_capabilities);
                    options.paused = true;

                    match transport.consume(options).await {
                        Ok(consumer) => {
                            let id = consumer.id();
                            let kind = consumer.kind();
                            let rtp_parameters = consumer.rtp_parameters().clone();
                            address.do_send(S2C::ConsumerCreated {
                                id,
                                producer_id,
                                kind,
                                rtp_parameters,
                            });
                            address.do_send(InternalMessage::SaveConsumer(consumer));
                            println!("[peer_id {peer_id:?}] {kind:?} consumer created: {id}");
                        }
                        Err(error) => {
                            eprintln!("[peer_id {peer_id:?}] Failed to create consumer: {error}");
                            address.do_send(InternalMessage::Stop);
                        }
                    }
                });
            }
            C2S::ConsumerResume { id } => {
                if let Some(consumer) = self.consumers.get(&id).cloned() {
                    let peer_id = self.id.clone();
                    actix::spawn(async move {
                        match consumer.resume().await {
                            Ok(_) => {
                                println!(
                                    "[peer_id {:?}] Successfully resumed {:?} consumer {}",
                                    peer_id,
                                    consumer.kind(),
                                    consumer.id(),
                                );
                            }
                            Err(error) => {
                                println!(
                                    "[peer_id {:?}] Failed to resume {:?} consumer {}: {}",
                                    peer_id,
                                    consumer.kind(),
                                    consumer.id(),
                                    error,
                                );
                            }
                        }
                    });
                }
            }
            C2S::Echo { text } => self.vc.echo(&self.id, &text),
            C2S::Notification { kind } => self.vc.notify(&self.id, &kind),
        }
    }
}

impl Handler<S2C> for PeerConnection {
    type Result = ();

    fn handle(&mut self, message: S2C, ctx: &mut Self::Context) {
        let msg_string = serde_json::to_string(&message).unwrap();
        ctx.text(msg_string);
    }
}

impl Handler<InternalMessage> for PeerConnection {
    type Result = ();

    fn handle(&mut self, message: InternalMessage, ctx: &mut Self::Context) {
        match message {
            InternalMessage::Stop => {
                ctx.stop();
            }
            InternalMessage::SaveProducer(producer) => {
                self.producers.push(producer);
            }
            InternalMessage::SaveConsumer(consumer) => {
                self.consumers.insert(consumer.id(), consumer);
            }
        }
    }
}
