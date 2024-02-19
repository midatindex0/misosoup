use async_lock::Mutex;
use mediasoup::prelude::*;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;

use crate::vc::{Vc, VcId, WeakVc};

#[derive(Default, Clone)]
pub struct VcRegistry {
    vcs: Arc<Mutex<HashMap<VcId, WeakVc>>>,
}

impl VcRegistry {
    pub async fn get_or_create_vc(
        &self,
        worker_manager: &WorkerManager,
        vc_id: VcId,
    ) -> Result<Vc, String> {
        let mut vcs = self.vcs.lock().await;
        match vcs.entry(vc_id.clone()) {
            Entry::Occupied(mut entry) => match entry.get().upgrade() {
                Some(vc) => Ok(vc),
                None => {
                    let vc = Vc::new(worker_manager, vc_id).await?;
                    entry.insert(vc.downgrade());
                    vc.on_close({
                        let vc_id = vc.id();
                        let vcs = Arc::clone(&self.vcs);

                        move || {
                            std::thread::spawn(move || {
                                futures_lite::future::block_on(async move {
                                    vcs.lock().await.remove(&vc_id);
                                });
                            });
                        }
                    })
                    .detach();
                    Ok(vc)
                }
            },
            Entry::Vacant(entry) => {
                let vc = Vc::new(worker_manager, vc_id).await?;
                entry.insert(vc.downgrade());
                vc.on_close({
                    let vc_id = vc.id();
                    let vcs = Arc::clone(&self.vcs);

                    move || {
                        std::thread::spawn(move || {
                            futures_lite::future::block_on(async move {
                                vcs.lock().await.remove(&vc_id);
                            });
                        });
                    }
                })
                .detach();
                Ok(vc)
            }
        }
    }
}
