// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS network, on libp2p.
//!
//! It only carries [`Msg`] values between nodes. It does not know what they
//! mean: [`fos_sync::Replica`] does. The node key is the libp2p identity, so
//! a [`PublicKey`] and a `PeerId` name the same node.
//!
//! In use: TCP and QUIC, Noise, Yamux, Identify, Ping, mDNS on the home
//! network, relays (Circuit Relay v2), hole punching (DCUtR) and AutoNAT.
//! There is no DHT and no bitswap.

use std::time::Duration;

use fos_core::{Error, PublicKey};
use fos_sync::Msg;
use libp2p::futures::StreamExt as _;
use libp2p::request_response::{self, ProtocolSupport};
use libp2p::swarm::behaviour::toggle::Toggle;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{Multiaddr, PeerId, StreamProtocol, Swarm, autonat, dcutr, identify, identity, mdns, noise, ping, relay, tcp, yamux};
use tokio::sync::mpsc;

/// The libp2p protocol of fileOS sync.
pub const PROTOCOL: &str = "/fileos/sync/0.1.0";

/// The largest message, in bytes. Blobs are sent whole for now.
const MAX_MESSAGE: u64 = 64 << 20;

/// How the network starts.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// Addresses to listen on, for example `/ip4/0.0.0.0/udp/0/quic-v1`.
    pub listen: Vec<Multiaddr>,
    /// Nodes to connect to at the start.
    pub peers: Vec<Multiaddr>,
    /// Find nodes on the home network with mDNS.
    pub mdns: bool,
}

/// Something that happened on the network.
#[derive(Clone, Debug)]
pub enum Event {
    Listening(Multiaddr),
    Connected(PublicKey),
    Disconnected(PublicKey),
    Message { from: PublicKey, msg: Msg },
}

/// What the node asks the network to do.
#[derive(Clone, Debug)]
pub enum Command {
    Send { to: PublicKey, msg: Msg },
    Dial(Multiaddr),
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    mdns: Toggle<mdns::tokio::Behaviour>,
    relay: relay::client::Behaviour,
    dcutr: dcutr::Behaviour,
    autonat: autonat::Behaviour,
    sync: request_response::cbor::Behaviour<Msg, bool>,
}

/// The `PeerId` of a node key.
///
/// # Panics
///
/// Never: every 32-byte value is taken as an ed25519 public key here.
pub fn peer_id(key: PublicKey) -> Option<PeerId> {
    let key = identity::ed25519::PublicKey::try_from_bytes(&key.0).ok()?;
    Some(identity::PublicKey::from(key).to_peer_id())
}

/// The node key of a `PeerId`, when it is an ed25519 key.
pub fn node_key(peer: &PeerId) -> Option<PublicKey> {
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
pub fn start(seed: [u8; 32], config: Config) -> Result<(mpsc::UnboundedSender<Command>, mpsc::UnboundedReceiver<Event>), Error> {
    let net_error = |e: &dyn std::fmt::Display| Error(format!("network: {e}"));
    let keypair = identity::Keypair::ed25519_from_bytes(seed).map_err(|e| net_error(&e))?;
    let mdns_on = config.mdns;
    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)
        .map_err(|e| net_error(&e))?
        .with_quic()
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
            let mdns = if mdns_on { mdns::tokio::Behaviour::new(mdns::Config::default(), me).ok() } else { None };
            Behaviour {
                identify: identify::Behaviour::new(identify::Config::new("/fileos/0.1.0".into(), key.public())),
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
        swarm.listen_on(addr.clone()).map_err(|e| Error(format!("cannot listen on {addr}: {e}")))?;
    }
    for addr in config.peers {
        swarm.dial(addr.clone()).map_err(|e| Error(format!("cannot dial {addr}: {e}")))?;
    }

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    tokio::spawn(run(swarm, cmd_rx, event_tx));
    Ok((cmd_tx, event_rx))
}

async fn run(mut swarm: Swarm<Behaviour>, mut commands: mpsc::UnboundedReceiver<Command>, events: mpsc::UnboundedSender<Event>) {
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None => return,
                Some(Command::Send { to, msg }) => {
                    if let Some(peer) = peer_id(to) {
                        swarm.behaviour_mut().sync.send_request(&peer, msg);
                    }
                }
                Some(Command::Dial(addr)) => {
                    if let Err(e) = swarm.dial(addr.clone()) {
                        tracing::warn!("cannot dial {addr}: {e}");
                    }
                }
            },
            event = swarm.select_next_some() => {
                if handle(&mut swarm, event, &events).is_err() {
                    return; // the node stopped
                }
            }
        }
    }
}

fn handle(
    swarm: &mut Swarm<Behaviour>,
    event: SwarmEvent<BehaviourEvent>,
    events: &mpsc::UnboundedSender<Event>,
) -> Result<(), mpsc::error::SendError<Event>> {
    match event {
        SwarmEvent::NewListenAddr { address, .. } => events.send(Event::Listening(address))?,
        SwarmEvent::ConnectionEstablished { peer_id, num_established, .. } if num_established.get() == 1 => {
            if let Some(key) = node_key(&peer_id) {
                events.send(Event::Connected(key))?;
            }
        }
        SwarmEvent::ConnectionClosed { peer_id, num_established: 0, .. } => {
            if let Some(key) = node_key(&peer_id) {
                events.send(Event::Disconnected(key))?;
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(found))) => {
            for (peer, addr) in found {
                if !swarm.is_connected(&peer) {
                    let _ = swarm.dial(addr);
                }
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
            // Keep the addresses a peer says it has, for dialing it again.
            for addr in info.listen_addrs {
                swarm.add_peer_address(peer_id, addr);
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Sync(request_response::Event::Message { peer, message, .. })) => {
            if let request_response::Message::Request { request, channel, .. } = message {
                let _ = swarm.behaviour_mut().sync.send_response(channel, true);
                if let Some(from) = node_key(&peer) {
                    events.send(Event::Message { from, msg: request })?;
                }
            }
        }
        SwarmEvent::Behaviour(BehaviourEvent::Sync(request_response::Event::OutboundFailure { peer, error, .. })) => {
            tracing::debug!("a message to {peer} was lost: {error}");
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_keys_and_peer_ids_match() {
        let key = fos_core::SecretKey::from_seed(&[9; 32]);
        let id = peer_id(key.public()).unwrap();
        let from_libp2p = identity::Keypair::ed25519_from_bytes(key.seed()).unwrap().public().to_peer_id();
        assert_eq!(id, from_libp2p);
        assert_eq!(node_key(&id), Some(key.public()));
    }
}
