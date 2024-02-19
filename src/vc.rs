use std::{collections::HashMap, sync::Arc, sync::Weak};

use event_listener_primitives::{Bag, BagOnce, HandlerId};
use mediasoup::{
    prelude::*,
    worker::{WorkerLogLevel, WorkerLogTag},
};
use parking_lot::Mutex;
use serde::Serialize;

use crate::peer::PeerId;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Hash)]
pub struct VcId(pub String);

#[derive(Default)]
struct Handlers {
    peer_join: Bag<Arc<dyn Fn(&PeerId) + Send + Sync>, PeerId>,
    peer_leave: Bag<Arc<dyn Fn(&PeerId) + Send + Sync>, PeerId>,
    producer_add: Bag<Arc<dyn Fn(&PeerId, &Producer) + Send + Sync>, PeerId, Producer>,
    producer_remove: Bag<Arc<dyn Fn(&PeerId, &ProducerId) + Send + Sync>, PeerId, ProducerId>,
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
                id: id,
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

        self.inner.handlers.peer_join.call_simple(&peer_id);
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

        self.inner.handlers.peer_leave.call_simple(peer_id);
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

    pub fn on_peer_join<F: Fn(&PeerId) + Send + Sync + 'static>(&self, callback: F) -> HandlerId {
        self.inner.handlers.peer_join.add(Arc::new(callback))
    }

    pub fn on_peer_leave<F: Fn(&PeerId) + Send + Sync + 'static>(&self, callback: F) -> HandlerId {
        self.inner.handlers.peer_leave.add(Arc::new(callback))
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
