// SPDX-License-Identifier: AGPL-3.0-only

//! Starts sign-in and saves its state on the node.

use fileos_core::Error;

use super::{Inner, Node, lock};

impl Node {
    /// Starts sign-in for people, for a gateway on `port`. The person is
    /// the identity of the node (its DID), or, with no identity, the node
    /// key as a `did:key`.
    ///
    /// # Errors
    ///
    /// When the sign-in state on disk is broken.
    pub fn start_sign_in(&self, port: u16) -> Result<(), Error> {
        let inner = self.inner();
        let state = match inner.store.auth()? {
            Some(bytes) => fileos_core::decode(&bytes)?,
            None => fileos_auth::AuthState::default(),
        };
        let auth = fileos_auth::Auth::new(
            inner.key.clone(),
            self.person_locked(&inner),
            format!("http://fileos.localhost:{port}"),
            "fileos.localhost".into(),
            state,
            inner.store.seed()?,
        );
        drop(inner);
        *lock(&self.shared.auth) = Some(auth);
        Ok(())
    }

    fn person_locked(&self, inner: &Inner) -> String {
        inner
            .identity(self.key())
            .and_then(|id| id.did)
            .unwrap_or_else(|| fileos_id::did_key(self.key()))
    }

    /// Runs `f` on the sign-in state, and keeps it on disk after. The
    /// person is brought up to date first, as the node may have joined an
    /// identity.
    ///
    /// # Errors
    ///
    /// When sign-in is not started, `f` fails, or the state cannot be kept.
    pub fn sign_in<T>(
        &self,
        f: impl FnOnce(&mut fileos_auth::Auth) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let person = self.person_locked(&self.inner());
        let mut guard = lock(&self.shared.auth);
        let auth = guard
            .as_mut()
            .ok_or_else(|| Error::unavailable("sign-in is not started".into()))?;
        if auth.sub() != person {
            auth.set_sub(person);
        }
        let before = auth.state().clone();
        let result = f(auth);
        if *auth.state() != before {
            self.inner()
                .store
                .set_auth(&fileos_core::encode(auth.state()))?;
        }
        result
    }
}
