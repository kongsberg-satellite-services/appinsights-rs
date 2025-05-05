use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use crossbeam_queue::SegQueue;
use futures_channel::mpsc::UnboundedSender;
use log::{debug, error, trace, warn};
use tokio::task::JoinHandle;

use crate::{
    channel::{command::Command, state::Worker, TelemetryChannel},
    contracts::Envelope,
    transmitter::Transmitter,
    TelemetryConfig,
};

/// A telemetry channel that stores events exclusively in memory.
pub struct InMemoryChannel {
    items: Arc<SegQueue<Envelope>>,
    // We have to keep the command sender wrapped in a type we can replace under the hood
    // in case of Worker panicks.
    command_sender: Option<Arc<Mutex<UnboundedSender<Command>>>>,
    // If the worker ever ends up in a infinite panic loop, we need to detect in our restart code
    // that a shutdown was requested, and that its unreasonable for us to continue restarting the worker
    shutdown_sender: Option<tokio::sync::oneshot::Sender<()>>,
    join: Option<JoinHandle<()>>,
}

impl InMemoryChannel {
    /// Creates a new instance of in-memory channel and starts a submission routine.
    pub fn new(config: &TelemetryConfig) -> Self {
        let items = Arc::new(SegQueue::new());

        let (command_sender, command_receiver) = futures_channel::mpsc::unbounded();
        let (shutdown_sender, mut shutdown_receiver) = tokio::sync::oneshot::channel();

        let mutex_sender = Arc::new(Mutex::new(command_sender));

        let worker_endpoint = config.endpoint().to_owned();
        let worker_interval = config.interval();
        let worker_items = items.clone();
        let worker_sender = mutex_sender.clone();

        // Create a task that will monitor the inner task that _actually_ run the worker.
        let task = async move {
            let mut receiver = command_receiver;

            // We will loop-execute the inner task, to watch for panics.
            loop {
                let endpoint = worker_endpoint.clone();
                let sender = worker_sender.clone();
                let items = worker_items.clone();

                let inner_task = async move {
                    let worker = Worker::new(Transmitter::new(&endpoint), items, receiver, worker_interval);
                    worker.run().await;
                };

                match tokio::spawn(inner_task).await {
                    Err(e) => {
                        match e.try_into_panic() {
                            Ok(reason) => {
                                let reason = reason.downcast_ref::<&str>().unwrap_or(&"no panic message provided");
                                error!("InMemoryChannel worker panicked: {reason}");
                            }
                            Err(e) => warn!("InMemoryChannel worker shut down unexpectedly with error: {e}"),
                        }

                        if shutdown_receiver.try_recv().is_ok() {
                            // A shutdown was requested after our panicking exit. Respect the shutdown
                            // to avoid a potential inifite restart loop.
                            let remaining_items = worker_items.clone().len();
                            debug!("InMemoryChannel worker is not restarted due to shutdown already requested. There were {remaining_items} envelopes still in queue that will not be transmitted.");
                            break;
                        }
                    }
                    Ok(_) => {
                        debug!("InMemoryChannel worker shut down gracefully");
                        break;
                    }
                };

                // re-initialize states so we can construct a new worker
                let (command_sender, command_receiver) = futures_channel::mpsc::unbounded();
                {
                    // This replaces the "sender" side stored in InMemoryChannel
                    let mut channel = sender.lock().unwrap_or_else(|e| {
                        sender.clear_poison();
                        e.into_inner()
                    });
                    let _ = std::mem::replace(&mut *channel, command_sender);
                }
                receiver = command_receiver;
            }
        };

        let handle = tokio::spawn(task);

        Self {
            items,
            command_sender: Some(mutex_sender),
            shutdown_sender: Some(shutdown_sender),
            join: Some(handle),
        }
    }

    async fn shutdown(&mut self, command: Command) {
        // send shutdown command to restart-worker-wrapper
        if let Some(sender) = self.shutdown_sender.take() {
            let _ = sender.send(());
        }

        // send shutdown command to worker
        if let Some(sender) = self.command_sender.take() {
            let guard = sender.lock().unwrap();
            send_command(&guard, command);
        }

        // wait until worker is finished
        if let Some(handle) = self.join.take() {
            debug!("Shutting down worker");
            handle.await.unwrap();
        }
    }
}

#[async_trait]
impl TelemetryChannel for InMemoryChannel {
    fn send(&self, envelop: Envelope) {
        trace!("Sending telemetry to channel");
        self.items.push(envelop);
    }

    fn flush(&self) {
        if let Some(sender) = &self.command_sender {
            let guard = sender.lock().unwrap();
            send_command(&guard, Command::Flush);
        }
    }

    async fn close(&mut self) {
        self.shutdown(Command::Close).await
    }

    async fn terminate(&mut self) {
        self.shutdown(Command::Terminate).await;
    }
}

fn send_command(sender: &UnboundedSender<Command>, command: Command) {
    debug!("Sending {} command to channel", command);
    if let Err(err) = sender.unbounded_send(command.clone()) {
        warn!("Unable to send {} command to channel: {}", command, err);
    }
}
