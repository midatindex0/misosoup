use std::{collections::HashMap, sync::Arc, sync::Weak};

use event_listener_primitives::{Bag, BagOnce, HandlerId};
use mediasoup::{
    prelude::*,
    worker::{WorkerLogLevel, WorkerLogTag},
};
use parking_lot::Mutex;
use serde::Serialize;

use crate::{
    message::{Notification, NotificationType},
    peer::PeerId,
};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Hash)]
pub struct VcId(pub String);

#[derive(Default)]
struct Handlers {
    notification: Bag<Arc<dyn Fn(&Notification) + Send + Sync>, Notification>,
    producer_add: Bag<Arc<dyn Fn(&PeerId, &Producer) + Send + Sync>, PeerId, Producer>,
    producer_remove: Bag<Arc<dyn Fn(&PeerId, &ProducerId) + Send + Sync>, PeerId, ProducerId>,
    echo: Bag<Arc<dyn Fn(&PeerId, &String) + Send + Sync>, PeerId, String>,
    close: BagOnce<Box<dyn FnOnce() + Send>>,
}

pub struct VcInner {
    id: VcId,
    router: Router,
    handlers: Handlers,
    clients: Mutex<HashMap<PeerId, Vec<Producer>>>,
}

impl Drop for VcInner {
    fn drop(&mut self) {
        dbg!("Vc {:?} closed", self.id.clone());
        self.handlers.close.call_simple();
    }
}

#[derive(Clone)]
pub struct Vc {
    inner: Arc<VcInner>,
}

impl Vc {
    pub async fn new(worker_manager: &WorkerManager, id: VcId) -> Result<Self, String> {
        let worker = worker_manager
            .create_worker({
                let mut settings = WorkerSettings::default();
                settings.log_level = WorkerLogLevel::Debug;
                settings.log_tags = vec![
                    WorkerLogTag::Info,
                    WorkerLogTag::Ice,
                    WorkerLogTag::Dtls,
                    WorkerLogTag::Rtp,
                    WorkerLogTag::Srtp,
                    WorkerLogTag::Rtcp,
                    WorkerLogTag::Rtx,
                    WorkerLogTag::Bwe,
                    WorkerLogTag::Score,
                    WorkerLogTag::Simulcast,
                    WorkerLogTag::Svc,
                    WorkerLogTag::Sctp,
                    WorkerLogTag::Message,
                ];

                settings
            })
            .await
            .map_err(|error| format!("Failed to create worker: {error}"))?;

        let router = worker
            .create_router(RouterOptions::new(crate::media_codecs()))
            .await
            .map_err(|error| format!("Failed to create router: {error}"))?;

        println!("Vc {id:?} created");

        Ok(Self {
            inner: Arc::new(VcInner {
                id,
                router,
                handlers: Handlers::default(),
                clients: Mutex::default(),
            }),
        })
    }

    pub fn id(&self) -> VcId {
        self.inner.id.clone()
    }

    pub fn router(&self) -> &Router {
        &self.inner.router
    }

    pub fn add_peer(&self, peer_id: PeerId) {
        self.inner
            .clients
            .lock()
            .entry(peer_id.clone())
            .or_default();

        self.inner
            .handlers
            .notification
            .call_simple(&Notification::PeerJoin { peer_id });
    }

    pub fn echo(&self, peer_id: &PeerId, text: &String) {
        self.inner.handlers.echo.call_simple(peer_id, text);
    }

    pub fn notify(&self, peer_id: &PeerId, notification: &NotificationType) {
        let notification = match notification {
            NotificationType::Loading => Notification::Loading {
                peer_id: peer_id.clone(),
            },
            NotificationType::Playing => Notification::Playing {
                peer_id: peer_id.clone(),
            },
            NotificationType::Idle => Notification::Idle {
                peer_id: peer_id.clone(),
            },
        };

        self.inner.handlers.notification.call_simple(&notification);
    }

    pub fn add_producer(&self, peer_id: PeerId, producer: Producer) {
        self.inner
            .clients
            .lock()
            .entry(peer_id.clone())
            .or_default()
            .push(producer.clone());

        self.inner
            .handlers
            .producer_add
            .call_simple(&peer_id, &producer);
    }

    pub fn remove_peer(&self, peer_id: &PeerId) {
        let producers = self.inner.clients.lock().remove(peer_id);

        for producer in producers.unwrap_or_default() {
            let producer_id = &producer.id();
            self.inner
                .handlers
                .producer_remove
                .call_simple(peer_id, producer_id);
        }

        self.inner
            .handlers
            .notification
            .call_simple(&Notification::PeerLeave {
                peer_id: peer_id.clone(),
            });
    }

    pub fn remove_producer(&self, peer_id: &PeerId, producer_id: &ProducerId) {
        if let Some(producers) = self.inner.clients.lock().get_mut(peer_id) {
            producers.retain(|p| &p.id() != producer_id);
        }

        self.inner
            .handlers
            .producer_remove
            .call_simple(peer_id, producer_id);
    }

    pub fn get_all_producers(&self) -> Vec<(PeerId, ProducerId)> {
        self.inner
            .clients
            .lock()
            .iter()
            .flat_map(|(peer_id, producers)| {
                producers
                    .iter()
                    .map(move |producer| (peer_id.clone(), producer.id()))
            })
            .collect()
    }

    pub fn get_all_peers(&self) -> Vec<PeerId> {
        self.inner
            .clients
            .lock()
            .keys()
            .map(|t| t.clone())
            .collect()
    }

    pub fn on_notification<F: Fn(&Notification) + Send + Sync + 'static>(
        &self,
        callback: F,
    ) -> HandlerId {
        self.inner.handlers.notification.add(Arc::new(callback))
    }

    pub fn on_producer_add<F: Fn(&PeerId, &Producer) + Send + Sync + 'static>(
        &self,
        callback: F,
    ) -> HandlerId {
        self.inner.handlers.producer_add.add(Arc::new(callback))
    }

    pub fn on_producer_remove<F: Fn(&PeerId, &ProducerId) + Send + Sync + 'static>(
        &self,
        callback: F,
    ) -> HandlerId {
        self.inner.handlers.producer_remove.add(Arc::new(callback))
    }

    pub fn on_echo<F: Fn(&PeerId, &String) + Send + Sync + 'static>(
        &self,
        callback: F,
    ) -> HandlerId {
        self.inner.handlers.echo.add(Arc::new(callback))
    }

    pub fn on_close<F: FnOnce() + Send + 'static>(&self, callback: F) -> HandlerId {
        self.inner.handlers.close.add(Box::new(callback))
    }

    pub fn downgrade(&self) -> WeakVc {
        WeakVc {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

#[derive(Clone)]
pub struct WeakVc {
    inner: Weak<VcInner>,
}

impl WeakVc {
    pub fn upgrade(&self) -> Option<Vc> {
        self.inner.upgrade().map(|inner| Vc { inner })
    }
}
