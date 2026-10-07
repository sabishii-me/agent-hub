//! Provider-TYPE descriptors, as DATA: the hub reads a descriptor a model-provider
//! plugin ships and maps it - it never imports or runs plugin code in-process
//! (ARCHITECTURE 5). A type no plugin ships stays absent; a descriptor that cannot be
//! honoured is reported in `broken[]`, never a 500 (TASK-048; `GET /v1/model-providers/types`).
//!
//! Location: a plugin directory whose `manifest.json` declares
//! `pluginType: "model-provider"` ships its descriptor in `provider.json` beside the
//! manifest. There is NO plugin-provider module protocol (the contract says so): the
//! hub only reads this JSON.

use std::path::Path;

/// One installed provider type, exactly the shape the contract requires.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct TypeDescriptor {
    pub id: String,
    pub version: u64,
    pub owner: String,
    pub name: std::collections::BTreeMap<String, String>,
    #[serde(rename = "authMethods")]
    pub auth_methods: Vec<String>,
    pub configuration: serde_json::Value,
    pub catalog: serde_json::Value,
}

/// Why a shipped descriptor could not be used.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BrokenType {
    pub plugin: String,
    pub error: String,
}

/// The loader result: usable types and the ones that could not be honoured.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TypeCatalog {
    pub types: Vec<TypeDescriptor>,
    pub broken: Vec<BrokenType>,
}

impl TypeCatalog {
    /// Scan a plugins root. A directory that is not a model-provider plugin is
    /// ignored; a model-provider plugin with an unreadable/invalid descriptor is
    /// `broken[]` (the hub never guesses a type into existence).
    pub fn scan(plugins_root: &Path) -> TypeCatalog {
        let mut out = TypeCatalog::default();
        let entries = match std::fs::read_dir(plugins_root) {
            Ok(e) => e,
            Err(_) => return out, // no plugins root = no types (a fact, not an error)
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let plugin = entry.file_name().to_string_lossy().to_string();
            let manifest = match std::fs::read_to_string(dir.join("manifest.json")) {
                Ok(m) => m,
                Err(_) => continue, // not a plugin directory
            };
            let is_provider = serde_json::from_str::<serde_json::Value>(&manifest)
                .ok()
                .and_then(|v| v.get("pluginType").and_then(|t| t.as_str()).map(str::to_string))
                .as_deref()
                == Some("model-provider");
            if !is_provider {
                continue;
            }
            match load_one(&dir) {
                Ok(t) => out.types.push(t),
                Err(e) => out.broken.push(BrokenType { plugin, error: e }),
            }
        }
        out
    }
}

impl TypeCatalog {
    /// The public response body: `{types, broken}`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "types": self.types, "broken": self.broken })
    }
}

impl TypeDescriptor {
    /// The ENDPOINT this type OWNS, when its descriptor declares one
    /// (`configuration.endpoint = {url, api}`). A type that owns its endpoint
    /// supplies it and REJECTS caller overrides (the DeepSeek case: the plugin
    /// knows `https://api.deepseek.com`, the person supplies only the key).
    /// Returns `None` for a type that accepts a caller endpoint
    /// (`custom-compatible`).
    pub fn owned_endpoint(&self) -> Option<(Option<String>, Option<String>)> {
        let ep = self.configuration.get("endpoint")?;
        let url = ep.get("url").and_then(|v| v.as_str()).map(str::to_string);
        let api = ep.get("api").and_then(|v| v.as_str()).map(str::to_string);
        if url.is_none() && api.is_none() {
            return None;
        }
        Some((url, api))
    }

    /// Whether the caller MUST supply a credential for this type (a `secret` field
    /// in `configuration.fields`). A type with such a field needs a token.
    pub fn requires_token(&self) -> bool {
        self.configuration
            .get("fields")
            .and_then(|f| f.as_array())
            .map(|a| {
                a.iter().any(|f| {
                    f.get("type").and_then(|t| t.as_str()) == Some("secret")
                        && f.get("required").and_then(|r| r.as_bool()).unwrap_or(true)
                })
            })
            .unwrap_or(false)
    }
}

/// Load and VALIDATE one descriptor (`provider.json`). Every required field must be
/// present and well-typed; `owner` must be `hub` (the hub owns the HTTP/auth/catalog,
/// not the plugin).
fn load_one(dir: &Path) -> Result<TypeDescriptor, String> {
    let raw = std::fs::read_to_string(dir.join("provider.json"))
        .map_err(|e| format!("no readable provider.json: {e}"))?;
    let t: TypeDescriptor =
        serde_json::from_str(&raw).map_err(|e| format!("provider.json is not a valid descriptor: {e}"))?;
    if t.id.trim().is_empty() {
        return Err("descriptor declares no id".into());
    }
    if t.version < 1 {
        return Err(format!("descriptor version must be >= 1, got {}", t.version));
    }
    if t.owner != "hub" {
        return Err(format!("descriptor owner must be \"hub\", got {:?}", t.owner));
    }
    if t.name.is_empty() {
        return Err("descriptor declares no name".into());
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ptypes-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_provider(dir: &Path, id: &str, descriptor: &str) {
        let p = dir.join(id);
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("manifest.json"),
            format!(r#"{{"id":"{id}","pluginType":"model-provider"}}"#),
        )
        .unwrap();
        std::fs::write(p.join("provider.json"), descriptor).unwrap();
    }

    #[test]
    fn a_valid_descriptor_is_reported() {
        let root = tmp("valid");
        write_provider(
            &root,
            "acme",
            r#"{"id":"acme","version":1,"owner":"hub","name":{"en":"Acme"},
                "authMethods":["api-key"],"configuration":{},"catalog":{}}"#,
        );
        let c = TypeCatalog::scan(&root);
        assert_eq!(c.types.len(), 1);
        assert_eq!(c.types[0].id, "acme");
        assert!(c.broken.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_broken_descriptor_is_reported_not_fatal() {
        let root = tmp("broken");
        write_provider(&root, "bad", r#"{"id":"bad","version":0}"#); // version<1, missing fields
        write_provider(
            &root,
            "good",
            r#"{"id":"good","version":1,"owner":"hub","name":{"en":"G"},
                "authMethods":[],"configuration":{},"catalog":{}}"#,
        );
        let c = TypeCatalog::scan(&root);
        assert_eq!(c.types.len(), 1, "the good one still loads");
        assert_eq!(c.broken.len(), 1, "the bad one is broken[]");
        assert_eq!(c.broken[0].plugin, "bad");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_non_provider_plugin_is_ignored() {
        let root = tmp("ignored");
        let p = root.join("pi");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(
            p.join("manifest.json"),
            r#"{"id":"pi","pluginType":"harness-adapter"}"#,
        )
        .unwrap();
        let c = TypeCatalog::scan(&root);
        assert!(c.types.is_empty() && c.broken.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_non_hub_owner_is_refused() {
        let root = tmp("owner");
        write_provider(
            &root,
            "x",
            r#"{"id":"x","version":1,"owner":"plugin","name":{"en":"X"},
                "authMethods":[],"configuration":{},"catalog":{}}"#,
        );
        let c = TypeCatalog::scan(&root);
        assert!(c.types.is_empty());
        assert_eq!(c.broken.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
