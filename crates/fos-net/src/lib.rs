// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS network, on libp2p.
//!
//! It only carries [`Msg`] values between nodes. It does not know what they
//! mean: [`fos_space::Node`] does. The node key is the libp2p identity, so
//! a [`PublicKey`] and a `PeerId` name the same node.
//!
//! In use: TCP, Noise, Yamux, Identify, Ping, mDNS on the home
//! network, relays (Circuit Relay v2), hole punching (`DCUtR`) and `AutoNAT`.
//! There is no DHT and no bitswap.

use std::collections::HashMap;
use std::time::Duration;

use fos_core::{Error, Hash, PublicKey};
use fos_space::Msg;
use libp2p::futures::StreamExt as _;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, Swarm, autonat, dcutr, identify, identity, mdns, noise,
    ping, relay, tcp, yamux,
};
use tokio::sync::mpsc;

/// The libp2p protocol of fileOS sync.
pub const PROTOCOL: &str = "/fileos/sync/0.1.0";

/// The largest message, in bytes. A message may carry a batch of entries.
const MAX_MESSAGE: u64 = 64 << 20;
/// Keep queues short because a sync message can hold many entries.
const QUEUE_CAPACITY: usize = 8;

/// How the network starts.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// Addresses to listen on, for example `/ip4/0.0.0.0/tcp/0`.
    pub listen: Vec<String>,
    /// Nodes to connect to at the start.
    pub peers: Vec<String>,
    /// Find nodes on the home network with mDNS.
    pub mdns: bool,
}

/// Something that happened on the network.
#[derive(Clone, Debug)]
pub enum Event {
    Listening(String),
    Connected(PublicKey),
    Disconnected(PublicKey),
    SendFailed {
        to: PublicKey,
        content: Option<Hash>,
    },
    Message {
        from: PublicKey,
        msg: Box<Msg>,
    },
}

/// What the node asks the network to do.
#[derive(Clone, Debug)]
pub enum Command {
    Send { to: PublicKey, msg: Box<Msg> },
    Dial(Address),
}

/// An address on the network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address(Multiaddr);

impl std::str::FromStr for Address {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        text.parse::<Multiaddr>()
            .map(Self)
            .map_err(|e| Error::invalid(format!("not a multiaddr: {e}")))
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The command side of a running network.
#[derive(Clone, Debug)]
pub struct Commands(mpsc::Sender<Command>);

impl Commands {
    /// Queues a command for the network.
    ///
    /// # Errors
    ///
    /// When the queue is full or the network has stopped.
    pub fn send(&self, command: Command) -> Result<(), Error> {
        self.0
            .try_send(command)
            .map_err(|e| Error::unavailable(format!("network command: {e}")))
    }
}

/// The event side of a running network.
#[derive(Debug)]
pub struct Events(mpsc::Receiver<Event>);

impl Events {
    /// Waits for the next event. Returns `None` when the network stops.
    pub async fn recv(&mut self) -> Option<Event> {
        self.0.recv().await
    }
}

mod behaviour {
    // The code that the derive makes has a match on an event type with no
    // values.
    #![allow(unreachable_code, reason = "made by the NetworkBehaviour derive")]

    use super::{
        Msg, NetworkBehaviour, Toggle, autonat, dcutr, identify, mdns, ping, relay,
        request_response,
    };

    #[derive(NetworkBehaviour)]
    pub(super) struct Behaviour {
        pub identify: identify::Behaviour,
        pub ping: ping::Behaviour,
        pub mdns: Toggle<mdns::tokio::Behaviour>,
        pub relay: relay::client::Behaviour,
        pub dcutr: dcutr::Behaviour,
        pub autonat: autonat::Behaviour,
        pub sync: request_response::cbor::Behaviour<Msg, bool>,
    }
}

use behaviour::{Behaviour, BehaviourEvent};

/// The libp2p peer ID of a node key, as text.
pub fn peer_id(key: PublicKey) -> Option<String> {
    peer_for(key).map(|id| id.to_string())
}

fn peer_for(key: PublicKey) -> Option<PeerId> {
    let key = identity::ed25519::PublicKey::try_from_bytes(&key.0).ok()?;
    Some(identity::PublicKey::from(key).to_peer_id())
}

fn node_key(peer: &PeerId) -> Option<PublicKey> {
    let multihash = peer.as_ref();
    // An ed25519 PeerId is the "identity" multihash of the public key.
    if multihash.code() != 0 {
        return None;
    }
    let key = identity::PublicKey::try_decode_protobuf(multihash.digest()).ok()?;
    Some(PublicKey(key.try_into_ed25519().ok()?.to_bytes()))
}

/// Starts the network. Gives a channel for commands and one for events.
///
/// # Errors
///
/// When a transport or a listen address does not work.
pub fn start(seed: [u8; 32], config: Config) -> Result<(Commands, Events), Error> {
    let net_error = |e: &dyn std::fmt::Display| Error::invalid(format!("network: {e}"));
    let keypair = identity::Keypair::ed25519_from_bytes(seed).map_err(|e| net_error(&e))?;
    let mdns_on = config.mdns;
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|e| net_error(&e))?
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|e| net_error(&e))?
        .with_behaviour(|key, relay| {
            let me = key.public().to_peer_id();
            let sync = request_response::cbor::Behaviour::with_codec(
                request_response::cbor::codec::Codec::default()
                    .set_request_size_maximum(MAX_MESSAGE)
                    .set_response_size_maximum(1024),
                [(StreamProtocol::new(PROTOCOL), ProtocolSupport::Full)],
                request_response::Config::default().with_request_timeout(Duration::from_secs(60)),
            );
            let mdns = if mdns_on {
                mdns::tokio::Behaviour::new(mdns::Config::default(), me).ok()
            } else {
                None
            };
            Behaviour {
                identify: identify::Behaviour::new(identify::Config::new(
                    "/fileos/0.1.0".into(),
                    key.public(),
                )),
                ping: ping::Behaviour::default(),
                mdns: Toggle::from(mdns),
                relay,
                dcutr: dcutr::Behaviour::new(me),
                autonat: autonat::Behaviour::new(me, autonat::Config::default()),
                sync,
            }
        })
        .map_err(|e| net_error(&e))?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(600)))
        .build();

    for addr in config.listen {
        swarm
            .listen_on(addr.parse::<Address>()?.0)
            .map_err(|e| Error::invalid(format!("cannot listen on {addr}: {e}")))?;
    }
    for addr in config.peers {
        swarm
            .dial(addr.parse::<Address>()?.0)
            .map_err(|e| Error::invalid(format!("cannot dial {addr}: {e}")))?;
    }

    let (cmd_tx, cmd_rx) = mpsc::channel(QUEUE_CAPACITY);
    let (event_tx, event_rx) = mpsc::channel(QUEUE_CAPACITY);
    tokio::spawn(run(swarm, cmd_rx, event_tx));
    Ok((Commands(cmd_tx), Events(event_rx)))
}

async fn run(
    mut swarm: Swarm<Behaviour>,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
) {
    let mut requests = HashMap::new();
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None => return,
                Some(Command::Send { to, msg }) => {
                    if let Some(peer) = peer_for(to) {
                        let content = match msg.as_ref() {
                            Msg::WantContent(hash) => Some(*hash),
                            _ => None,
                        };
                        let id = swarm.behaviour_mut().sync.send_request(&peer, *msg);
                        if let Some(hash) = content {
                            requests.insert(id, hash);
                        }
                    }
                }
                Some(Command::Dial(addr)) => {
                    let text = addr.to_string();
                    if let Err(e) = swarm.dial(addr.0) {
                        tracing::warn!(name: "network.dial.failed", address = %text, error = %e, "could not dial node");
                    }
                }
            },
            event = swarm.select_next_some() => {
                if let Some(event) = handle(&mut swarm, event, &mut requests)
                    && events.send(event).await.is_err()
                {
                    return;
                }
            }
        }
    }
}

/// Handles a swarm event and gives any event for the node.
fn handle(
    swarm: &mut Swarm<Behaviour>,
    event: SwarmEvent<BehaviourEvent>,
    requests: &mut HashMap<request_response::OutboundRequestId, Hash>,
) -> Option<Event> {
    let event = match event {
        SwarmEvent::NewListenAddr { address, .. } => Event::Listening(address.to_string()),
        SwarmEvent::ConnectionEstablished {
            peer_id,
            num_established,
            ..
        } if num_established.get() == 1 => {
            let key = node_key(&peer_id)?;
            Event::Connected(key)
        }
        SwarmEvent::ConnectionClosed {
            peer_id,
            num_established: 0,
            ..
        } => {
            let key = node_key(&peer_id)?;
            Event::Disconnected(key)
        }
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(found))) => {
            for (peer, addr) in found {
                if !swarm.is_connected(&peer) {
                    let _ = swarm.dial(addr);
                }
            }
            return None;
        }
        SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
            peer_id,
            info,
            ..
        })) => {
            // Keep the addresses a peer says it has, for dialing it again.
            for addr in info.listen_addrs {
                swarm.add_peer_address(peer_id, addr);
            }
            return None;
        }
        SwarmEvent::Behaviour(BehaviourEvent::Sync(request_response::Event::Message {
            peer,
            message:
                request_response::Message::Request {
                    request, channel, ..
                },
            ..
        })) => {
            let _ = swarm.behaviour_mut().sync.send_response(channel, true);
            let from = node_key(&peer)?;
            Event::Message {
                from,
                msg: Box::new(request),
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Sync(request_response::Event::Message {
            message: request_response::Message::Response { request_id, .. },
            ..
        })) => {
            requests.remove(&request_id);
            return None;
        }
        SwarmEvent::Behaviour(BehaviourEvent::Sync(request_response::Event::OutboundFailure {
            peer,
            error,
            request_id,
            ..
        })) => {
            tracing::debug!(name: "network.send.failed", %peer, %error, "message lost");
            let content = requests.remove(&request_id);
            Event::SendFailed {
                to: node_key(&peer)?,
                content,
            }
        }
        _ => return None,
    };
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_keys_and_peer_ids_match() {
        let key = fos_core::SecretKey::from_seed(&[9; 32]);
        let id = peer_for(key.public()).unwrap();
        let from_libp2p = identity::Keypair::ed25519_from_bytes(key.seed())
            .unwrap()
            .public()
            .to_peer_id();
        assert_eq!(id, from_libp2p);
        assert_eq!(node_key(&id), Some(key.public()));
    }
}
