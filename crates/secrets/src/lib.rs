//! The minimal secret store (`ARCHITECTURE` §2): credentials live in the **OS
//! keychain** (`keyring`), never in a plaintext file.
//!
//! Each secret is one keychain entry under the service `agent-hub`, named by a
//! key (a provider's id). The store is **probed once at boot**: if the OS store is
//! not reachable, [`SecretStore::is_available`] is false and a caller must refuse
//! the credential path (`501 not_implemented`) rather than fall back to plaintext.

/// A credential store backed by the OS keychain.
pub struct SecretStore {
    service: String,
    available: bool,
}

impl SecretStore {
    /// Probe the OS store: write, read and delete a throwaway entry. If any step
    /// fails, the store is unavailable and a credential must be refused.
    pub fn probe(service: impl Into<String>) -> Self {
        let service = service.into();
        let available = probe_store(&service);
        if available {
            tracing::info!(%service, "OS secret store is available");
        } else {
            tracing::warn!(%service, "OS secret store is NOT available; credentials are refused");
        }
        SecretStore { service, available }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, key).map_err(|e| SecretError::Store(e.to_string()))
    }

    /// Store a credential. Refused when the store is unavailable.
    pub fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        if !self.available {
            return Err(SecretError::Unavailable);
        }
        self.entry(key)?.set_password(value).map_err(|e| SecretError::Store(e.to_string()))
    }

    /// Read a credential; `None` when none is stored.
    pub fn get(&self, key: &str) -> Result<Option<String>, SecretError> {
        if !self.available {
            return Err(SecretError::Unavailable);
        }
        match self.entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Store(e.to_string())),
        }
    }

    /// Remove a credential; removing a missing one is a no-op success.
    pub fn delete(&self, key: &str) -> Result<(), SecretError> {
        if !self.available {
            return Err(SecretError::Unavailable);
        }
        match self.entry(key)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Store(e.to_string())),
        }
    }

    /// Whether a credential is present.
    pub fn has(&self, key: &str) -> bool {
        matches!(self.get(key), Ok(Some(_)))
    }
}

fn probe_store(service: &str) -> bool {
    let Ok(entry) = keyring::Entry::new(service, "__probe__") else {
        return false;
    };
    if entry.set_password("probe").is_err() {
        return false;
    }
    let ok = entry.get_password().map(|v| v == "probe").unwrap_or(false);
    let _ = entry.delete_credential();
    ok
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the OS secret store is not available; a credential is refused rather than stored in plaintext")]
    Unavailable,
    #[error("secret store: {0}")]
    Store(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_delete_roundtrip() {
        let s = SecretStore::probe("agent-hub-test");
        if !s.is_available() {
            eprintln!("SKIP: OS secret store unavailable on this host");
            return;
        }
        s.set("k", "v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v"));
        assert!(s.has("k"));
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
        // Deleting a missing entry is a success.
        s.delete("k").unwrap();
    }

    #[test]
    fn an_unavailable_store_refuses() {
        let s = SecretStore { service: "x".into(), available: false };
        assert!(matches!(s.set("k", "v"), Err(SecretError::Unavailable)));
        assert!(matches!(s.get("k"), Err(SecretError::Unavailable)));
    }
}
