#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{broadcast, mpsc};

use soulbrowser_core_types::SoulError;

/// Trait implemented by payload types that can be carried on the bus.
pub trait Event: Clone + Send + Sync + std::fmt::Debug + 'static {}

impl<T> Event for T where T: Clone + Send + Sync + std::fmt::Debug + 'static {}

#[async_trait]
pub trait EventBus<E>: Send + Sync
where
    E: Event,
{
    async fn publish(&self, event: E) -> Result<(), SoulError>;
    fn subscribe(&self) -> broadcast::Receiver<E>;
}

/// Simple in-memory bus suitable for unit tests and early integration.
pub struct InMemoryBus<E>
where
    E: Event,
{
    sender: broadcast::Sender<E>,
}

impl<E> InMemoryBus<E>
where
    E: Event,
{
    pub fn new(capacity: usize) -> Arc<Self> {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Arc::new(Self { sender })
    }
}

#[async_trait]
impl<E> EventBus<E> for InMemoryBus<E>
where
    E: Event,
{
    async fn publish(&self, event: E) -> Result<(), SoulError> {
        self.sender
            .send(event)
            .map(|_| ())
            .map_err(|err| SoulError::new(err.to_string()))
    }

    fn subscribe(&self) -> broadcast::Receiver<E> {
        self.sender.subscribe()
    }
}

/// Helper to materialise an mpsc receiver from the bus subscription
/// so callers can await events without handling broadcast semantics directly.
pub fn to_mpsc<E>(bus: Arc<InMemoryBus<E>>, capacity: usize) -> mpsc::Receiver<E>
where
    E: Event,
{
    let mut rx = bus.subscribe();
    let (tx, out_rx) = mpsc::channel(capacity.max(1));
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if tx.send(ev).await.is_err() {
                break;
            }
        }
    });
    out_rx
}
