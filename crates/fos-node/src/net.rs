// SPDX-License-Identifier: AGPL-3.0-only

//! Runs the network and passes its events to the node in order.

use std::sync::Arc;
use std::time::Duration;

use fos_core::Error;

use super::{Inner, Node};

/// Where to listen, whom to dial, and whether to find local nodes.
#[derive(Clone, Debug, Default)]
pub struct NetConfig {
    pub listen: Vec<String>,
    pub peers: Vec<String>,
    pub mdns: bool,
}

impl Node {
    /// Starts the network. Call it inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// When the network does not start.
    pub fn start_net(&self, config: NetConfig) -> Result<(), Error> {
        let mut inner = self.inner();
        if inner.net.is_some() {
            return Err(Error::invalid("the network is already started".into()));
        }
        let config = fos_net::Config {
            listen: config.listen,
            peers: config.peers,
            mdns: config.mdns,
        };
        let (commands, mut events) = fos_net::start(inner.key.seed(), config)?;
        inner.net = Some(commands);
        let shared = Arc::downgrade(&self.shared);
        tokio::spawn(async move {
            // Missing content is retried after three of these ticks.
            let mut retry = tokio::time::interval(Duration::from_secs(5));
            retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                let event = tokio::select! {
                    event = events.recv() => {
                        let Some(event) = event else { break };
                        Some(event)
                    },
                    _ = retry.tick() => None,
                };
                let Some(shared) = shared.upgrade() else {
                    break;
                };
                let node = Node { shared };
                // One job at a time keeps events in order and provides
                // backpressure without holding up a Tokio worker.
                if let Err(error) = tokio::task::spawn_blocking(move || node.on_net(event)).await {
                    tracing::error!(name: "node.network.worker.failed", %error, "network work stopped");
                    break;
                }
            }
        });
        Ok(())
    }

    /// Connects to another node.
    ///
    /// # Errors
    ///
    /// When the network is not started.
    pub fn dial(&self, addr: &str) -> Result<(), Error> {
        let addr = addr.parse()?;
        let inner = self.inner();
        let net = inner
            .net
            .as_ref()
            .ok_or_else(|| Error::unavailable("the network is not started".into()))?;
        net.send(fos_net::Command::Dial(addr))
    }

    fn on_net(&self, event: Option<fos_net::Event>) {
        {
            let mut inner = self.inner();
            let step = match event {
                Some(fos_net::Event::Listening(addr)) => {
                    inner.listening.push(format!(
                        "{addr}/p2p/{}",
                        fos_net::peer_id(self.key()).unwrap_or_default()
                    ));
                    return;
                }
                Some(fos_net::Event::Connected(peer)) => inner.node.connected(peer),
                Some(fos_net::Event::Disconnected(peer)) => {
                    inner.node.disconnected(peer);
                    return;
                }
                Some(fos_net::Event::Message { from, msg }) => {
                    let Inner { node, store, .. } = &mut *inner;
                    node.receive(from, *msg, store)
                }
                Some(fos_net::Event::SendFailed { to, content }) => {
                    inner.send_failed(to, content);
                    return;
                }
                None => {
                    let Inner {
                        node,
                        store,
                        net_dirty,
                        ..
                    } = &mut *inner;
                    if std::mem::take(net_dirty) {
                        let mut step = node.announce();
                        step.out.extend(node.retry_contents(store).out);
                        step
                    } else {
                        node.retry_contents(store)
                    }
                }
            };
            if let Err(e) = inner.process(step, &self.shared.changed) {
                tracing::warn!(name: "node.sync.apply.failed", error = %e, "could not apply sync changes");
            }
        }
        self.pump();
    }
}
