//! Where the node's signing key lives.
//!
//! Injected rather than hardcoded, for the reason recorded as AD-20 in the L3
//! ledger: a component that can only talk to the real OS keychain cannot be
//! proven in CI, and a Linux runner has no Secret Service at all.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("credential store backend failed: {0}")]
    Backend(String),
}

/// Reads and writes one secret per account name.
///
/// Absence and failure are different answers: `Ok(None)` means the key was
/// never minted, `Err` means the store could not be consulted. Collapsing the
/// second into the first is how a locked keychain gets reported as a fresh
/// install — the failure mode AD-25 named in the connector plane.
pub trait KeyStore {
    fn store(&self, account: &str, secret: &str) -> Result<(), KeyStoreError>;
    fn load(&self, account: &str) -> Result<Option<String>, KeyStoreError>;
    fn delete(&self, account: &str) -> Result<(), KeyStoreError>;
}

/// Non-durable store, for tests and for a node that must not persist a key.
///
/// Nothing in the shipped runtime path uses this: a node whose key vanishes on
/// exit cannot be re-identified by a peer.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    entries: Mutex<HashMap<String, String>>,
}

impl MemoryKeyStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for MemoryKeyStore {
    fn store(&self, account: &str, secret: &str) -> Result<(), KeyStoreError> {
        self.entries
            .lock()
            .map_err(|error| KeyStoreError::Backend(error.to_string()))?
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, KeyStoreError> {
        Ok(self
            .entries
            .lock()
            .map_err(|error| KeyStoreError::Backend(error.to_string()))?
            .get(account)
            .cloned())
    }

    fn delete(&self, account: &str) -> Result<(), KeyStoreError> {
        self.entries
            .lock()
            .map_err(|error| KeyStoreError::Backend(error.to_string()))?
            .remove(account);
        Ok(())
    }
}

/// The OS credential store, through the same wrapper the connector plane uses.
pub struct VaultKeyStore {
    vault: heiwa_vault::Vault,
}

impl VaultKeyStore {
    pub fn new() -> Self {
        Self {
            vault: heiwa_vault::Vault::new(crate::node::KEY_STORE_SERVICE),
        }
    }
}

impl Default for VaultKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyStore for VaultKeyStore {
    fn store(&self, account: &str, secret: &str) -> Result<(), KeyStoreError> {
        self.vault
            .store(account, secret)
            .map_err(|error| KeyStoreError::Backend(error.to_string()))
    }

    fn load(&self, account: &str) -> Result<Option<String>, KeyStoreError> {
        absent_is_none(self.vault.load(account))
    }

    fn delete(&self, account: &str) -> Result<(), KeyStoreError> {
        match self.vault.delete(account) {
            Ok(()) | Err(heiwa_vault::VaultError::NotFound { .. }) => Ok(()),
            Err(error) => Err(KeyStoreError::Backend(error.to_string())),
        }
    }
}

/// The one piece of `VaultKeyStore` that is a decision rather than delegation.
pub(crate) fn absent_is_none(
    result: heiwa_vault::Result<String>,
) -> Result<Option<String>, KeyStoreError> {
    match result {
        Ok(secret) => Ok(Some(secret)),
        Err(heiwa_vault::VaultError::NotFound { .. }) => Ok(None),
        Err(error) => Err(KeyStoreError::Backend(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_that_was_never_stored_reads_as_absent() {
        let absent = Err(heiwa_vault::VaultError::NotFound {
            service: "heiwa-mesh".to_string(),
            account: "sha256:whatever".to_string(),
        });
        assert!(matches!(absent_is_none(absent), Ok(None)));
    }

    #[test]
    fn a_backend_failure_is_a_failure_not_an_absence() {
        let broken = Err(heiwa_vault::VaultError::InvalidUtf8);
        assert!(
            matches!(absent_is_none(broken), Err(KeyStoreError::Backend(_))),
            "a locked or broken keychain must never be reported as a fresh install"
        );
    }

    #[test]
    fn a_stored_secret_reads_back() {
        assert_eq!(
            absent_is_none(Ok("secret".to_string())).expect("load"),
            Some("secret".to_string())
        );
    }
}
