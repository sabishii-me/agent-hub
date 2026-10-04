//! The minimal secret store (`ARCHITECTURE` §2): credentials live in the **OS
//! keychain** (`keyring`), never in a plaintext file.
//!
//! Each secret is one keychain entry under the fixed service `agent-hub`. The
//! INSTANCE is part of the entry's KEY, not the service: the OS keychain caps the
//! target name and the user name (measured on Windows: target <= 29 chars, user
//! <= 16), and a raw `<instance>:<kind>-<id>` (a 32-hex instance id alone is 32
//! chars) does not fit. So the key is a fixed 16-hex-char digest of the caller's
//! reference string, and the store first tries a LEGACY entry (`agent-hub:<instance>`,
//! raw key) so a credential written by an older build is migrated, not orphaned.
//! The store is **probed once at boot**: if the OS store is not reachable,
//! [`SecretStore::is_available`] is false and a caller must refuse the credential
//! path (`501 not_implemented`) rather than fall back to plaintext.

/// A credential store backed by the OS keychain.
#[derive(Clone)]
pub struct SecretStore {
    /// The keychain SERVICE: always the fixed `agent-hub`. The instance lives in
    /// the key, so the service stays within the OS target-name limit.
    service: String,
    /// The hub instance id, used only to derive the LEGACY entry name for
    /// migration (`agent-hub:<instance>` + the raw key).
    instance: String,
    available: bool,
}

impl SecretStore {
    /// Probe the OS store: write, read and delete a throwaway entry. If any step
    /// fails, the store is unavailable and a credential must be refused.
    /// The one service name every entry lives under. Fixed and short: the OS caps
    /// the target name (measured: 29 chars on Windows), and a raw instance id would
    /// exceed it.
    pub const SERVICE: &'static str = "agent-hub";

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
        SecretStore { service, instance: String::new(), available }
    }

    /// A store for a hub INSTANCE: two instances (different data dirs, same OS
    /// user) never share an entry, because the instance is folded into every key.
    /// `instance` is the persistent hub instance id.
    pub fn for_instance(instance: &str) -> Self {
        let available = probe_store(Self::SERVICE);
        if available {
            tracing::info!(service = Self::SERVICE, "OS secret store is available");
        } else {
            tracing::warn!(service = Self::SERVICE, "OS secret store is NOT available; credentials are refused");
        }
        SecretStore { service: Self::SERVICE.to_string(), instance: instance.to_string(), available }
    }

    /// The bounded keychain KEY (user name) for a caller's reference: a fixed
    /// 16-hex-char digest. Deterministic, so the same reference always maps to the
    /// same entry; collision-resistant enough for credentials (64 bits).
    fn entry_key(&self, key: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(self.instance.as_bytes());
        h.update([0x1f]);
        h.update(key.as_bytes());
        let d = h.finalize();
        let mut s = String::with_capacity(16);
        for b in d.iter().take(8) { s.push_str(&format!("{b:02x}")); }
        s
    }

    /// The LEGACY entry (service `agent-hub:<instance>`, raw key) an older build
    /// wrote, for migration. Returns `None` when that name would itself exceed
    /// the OS limits (target <= 29, user <= 16) - in which case no legacy entry
    /// can exist (the old build could not have written one either), so there is
    /// nothing to migrate and no reason to attempt an over-limit keychain call.
    fn legacy_entry(&self, key: &str) -> Option<keyring::Entry> {
        if self.instance.is_empty() { return None; }
        let service = format!("agent-hub:{}", self.instance);
        if service.chars().count() > 29 || key.chars().count() > 16 { return None; }
        keyring::Entry::new(&service, key).ok()
    }

    pub fn is_available(&self) -> bool {
        self.available
    }

    /// A store that is known-unavailable, for tests that exercise the refusal
    /// path without touching the OS keychain.
    #[doc(hidden)]
    pub fn probe_unavailable_for_test() -> Self {
        SecretStore { service: "agent-hub-unavailable".into(), instance: String::new(), available: false }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, &self.entry_key(key)).map_err(|e| SecretError::Store(e.to_string()))
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
            Err(keyring::Error::NoEntry) => self.get_legacy_and_migrate(key),
            Err(e) => Err(SecretError::Store(e.to_string())),
        }
    }

    /// A miss on the current key: try the LEGACY entry an older build wrote.
    /// A hit is re-written under the current key and the legacy entry removed, so
    /// the credential migrates once and is never orphaned.
    fn get_legacy_and_migrate(&self, key: &str) -> Result<Option<String>, SecretError> {
        let Some(legacy) = self.legacy_entry(key) else { return Ok(None); };
        match legacy.get_password() {
            Ok(v) => {
                if self.entry(key)?.set_password(&v).is_ok() {
                    let _ = legacy.delete_credential();
                }
                Ok(Some(v))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Store(e.to_string())),
        }
    }

    /// Remove a credential; removing a missing one is a no-op success.
    pub fn delete(&self, key: &str) -> Result<(), SecretError> {
        if !self.available {
            return Err(SecretError::Unavailable);
        }
        if let Some(legacy) = self.legacy_entry(key) { let _ = legacy.delete_credential(); }
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
    // The probe name must fit the OS user-name limit (<= 16 on Windows):
    // "__probe__" + 6 hex = 15 chars. Random so it never collides with a real
    // entry nor a previous probe.
    let name = format!("__probe__{}", &random_hex()[..6]);
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

    #[test]
    fn set_get_delete_roundtrip() {
        // A fresh INSTANCE per run: two runs never share an entry. The service is
        // the fixed `agent-hub` (the OS caps the target name); the instance is in
        // the key.
        let s = SecretStore::for_instance(&format!("t{}", &random_hex()[..8]));
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
        let s = SecretStore { service: "x".into(), instance: String::new(), available: false };
        assert!(matches!(s.set("k", "v"), Err(SecretError::Unavailable)));
        assert!(matches!(s.get("k"), Err(SecretError::Unavailable)));
    }
}
