// SPDX-License-Identifier: AGPL-3.0-only

//! Batch access events on a dedicated writer thread.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

const QUEUE_CAPACITY: usize = 1024;
const BATCH_SIZE: usize = 64;

#[derive(Serialize)]
pub(crate) struct RequestEvent {
    #[serde(rename = "event.name")]
    pub(crate) name: &'static str,
    #[serde(rename = "message.template")]
    pub(crate) template: &'static str,
    pub(crate) time: u64,
    #[serde(rename = "http.request.method")]
    pub(crate) method: &'static str,
    #[serde(rename = "http.route")]
    pub(crate) route: &'static str,
    #[serde(rename = "http.response.status_code")]
    pub(crate) status: u16,
    pub(crate) ms: u128,
}

enum Message {
    Request(RequestEvent),
    Flush(oneshot::Sender<()>),
}

#[derive(Default)]
pub(crate) struct AccessLog {
    sender: OnceLock<mpsc::Sender<Message>>,
}

impl AccessLog {
    pub(crate) async fn record(&self, path: impl FnOnce() -> PathBuf, event: RequestEvent) {
        let sender = self.sender.get_or_init(|| {
            let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
            let path = path();
            if let Err(error) = std::thread::Builder::new()
                .name("hs-access-log".into())
                .spawn(move || {
                    write_events(path, rx);
                })
            {
                tracing::warn!(name: "http.access_log.start_failed", %error, "Could not start access event writer");
            }
            tx
        });
        let _ = sender.send(Message::Request(event)).await;
    }

    pub(crate) async fn flush(&self) {
        if let Some(sender) = self.sender.get() {
            let (ack, written) = oneshot::channel();
            if sender.send(Message::Flush(ack)).await.is_ok() {
                let _ = written.await;
            }
        }
    }
}

fn write_events(path: PathBuf, mut rx: mpsc::Receiver<Message>) {
    let mut file = None;
    let mut unavailable = false;
    let mut batch = Vec::with_capacity(BATCH_SIZE);
    let mut bytes = Vec::new();
    while rx.blocking_recv_many(&mut batch, BATCH_SIZE) != 0 {
        let written = append(&path, &mut file, &batch, &mut bytes);
        if let Err(error) = written {
            file = None;
            if !unavailable {
                tracing::warn!(name: "http.access_log.write_failed", %error,
                    "Could not write access events; the writer will try again on the next batch");
            }
            unavailable = true;
        } else if unavailable {
            tracing::info!(name: "http.access_log.recovered", "Access log writer recovered");
            unavailable = false;
        }
        for message in batch.drain(..) {
            if let Message::Flush(ack) = message {
                let _ = ack.send(());
            }
        }
    }
}

fn append(
    path: &std::path::Path,
    file: &mut Option<File>,
    batch: &[Message],
    bytes: &mut Vec<u8>,
) -> io::Result<()> {
    bytes.clear();
    for message in batch {
        if let Message::Request(event) = message {
            serde_json::to_writer(&mut *bytes, event)?;
            bytes.push(b'\n');
        }
    }
    if bytes.is_empty() {
        return Ok(());
    }
    let file = match file {
        Some(file) => file,
        None => file.insert(OpenOptions::new().create(true).append(true).open(path)?),
    };
    file.write_all(bytes)
}
