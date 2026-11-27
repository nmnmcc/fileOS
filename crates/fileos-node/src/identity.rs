// SPDX-License-Identifier: AGPL-3.0-only

//! Keeps the identity log and the certificates of its nodes.

use std::fmt::{self, Write as _};

use fileos_core::{Error, Hash, PublicKey, SecretKey};
use fileos_id::{Link, NodeCert, Op as DidOp, Service, State};
use fileos_space::{Act, Id, Principal};

use super::{Changed, Inner, Node, READ, clean};

/// Where the identity space is mounted in the home space.
pub const IDENTITY_AT: &str = "/identity";

/// The identity of a node.
#[derive(Clone, Debug)]
pub struct Identity {
    /// The DID of the identity. `None` while the node waits for the
    /// identity space from another node, or when its log is not valid.
    pub did: Option<String>,
    /// The identity space.
    pub space: Hash,
    /// The rotation keys, as `did:key`, the first one with the highest priority.
    pub rotation_keys: Vec<String>,
    /// Other names of the identity.
    pub also_known_as: Vec<String>,
    /// This node holds a rotation key of the identity.
    pub holds_rotation_key: bool,
    /// This node holds the signing key of the identity, so it can add nodes.
    pub holds_signing_key: bool,
    /// The nodes of the identity: the ones with a good certificate.
    pub nodes: Vec<PublicKey>,
    /// The link that people give to others, with the addresses of this node
    /// as hints.
    pub link: Option<String>,
}

impl Node {
    /// Makes a new identity. This node holds its rotation key and its
    /// signing key, and is its first node. Gives the DID.
    ///
    /// # Errors
    ///
    /// When this node already has an identity, or the keys cannot be kept.
    pub fn identity_new(&self) -> Result<String, Error> {
        let mut inner = self.inner();
        if let Some(space) = inner.identity_space() {
            return Err(Error::invalid(format!(
                "this node already has an identity, with the space {space}"
            )));
        }
        // Keys from an earlier try that did not end are used again.
        let rotation = inner.identity_key("rotation")?;
        let signing = inner.identity_key("signing")?;
        let now = (inner.clock)();
        let (space, step) = inner.node.create_as(&signing, now);
        inner.process(step, &self.shared.changed)?;
        inner.mount(space, IDENTITY_AT, &self.shared.changed)?;
        let state = State {
            rotation_keys: vec![fileos_id::did_key(rotation.public())],
            verification_methods: [(
                fileos_id::SIGNING_METHOD.to_string(),
                fileos_id::did_key(signing.public()),
            )]
            .into(),
            also_known_as: Vec::new(),
            services: [(
                fileos_id::FOLDER_SERVICE.to_string(),
                Service {
                    kind: fileos_id::FOLDER_SERVICE_TYPE.to_string(),
                    endpoint: space.to_string(),
                },
            )]
            .into(),
        };
        let genesis = DidOp::genesis(state, &rotation)?;
        let did = genesis.did();
        let path = format!("{IDENTITY_AT}{}", fileos_id::op_path(&genesis));
        inner.put(
            Some(&signing),
            &path,
            &genesis.to_bytes(),
            &self.shared.changed,
        )?;
        inner.add_node(&did, &signing, self.key(), &self.shared.changed)?;
        drop(inner);
        self.pump();
        Ok(did)
    }

    /// Adds a node to the identity: puts its certificate into the identity
    /// space, and lets it read and keep the space. Only the node that holds
    /// the signing key of the identity can do this. Gives the ID of the
    /// identity space, which the other node joins.
    ///
    /// # Errors
    ///
    /// When this node does not hold the signing key of the identity.
    pub fn identity_add_node(&self, node: PublicKey) -> Result<Hash, Error> {
        let mut inner = self.inner();
        let (did, signing, space) = inner.held_identity()?;
        inner.add_node(&did, &signing, node, &self.shared.changed)?;
        drop(inner);
        self.pump();
        Ok(space)
    }

    /// Takes a node away from the identity: deletes its certificate and
    /// takes back its grant on the identity space.
    ///
    /// # Errors
    ///
    /// When this node does not hold the signing key of the identity, or the
    /// node is not one of its nodes.
    pub fn identity_remove_node(&self, node: PublicKey) -> Result<(), Error> {
        let mut inner = self.inner();
        let (did, signing, space) = inner.held_identity()?;
        let path = format!("{IDENTITY_AT}{}", fileos_id::node_path(node));
        inner.delete(Some(&signing), &path, &self.shared.changed)?;
        // The revocation ends the node in every space that counted it as a
        // node of the identity.
        let revocation = fileos_id::NodeRevocation::sign(&did, &signing, node);
        let path = format!("{IDENTITY_AT}{}", fileos_id::revocation_path(node));
        inner.put(
            Some(&signing),
            &path,
            &revocation.to_bytes(),
            &self.shared.changed,
        )?;
        let grants: Vec<Id> = inner
            .node
            .space(space)
            .map(|s| {
                s.grants()
                    .into_iter()
                    .filter(|g| g.given.to == Principal::Key(node))
                    .map(|g| g.given.id)
                    .collect()
            })
            .unwrap_or_default();
        for grant in grants {
            inner.write(
                Some(&signing),
                space,
                vec![Act::Take { grant }],
                &self.shared.changed,
            )?;
        }
        drop(inner);
        self.pump();
        Ok(())
    }

    /// Makes this node one of the nodes of an identity, by the ID of its
    /// space. The node that holds the signing key must add this node first
    /// ([`Node::identity_add_node`]), or the space does not come.
    ///
    /// # Errors
    ///
    /// When this node already has an identity.
    pub fn identity_join(&self, space: Hash) -> Result<(), Error> {
        if let Some(have) = self.inner().identity_space() {
            return Err(Error::invalid(format!(
                "this node already has an identity, with the space {have}"
            )));
        }
        self.mount(space, IDENTITY_AT)
    }

    /// The identity of this node, if it has one.
    pub fn identity(&self) -> Option<Identity> {
        self.inner().identity(self.key())
    }
}

impl Inner {
    pub(super) fn identity_space(&self) -> Option<Hash> {
        self.mounts()
            .into_iter()
            .find(|(at, _)| *at == clean(IDENTITY_AT))
            .map(|(_, id)| id)
    }

    /// A secret key of the identity on this node, by its use. It is made
    /// and kept when this node does not have it yet.
    fn identity_key(&mut self, usage: &str) -> Result<SecretKey, Error> {
        let held = match usage {
            "rotation" => &mut self.rotation_key,
            _ => &mut self.signing_key,
        };
        if let Some(key) = held {
            return Ok(key.clone());
        }
        let key = SecretKey::from_seed(&fileos_store::random_seed()?);
        self.store.set_identity_key(usage, &key)?;
        *held = Some(key.clone());
        if usage == "signing" {
            self.node.add_key(key.clone());
        }
        Ok(key)
    }

    /// The bytes of the files under `dir` in a space, by the rest of their
    /// name.
    fn files_in(&self, space: Hash, dir: &str) -> Vec<(String, Vec<u8>)> {
        let Some(s) = self.node.space(space) else {
            return Vec::new();
        };
        let prefix = format!("{}/", dir.trim_matches('/'));
        s.files()
            .iter()
            .filter_map(|f| {
                let name = f.versions.first()?.name.strip_prefix(&prefix)?.to_string();
                let bytes = self.bytes(space, f)?;
                Some((name, bytes))
            })
            .collect()
    }

    /// The operations of the DID log in the identity space.
    fn identity_ops(&self, space: Hash) -> Vec<DidOp> {
        self.files_in(space, fileos_id::OPS)
            .into_iter()
            .filter_map(|(name, bytes)| {
                let op = DidOp::from_bytes(&bytes).ok()?;
                (op.hash().to_string() == name).then_some(op)
            })
            .collect()
    }

    /// The DID and the state of the identity whose log is in `space`: the
    /// one whose state names this space, and whose signing key owns it.
    fn resolve_identity(&self, space: Hash) -> Option<(String, State)> {
        let owner = self.node.space(space)?.owner();
        let ops = self.identity_ops(space);
        fileos_id::dids(&ops).into_iter().find_map(|did| {
            let state = fileos_id::resolve(&did, &ops).ok()?.state?;
            (state.folder() == Some(space)
                && state.signing_key().map(Principal::Key) == Some(owner))
            .then_some((did, state))
        })
    }

    /// The DID, the signing key and the space of the identity, when this
    /// node holds the signing key.
    fn held_identity(&self) -> Result<(String, SecretKey, Hash), Error> {
        let none =
            || Error::denied("this node does not hold the signing key of an identity".into());
        let signing = self.signing_key.clone().ok_or_else(none)?;
        let space = self.identity_space().ok_or_else(none)?;
        let (did, state) = self.resolve_identity(space).ok_or_else(none)?;
        if state.signing_key() != Some(signing.public()) {
            return Err(none());
        }
        Ok((did, signing, space))
    }

    /// Puts the certificate of `node` into the identity space, and lets the
    /// node read and keep the space.
    fn add_node(
        &mut self,
        did: &str,
        signing: &SecretKey,
        node: PublicKey,
        changed: &Changed,
    ) -> Result<(), Error> {
        let cert = NodeCert::sign(did, signing, node);
        let path = format!("{IDENTITY_AT}{}", fileos_id::node_path(node));
        self.put(Some(signing), &path, &cert.to_bytes(), changed)?;
        if node == signing.public() {
            return Ok(());
        }
        self.grant(
            Some(signing),
            IDENTITY_AT,
            Principal::Key(node),
            &READ,
            changed,
        )
        .map(drop)
    }

    /// The identity of this node. Which nodes belong to it is a fact: the
    /// kernel checks the DID log and the certificates.
    pub(super) fn identity(&self, me: PublicKey) -> Option<Identity> {
        let space = self.identity_space()?;
        let holds = |key: &Option<SecretKey>, keys: &[PublicKey]| {
            key.as_ref().is_some_and(|k| keys.contains(&k.public()))
        };
        let Some((did, state)) = self.resolve_identity(space) else {
            return Some(Identity {
                did: None,
                space,
                rotation_keys: Vec::new(),
                also_known_as: Vec::new(),
                holds_rotation_key: false,
                holds_signing_key: false,
                nodes: Vec::new(),
                link: None,
            });
        };
        let signing = state.signing_key();
        let nodes = self
            .files_in(space, fileos_id::NODES)
            .into_iter()
            .filter_map(|(name, bytes)| {
                let cert = NodeCert::from_bytes(&bytes).ok()?;
                (cert.did == did
                    && signing.is_some_and(|key| cert.verify(key))
                    && cert.node.to_string() == name)
                    .then_some(cert.node)
            })
            .collect::<Vec<_>>();
        // Other nodes find this one by its addresses only if it is one of
        // the nodes of the identity.
        let hints = if nodes.contains(&me) {
            self.listening.clone()
        } else {
            Vec::new()
        };
        Some(Identity {
            link: Some(
                Link {
                    did: did.clone(),
                    hints,
                }
                .to_string(),
            ),
            did: Some(did),
            space,
            holds_rotation_key: holds(
                &self.rotation_key,
                &state.rotation_keys().unwrap_or_default(),
            ),
            holds_signing_key: holds(&self.signing_key, &signing.into_iter().collect::<Vec<_>>()),
            rotation_keys: state.rotation_keys,
            also_known_as: state.also_known_as,
            nodes,
        })
    }

    /// When this node is a node of its identity, tells the kernel, so that a
    /// grant to the identity is a grant to this node too.
    pub(super) fn update_proof(&mut self, changed: &Changed) -> Result<(), Error> {
        let Some(space) = self.identity_space() else {
            return Ok(());
        };
        let revocations: Vec<fileos_core::Bytes> = self
            .files_in(space, fileos_id::REVOKED)
            .into_iter()
            .map(|(_, bytes)| fileos_core::Bytes(bytes))
            .collect();
        if !revocations.is_empty() {
            let step = self.node.add_revocations(revocations);
            self.process(step, changed)?;
        }
        let me = self.key.public().to_string();
        let ops: Vec<fileos_core::Bytes> = self
            .files_in(space, fileos_id::OPS)
            .into_iter()
            .map(|(_, bytes)| fileos_core::Bytes(bytes))
            .collect();
        let Some(cert) = self
            .files_in(space, fileos_id::NODES)
            .into_iter()
            .find(|(name, _)| *name == me)
            .map(|(_, bytes)| fileos_core::Bytes(bytes))
        else {
            return Ok(());
        };
        match self.node.set_identity(ops, cert) {
            Ok(step) => self.process(step, changed),
            // Not a good certificate (yet, or any more): nothing to show.
            Err(_) => Ok(()),
        }
    }
}

/// Reads a principal: a node or app key in hex, or a `did:fileos` DID for a
/// person and all their nodes.
///
/// # Errors
///
/// When the text is neither.
pub fn parse_principal(text: &str) -> Result<Principal, Error> {
    if text.starts_with("did:") {
        return fileos_id::did_hash(text).map(Principal::Identity);
    }
    text.parse().map(Principal::Key)
}
impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        match &self.did {
            Some(did) => {
                let _ = writeln!(out, "did      {did}");
            }
            None => {
                let _ = writeln!(
                    out,
                    "did      (waiting for the identity space: the node that holds the \
                     signing key must add this node, and be connected)"
                );
            }
        }
        let _ = writeln!(out, "space    {}", self.space);
        if let Some(link) = &self.link {
            let _ = writeln!(out, "link     {link}");
        }
        for name in &self.also_known_as {
            let _ = writeln!(out, "aka      {name}");
        }
        for (i, key) in self.rotation_keys.iter().enumerate() {
            let _ = writeln!(out, "rotation {key} (priority {})", i + 1);
        }
        if self.holds_rotation_key {
            let _ = writeln!(out, "         this node holds a rotation key");
        }
        if self.holds_signing_key {
            let _ = writeln!(out, "         this node holds the signing key");
        }
        for node in &self.nodes {
            let _ = writeln!(out, "node     {node}");
        }
        f.write_str(&out)
    }
}
