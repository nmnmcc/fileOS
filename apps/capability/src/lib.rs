// SPDX-License-Identifier: AGPL-3.0-only

//! The default fileOS engine: capabilities.
//!
//! - The key that made a folder may do everything in it.
//! - Other keys may do what a grant gives them, on a path and the paths under it.
//! - A key with `grant` on a path may give others the abilities it has
//!   there, but never more.
//! - When a grant is taken back, the grants that depended on it go too.
//!
//! Policy entries are lines of text:
//!
//! ```text
//! grant <key> <ability>[,<ability>…] <path>
//! revoke <key> <path>
//! ```
//!
//! A key is 64 hex characters. The abilities are `read`, `write`, `append`,
//! `grant`, `store` and `all`.

use fos_app::engine::exports::fos::app::authz::Guest;
use fos_app::engine::fos::app::types::{Ability as WitAbility, Decision, Op};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Ability {
    Read,
    Write,
    Append,
    Grant,
    Store,
}

const ALL: [Ability; 5] = [Ability::Read, Ability::Write, Ability::Append, Ability::Grant, Ability::Store];

impl From<WitAbility> for Ability {
    fn from(a: WitAbility) -> Self {
        match a {
            WitAbility::Read => Self::Read,
            WitAbility::Write => Self::Write,
            WitAbility::Append => Self::Append,
            WitAbility::Grant => Self::Grant,
            WitAbility::Store => Self::Store,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Grant {
    by: Vec<u8>,
    to: Vec<u8>,
    abilities: Vec<Ability>,
    prefix: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct State {
    creator: Vec<u8>,
    grants: Vec<Grant>,
}

impl State {
    fn load(bytes: &[u8]) -> Self {
        cbor4ii::serde::from_slice(bytes).unwrap_or_default()
    }

    fn save(&self) -> Vec<u8> {
        cbor4ii::serde::to_vec(Vec::new(), self).unwrap_or_default()
    }

    fn may(&self, key: &[u8], ability: Ability, path: &str) -> bool {
        key == self.creator.as_slice()
            || self
                .grants
                .iter()
                .any(|g| g.to == key && g.abilities.contains(&ability) && covers(&g.prefix, path))
    }

    fn keys_with(&self, ability: Ability) -> Vec<Vec<u8>> {
        let mut keys = vec![self.creator.clone()];
        for g in &self.grants {
            if g.abilities.contains(&ability) && g.prefix == "/" && !keys.contains(&g.to) {
                keys.push(g.to.clone());
            }
        }
        keys
    }

    fn apply(&mut self, author: &[u8], line: &str) -> Result<(), String> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["grant", to, abilities, prefix] => {
                let to = hex(to)?;
                let abilities = parse_abilities(abilities)?;
                check_prefix(prefix)?;
                if !self.may(author, Ability::Grant, prefix) {
                    return Err(format!("no grant ability on {prefix}"));
                }
                if let Some(a) = abilities.iter().find(|a| !self.may(author, **a, prefix)) {
                    return Err(format!("cannot give {a:?} on {prefix}: the author does not have it"));
                }
                let grant = Grant { by: author.to_vec(), to, abilities, prefix: (*prefix).to_string() };
                if !self.grants.contains(&grant) {
                    self.grants.push(grant);
                }
                Ok(())
            }
            ["revoke", to, prefix] => {
                let to = hex(to)?;
                check_prefix(prefix)?;
                if !self.may(author, Ability::Grant, prefix) {
                    return Err(format!("no grant ability on {prefix}"));
                }
                self.grants.retain(|g| !(g.to == to && covers(prefix, &g.prefix)));
                self.drop_orphans();
                Ok(())
            }
            _ => Err(format!("not a policy line: {line:?}")),
        }
    }

    /// Takes away grants whose giver no longer has what they gave.
    fn drop_orphans(&mut self) {
        loop {
            let before = self.grants.len();
            let snapshot = self.clone();
            self.grants.retain(|g| {
                g.by == snapshot.creator
                    || (snapshot.may_without(&g.by, Ability::Grant, &g.prefix, g)
                        && g.abilities.iter().all(|a| snapshot.may_without(&g.by, *a, &g.prefix, g)))
            });
            if self.grants.len() == before {
                return;
            }
        }
    }

    fn may_without(&self, key: &[u8], ability: Ability, path: &str, skip: &Grant) -> bool {
        key == self.creator.as_slice()
            || self.grants.iter().any(|g| {
                g != skip && g.to == key && g.abilities.contains(&ability) && covers(&g.prefix, path)
            })
    }
}

/// Says if a grant on `prefix` covers `path`.
fn covers(prefix: &str, path: &str) -> bool {
    prefix == "/"
        || path == prefix
        || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

fn check_prefix(prefix: &str) -> Result<(), String> {
    if prefix.starts_with('/') && (prefix == "/" || !prefix.ends_with('/')) {
        Ok(())
    } else {
        Err(format!("not a path: {prefix:?}"))
    }
}

fn parse_abilities(list: &str) -> Result<Vec<Ability>, String> {
    let mut out = Vec::new();
    for name in list.split(',') {
        match name {
            "all" => out.extend(ALL),
            "read" => out.push(Ability::Read),
            "write" => out.push(Ability::Write),
            "append" => out.push(Ability::Append),
            "grant" => out.push(Ability::Grant),
            "store" => out.push(Ability::Store),
            _ => return Err(format!("not an ability: {name:?}")),
        }
    }
    out.dedup();
    Ok(out)
}

fn hex(text: &str) -> Result<Vec<u8>, String> {
    let bad = || format!("not a key of 64 hex characters: {text:?}");
    if text.len() != 64 || !text.is_ascii() {
        return Err(bad());
    }
    (0..32)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| bad()))
        .collect()
}

struct Engine;

impl Guest for Engine {
    fn init(_folder: String, creator: Vec<u8>) -> Vec<u8> {
        State { creator, grants: Vec::new() }.save()
    }

    fn apply(state: Vec<u8>, author: Vec<u8>, policy: Vec<u8>) -> Result<Vec<u8>, String> {
        let mut state = State::load(&state);
        let text = String::from_utf8(policy).map_err(|_| "a policy entry is text".to_string())?;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            state.apply(&author, line)?;
        }
        Ok(state.save())
    }

    fn check(state: Vec<u8>, principal: Vec<u8>, op: Op) -> Decision {
        let ability = Ability::from(op.ability);
        if State::load(&state).may(&principal, ability, &op.path) {
            Decision::Allow
        } else {
            Decision::Deny(format!("no capability for {ability:?} on {}", op.path))
        }
    }

    fn readers(state: Vec<u8>) -> Vec<Vec<u8>> {
        State::load(&state).keys_with(Ability::Read)
    }

    fn storers(state: Vec<u8>) -> Vec<Vec<u8>> {
        State::load(&state).keys_with(Ability::Store)
    }
}

fos_app::engine::export_engine!(Engine);

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 32] = [1; 32];
    const B: [u8; 32] = [2; 32];
    const C: [u8; 32] = [3; 32];

    fn h(k: [u8; 32]) -> String {
        k.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn grants_and_revokes() {
        let mut s = State { creator: A.to_vec(), grants: Vec::new() };
        assert!(s.may(&A, Ability::Write, "/x"));
        assert!(!s.may(&B, Ability::Read, "/x"));

        s.apply(&A, &format!("grant {} read,grant,store /photos", h(B))).unwrap();
        assert!(s.may(&B, Ability::Read, "/photos/a.jpg"));
        assert!(!s.may(&B, Ability::Read, "/photosx"));
        assert!(!s.may(&B, Ability::Write, "/photos/a.jpg"));

        // B may give on only what it has.
        s.apply(&B, &format!("grant {} read /photos/2026", h(C))).unwrap();
        assert!(s.may(&C, Ability::Read, "/photos/2026/a.jpg"));
        assert!(s.apply(&B, &format!("grant {} write /photos", h(C))).is_err());
        assert!(s.apply(&C, &format!("grant {} read /photos", h(B))).is_err());

        // Taking B's grant back takes C's grant, which came from B, too.
        s.apply(&A, &format!("revoke {} /photos", h(B))).unwrap();
        assert!(!s.may(&B, Ability::Read, "/photos/a.jpg"));
        assert!(!s.may(&C, Ability::Read, "/photos/2026/a.jpg"));
        assert!(s.grants.is_empty());
    }

    #[test]
    fn storers_are_keys_with_store_on_everything() {
        let mut s = State { creator: A.to_vec(), grants: Vec::new() };
        s.apply(&A, &format!("grant {} store /", h(B))).unwrap();
        s.apply(&A, &format!("grant {} store /x", h(C))).unwrap();
        assert_eq!(s.keys_with(Ability::Store), vec![A.to_vec(), B.to_vec()]);
    }
}
