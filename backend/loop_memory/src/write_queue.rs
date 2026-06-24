use crate::engine::{MemoryStore, RedbMemoryStore};
use crate::types::MemoryEntry;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

pub struct MemoryWriteQueue {
    tx: mpsc::UnboundedSender<MemoryEntry>,
    pending: Arc<RwLock<Vec<MemoryEntry>>>,
}

impl MemoryWriteQueue {
    pub fn new(store: Arc<RedbMemoryStore>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<MemoryEntry>();
        let pending = Arc::new(RwLock::new(Vec::<MemoryEntry>::new()));
        let pending_for_task = pending.clone();

        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(entry) = rx.try_recv() {
                    batch.push(entry);
                    if batch.len() >= 20 {
                        break;
                    }
                }

                let ids = batch
                    .iter()
                    .map(|entry| entry.id.clone())
                    .collect::<Vec<_>>();
                for entry in batch {
                    if let Err(e) = store.store(entry).await {
                        tracing::error!("[loop_memory] queued write failed: {}", e);
                    }
                }

                let mut pending = pending_for_task.write().await;
                pending.retain(|entry| !ids.contains(&entry.id));
            }
        });

        Self { tx, pending }
    }

    pub async fn enqueue(&self, entry: MemoryEntry) -> Result<(), String> {
        self.pending.write().await.push(entry.clone());
        self.tx
            .send(entry)
            .map_err(|e| format!("memory write queue closed: {}", e))
    }

    pub async fn find_pending(&self, content_substring: &str) -> Vec<MemoryEntry> {
        self.pending
            .read()
            .await
            .iter()
            .filter(|entry| entry.content.contains(content_substring))
            .cloned()
            .collect()
    }
}
