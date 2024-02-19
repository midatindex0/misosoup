use crate::peer::PeerId;
use crate::vc::VcId;
use actix::prelude::*;
use mediasoup::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportOptions {
    pub id: TransportId,
    pub dtls_parameters: DtlsParameters,
    pub ice_candidates: Vec<IceCandidate>,
    pub ice_parameters: IceParameters,
}

#[derive(Serialize, Message)]
#[serde(tag = "action")]
#[rtype(result = "()")]
pub enum S2C {
    #[serde(rename_all = "camelCase")]
    Init {
        vc_id: VcId,
        consumer_transport_options: TransportOptions,
        producer_transport_options: TransportOptions,
        router_rtp_capabilities: RtpCapabilitiesFinalized,
    },

    #[serde(rename_all = "camelCase")]
    ProducerAdd {
        peer_id: PeerId,
        producer_id: ProducerId,
    },

    #[serde(rename_all = "camelCase")]
    ProducerRemove {
        peer_id: PeerId,
        producer_id: ProducerId,
    },

    #[serde(rename_all = "camelCase")]
    PeerJoin {
        peer_id: PeerId,
    },

    #[serde(rename_all = "camelCase")]
    PeerLeave {
        peer_id: PeerId,
    },

    ConnectedProducerTransport,

    #[serde(rename_all = "camelCase")]
    ProducerCreated {
        id: ProducerId,
    },

    ConnectedConsumerTransport,

    #[serde(rename_all = "camelCase")]
    ConsumerCreated {
        id: ConsumerId,
        producer_id: ProducerId,
        kind: MediaKind,
        rtp_parameters: RtpParameters,
    },
}

#[derive(Deserialize, Message)]
#[serde(tag = "action")]
#[rtype(result = "()")]
pub enum C2S {
    #[serde(rename_all = "camelCase")]
    Init { rtp_capabilities: RtpCapabilities },

    #[serde(rename_all = "camelCase")]
    ConnectProducerTransport { dtls_parameters: DtlsParameters },

    #[serde(rename_all = "camelCase")]
    Produce {
        kind: MediaKind,
        rtp_parameters: RtpParameters,
    },

    #[serde(rename_all = "camelCase")]
    ConnectConsumerTransport { dtls_parameters: DtlsParameters },

    #[serde(rename_all = "camelCase")]
    Consume { producer_id: ProducerId },

    #[serde(rename_all = "camelCase")]
    ConsumerResume { id: ConsumerId },
}

#[derive(Message)]
#[rtype(result = "()")]
pub enum InternalMessage {
    SaveProducer(Producer),

    SaveConsumer(Consumer),

    Stop,
}
