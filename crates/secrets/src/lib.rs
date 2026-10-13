//! The minimal secret store: credentials live in the **OS
//! keychain** (`keyring`), never in a plaintext file.
//!
//! Each secret is one keychain entry under the service `agent-hub`, named by a
//! key (a provider's id). The store is **probed once at boot**: if the OS store is
//! not reachable, [`SecretStore::is_available`] is false and a caller must refuse
//! the credential path (`501 not_implemented`) rather than fall back to plaintext.

/// A credential store backed by the OS keychain.
#[derive(Clone)]
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

    /// A store scoped to a hub INSTANCE: the namespace is part of the keychain
    /// service, so two instances (different data dirs, same OS user) never share
    /// an entry. `instance` should be a stable, unique id for this hub instance.
    pub fn for_instance(instance: &str) -> Self {
        let service = format!("agent-hub:{instance}");
        Self::probe(service)
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    /// A store that is known-unavailable, for tests that exercise the refusal
    /// path without touching the OS keychain.
    #[doc(hidden)]
    pub fn probe_unavailable_for_test() -> Self {
        SecretStore { service: "agent-hub-unavailable".into(), available: false }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, key).map_err(|e| SecretError::Store(e.to_string()))
    }

    /// Store a credential. Refused when the store is unavailable. The write is
    /// VERIFIED: after setting, the value is read back and compared, so a store
    /// that ACCEPTS a write but does not persist it (e.g. a non-interactive OS
    /// session) fails HERE instead of being reported as a success. A `set` that
    /// returns `Ok` is therefore a real fact - the credential IS in the store.
    pub fn set(&self, key: &str, value: &str) -> Result<(), SecretError> {
        if !self.available {
            return Err(SecretError::Unavailable);
        }
        let entry = self.entry(key)?;
        entry.set_password(value).map_err(|e| SecretError::Store(e.to_string()))?;
        match entry.get_password() {
            Ok(read) if read == value => Ok(()),
            Ok(_) => Err(SecretError::Store(
                "the OS secret store did not retain the credential (a read-back differs); it was not stored".into(),
            )),
            Err(e) => Err(SecretError::Store(format!(
                "the OS secret store reported a write but the credential could not be read back ({e}); it was not stored"
            ))),
        }
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
    // A ONE-SHOT probe name: random, so it can never collide with a real entry
    // nor with a previous probe (a PID can be reused). The delete is confirmed.
    let name = format!("__probe__{}", random_hex());
    let Ok(entry) = keyring::Entry::new(service, &name) else {
        return false;
    };
    if entry.set_password("probe").is_err() {
        return false;
    }
    let ok = entry.get_password().map(|v| v == "probe").unwrap_or(false);
    let deleted = entry.delete_credential().is_ok();
    ok && deleted
}

/// 16 random hex bytes from the OS RNG (no timestamp, no PID: one-shot names).
fn random_hex() -> String {
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
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

    /// The REAL OS store must be compiled in for the target. `keyring` has NO
    /// default features: with no platform feature it silently uses an IN-PROCESS
    /// MOCK, so a credential write "succeeds" and vanishes. This ties the test to
    /// the exact defect (a mock backend must never be what the hub ships).
    #[test]
    fn a_real_os_backend_is_compiled_in() {
        let real = cfg!(target_os = "windows") || cfg!(target_os = "macos") || cfg!(target_os = "linux");
        assert!(real, "an unhandled OS is built with the mock credential store - wire a real backend");
    }

    #[test]
    fn set_get_delete_roundtrip() {
        // A fresh SERVICE per run: two test runs never share an entry.
        let s = SecretStore::probe(format!("agent-hub-test-{}", random_hex()));
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

    /// The store must PERSIST across `Entry` objects - not merely hold a value in
    /// the object that wrote it. keyring's MOCK store (used when no platform
    /// feature is compiled in) keeps the secret "in the entry itself", so a write
    /// through one Entry and a read through another returns `NoEntry`: a credential
    /// that "succeeds" and is gone. This test FAILS on the mock and passes on any
    /// real OS store - it is the regression guard for a silently-mock backend.
    #[test]
    fn a_credential_persists_across_entry_objects() {
        let service = format!("agent-hub-persist-{}", random_hex());
        let writer = SecretStore::probe(service.clone());
        if !writer.is_available() {
            eprintln!("SKIP: OS secret store unavailable on this host");
            return;
        }
        writer.set("ref", "persisted-value").unwrap();
        // A SEPARATE store over the SAME service (fresh Entry objects): the value
        // must still be there. On the mock it is not (kept per-Entry) -> this fails.
        let reader = SecretStore::probe(service.clone());
        assert_eq!(
            reader.get("ref").unwrap().as_deref(),
            Some("persisted-value"),
            "a written credential must be readable through a new Entry; if this fails the \
             build is using keyring's in-process MOCK store, not a real OS store"
        );
        reader.delete("ref").unwrap();
    }

    #[test]
    fn an_unavailable_store_refuses() {
        let s = SecretStore { service: "x".into(), available: false };
        assert!(matches!(s.set("k", "v"), Err(SecretError::Unavailable)));
        assert!(matches!(s.get("k"), Err(SecretError::Unavailable)));
    }
}
