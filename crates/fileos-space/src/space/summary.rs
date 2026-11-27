// SPDX-License-Identifier: AGPL-3.0-only

//! The state signed by a checkpoint, including older on-disk forms.

use fileos_core::{Bytes, Hash, PublicKey};
use serde::Serialize;

use super::{Content, Given, Id, Principal, Sealed, Version, Wrapped};

/// The state that a checkpoint signs: the newest versions of every file,
/// the grants in place, and the last entry of each log.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, serde::Deserialize)]
pub struct Summary {
    pub files: Vec<(Id, Vec<Version>)>,
    pub grants: Vec<Given>,
    /// Every group, with its owner.
    pub groups: Vec<(Id, Principal)>,
    /// For each identity: the operations of its DID log, and the node
    /// certificates that are still good.
    #[serde(default)]
    pub identities: Vec<(Hash, Vec<Bytes>, Vec<Bytes>)>,
    /// Node revocations that are good.
    #[serde(default)]
    pub revocations: Vec<Bytes>,
    /// For each author: its last entry and the place of it.
    pub logs: Vec<(PublicKey, Hash, u64)>,
    /// The newest epochs.
    #[serde(default)]
    pub epochs: Vec<Hash>,
    /// The sealed keys that the newest versions and the newest epochs still
    /// need. Keys that nothing needs are left out, so content that is gone
    /// cannot be opened again (crypto-shredding).
    #[serde(default)]
    pub sealed: Vec<Sealed>,
    /// The data keys of files, wrapped again under the newest epochs.
    #[serde(default)]
    pub rewrap: Vec<(Id, Wrapped)>,
    /// The newest recipient key of each device, with the time it was made.
    #[serde(default)]
    pub recipients: Vec<(PublicKey, Bytes, u64)>,
}

impl Summary {
    /// The hash in checkpoints made by this version.
    pub fn hash(&self) -> Hash {
        Hash::of_value(self)
    }

    pub(super) fn matches_hash(&self, expected: Hash) -> bool {
        self.hash() == expected
            || [
                Format::Encryption,
                Format::Revocations,
                Format::Identities,
                Format::Basic,
            ]
            .into_iter()
            .any(|format| self.legacy_hash(format) == Some(expected))
    }

    fn legacy_hash(&self, format: Format) -> Option<Hash> {
        if !self.recipients.is_empty()
            || self.sealed.iter().any(|sealed| sealed.recipient.is_some())
            || (format < Format::Encryption
                && (!self.epochs.is_empty()
                    || !self.sealed.is_empty()
                    || !self.rewrap.is_empty()
                    || self
                        .files
                        .iter()
                        .any(|(_, versions)| versions.iter().any(|v| v.key.is_some()))))
            || (format < Format::Revocations && !self.revocations.is_empty())
            || (format < Format::Identities && !self.identities.is_empty())
        {
            return None;
        }
        // A field left out of an old hash must have its default value.
        // Keep the old field order because it is part of the signed bytes.
        Some(Hash::of_value(&OldSummary {
            files: self
                .files
                .iter()
                .map(|(id, versions)| {
                    (
                        *id,
                        versions
                            .iter()
                            .map(|v| OldVersion {
                                id: v.id,
                                name: &v.name,
                                content: v.content,
                                author: v.author,
                                time: v.time,
                                parents: &v.parents,
                                key: (format >= Format::Encryption).then_some(v.key.as_ref()),
                            })
                            .collect(),
                    )
                })
                .collect(),
            grants: &self.grants,
            groups: &self.groups,
            identities: (format >= Format::Identities).then_some(&self.identities),
            revocations: (format >= Format::Revocations).then_some(&self.revocations),
            logs: &self.logs,
            epochs: (format >= Format::Encryption).then_some(&self.epochs),
            sealed: (format >= Format::Encryption).then(|| {
                self.sealed
                    .iter()
                    .map(|s| OldSealed {
                        to: s.to,
                        refs: &s.refs,
                        enc: &s.enc,
                        keys: &s.keys,
                    })
                    .collect()
            }),
            rewrap: (format >= Format::Encryption).then_some(&self.rewrap),
        }))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Format {
    Basic,
    Identities,
    Revocations,
    Encryption,
}

type Identities = [(Hash, Vec<Bytes>, Vec<Bytes>)];

#[derive(Serialize)]
struct OldSummary<'a> {
    files: Vec<(Id, Vec<OldVersion<'a>>)>,
    grants: &'a [Given],
    groups: &'a [(Id, Principal)],
    #[serde(skip_serializing_if = "Option::is_none")]
    identities: Option<&'a Identities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revocations: Option<&'a [Bytes]>,
    logs: &'a [(PublicKey, Hash, u64)],
    #[serde(skip_serializing_if = "Option::is_none")]
    epochs: Option<&'a [Hash]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sealed: Option<Vec<OldSealed<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rewrap: Option<&'a [(Id, Wrapped)]>,
}

#[derive(Serialize)]
struct OldVersion<'a> {
    id: Id,
    name: &'a str,
    content: Option<Content>,
    author: PublicKey,
    time: u64,
    parents: &'a [Id],
    #[expect(
        clippy::option_option,
        reason = "an absent field and a null key have different bytes"
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<Option<&'a Wrapped>>,
}

#[derive(Serialize)]
struct OldSealed<'a> {
    to: PublicKey,
    refs: &'a [crate::KeyRef],
    enc: &'a Bytes,
    keys: &'a Bytes,
}
