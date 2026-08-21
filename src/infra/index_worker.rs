use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use crate::application::review_service::ReviewService;

pub fn spawn_index_build(service: Arc<ReviewService>, wake: Box<dyn Fn() + Send>) -> Receiver<Result<usize, String>> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let result = service.rebuild_symbol_index().map_err(|error| error.to_string());
        let _ = tx.send(result);
        wake();
    });
    rx
}
