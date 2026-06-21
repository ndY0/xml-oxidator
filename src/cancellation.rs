use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tokio::sync::OnceCell;
use std::{error::Error, sync::Arc};

#[derive(Clone)]
pub struct ShutdownHandle<E> {
    token: CancellationToken,
    error: Arc<OnceCell<E>>,
}

impl <E: Error + Clone> ShutdownHandle<E> {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
            error: Arc::new(OnceCell::new()),
        }
    }

    pub async fn trigger_fatal(&self, err: E) {
        let _ = self.error.set(err);
        self.token.cancel();
    }

    pub fn is_cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.token.cancelled()
    }
}