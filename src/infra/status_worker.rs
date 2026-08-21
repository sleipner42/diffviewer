use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;

use crate::application::review_service::{RepoSnapshot, ReviewService};

pub enum WorkerEvent {
    Snapshot(RepoSnapshot),
    Failed(String),
}

pub struct StatusWorker {
    commands: Sender<WorkerCommand>,
    events: Receiver<WorkerEvent>,
}

enum WorkerCommand {
    Refresh,
}

impl StatusWorker {
    pub fn spawn(service: Arc<ReviewService>, interval: Duration, wake: Box<dyn Fn() + Send>) -> Self {
        let (command_tx, command_rx) = channel();
        let (event_tx, event_rx) = channel();
        thread::spawn(move || run_loop(service, interval, command_rx, event_tx, wake));
        Self {
            commands: command_tx,
            events: event_rx,
        }
    }

    pub fn request_refresh(&self) {
        let _ = self.commands.send(WorkerCommand::Refresh);
    }

    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        self.events.try_iter().collect()
    }
}

fn run_loop(
    service: Arc<ReviewService>,
    interval: Duration,
    commands: Receiver<WorkerCommand>,
    events: Sender<WorkerEvent>,
    wake: Box<dyn Fn() + Send>,
) {
    let mut last: Option<Result<RepoSnapshot, String>> = None;
    loop {
        match commands.recv_timeout(interval) {
            Err(RecvTimeoutError::Disconnected) => return,
            Ok(WorkerCommand::Refresh) | Err(RecvTimeoutError::Timeout) => {}
        }
        for _ in commands.try_iter() {}
        let current = service.snapshot().map_err(|error| error.to_string());
        if last.as_ref() == Some(&current) {
            continue;
        }
        let event = match &current {
            Ok(snapshot) => WorkerEvent::Snapshot(snapshot.clone()),
            Err(error) => WorkerEvent::Failed(error.clone()),
        };
        if events.send(event).is_err() {
            return;
        }
        wake();
        last = Some(current);
    }
}
