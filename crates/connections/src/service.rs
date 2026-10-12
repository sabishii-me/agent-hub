//! The connections domain: hub-managed connections in the database, the credential
//! in the OS secret store (a reference, never a value).

use std::sync::Arc;

use agent_hub_db::{ConnectionRow, Db, DbError};
use agent_hub_secrets::SecretStore;

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("this hub has no secret store; a credential is refused rather than stored in plaintext")]
    NoSecretStore,
    #[error("the stored credential could not be read: {0}")]
    SecretUnreadable(String),
}

impl ConnectionError {
    pub fn code(&self) -> &'static str {
        match self {
            ConnectionError::Db(DbError::NotFound(_)) => "connection_not_found",
            ConnectionError::Db(DbError::Conflict(_)) => "already_exists",
            ConnectionError::Db(_) => "internal_error",
            ConnectionError::Validation(_) => "validation_failed",
            ConnectionError::NoSecretStore => "not_implemented",
            ConnectionError::SecretUnreadable(_) => "internal_error",
        }
    }

    pub fn to_domain_error(&self) -> agent_hub_transport::DomainError {
        agent_hub_transport::DomainError::new(self.code(), self.to_string())
    }
}

/// A connection as the contract renders it: **credential-free**
/// (`{id, name, scheme, endpoint, envName, state, credentialConfigured}`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConnectionView {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(rename = "envName", skip_serializing_if = "Option::is_none")]
    pub env_name: Option<String>,
    pub state: String,
    #[serde(rename = "credentialConfigured")]
    pub credential_configured: bool,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct CreateConnection {
    pub id: Option<String>,
    pub name: Option<String>,
    pub scheme: Option<String>,
    pub endpoint: Option<String>,
    #[serde(rename = "envName")]
    pub env_name: Option<String>,
    pub token: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct PatchConnection {
    pub name: Option<String>,
    pub scheme: Option<String>,
    pub endpoint: Option<String>,
    #[serde(rename = "envName")]
    pub env_name: Option<String>,
    pub token: Option<Option<String>>,
    pub state: Option<String>,
}

pub struct Connections {
    db: Db,
    secrets: Arc<SecretStore>,
    namespace: String,
    /// One lock per connection id: a two-store operation (row + keychain) is
    /// serialised, so a create never interleaves with a delete.
    locks: std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Connections {
    pub fn new(db: Db, secrets: Arc<SecretStore>, namespace: impl Into<String>) -> Self {
        Connections {
            db,
            secrets,
            namespace: namespace.into(),
            locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn lock_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .expect("connection locks")
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    fn secret_ref(&self, id: &str) -> String {
        format!("{}:connection-{id}", self.namespace)
    }

    /// The view, with `credentialConfigured` READ from the secret store using the
    /// row's OWN reference (never a re-derived key: a rebuilt id cannot re-acquire
    /// an old credential).
    fn view(&self, row: &ConnectionRow) -> Result<ConnectionView, ConnectionError> {
        let configured = match &row.secret_ref {
            Some(r) => self
                .secrets
                .get(r)
                .map_err(|e| ConnectionError::SecretUnreadable(e.to_string()))?
                .is_some(),
            None => false,
        };
        Ok(ConnectionView {
            id: row.id.clone(),
            name: row.name.clone(),
            scheme: row.scheme.clone(),
            endpoint: row.endpoint.clone(),
            env_name: row.env_name.clone(),
            state: row.state.clone(),
            credential_configured: configured,
        })
    }

    pub fn list(&self) -> Result<Vec<ConnectionView>, ConnectionError> {
        self.db
            .list_connections()?
            .iter()
            .map(|r| self.view(r))
            .collect()
    }

    pub fn get(&self, id: &str) -> Result<ConnectionView, ConnectionError> {
        let row = self
            .db
            .connection(id)?
            .ok_or_else(|| ConnectionError::Db(DbError::NotFound(id.into())))?;
        self.view(&row)
    }

    pub async fn create(&self, req: CreateConnection) -> Result<ConnectionView, ConnectionError> {
        if req.token.is_some() && !self.secrets.is_available() {
            return Err(ConnectionError::NoSecretStore);
        }
        let id = req
            .id
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ConnectionError::Validation("id is required".into()))?;
        let name = req
            .name
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ConnectionError::Validation("name is required".into()))?;
        let lock = self.lock_for(&id);
        let _guard = lock.lock().await;

        let row = ConnectionRow {
            id: id.clone(),
            name,
            scheme: req.scheme.clone(),
            endpoint: req.endpoint.clone(),
            env_name: req.env_name.clone(),
            state: req.state.clone().unwrap_or_else(|| "enabled".into()),
            secret_ref: None,
            incarnation: String::new(),
            revision: 1,
        };
        let mut stored = self.db.insert_connection(&row)?;
        if let Some(t) = &req.token {
            self.secrets
                .set(&self.secret_ref(&id), t)
                .map_err(|e| ConnectionError::SecretUnreadable(e.to_string()))?;
            stored.secret_ref = Some(self.secret_ref(&id));
            if let Err(e) = self.db.save_connection(&stored) {
                // The credential exists but the row does not point at it: remove
                // the credential rather than leave an unowned secret.
                let _ = self.secrets.delete(&self.secret_ref(&id));
                let _ = self.db.delete_connection(&id);
                return Err(ConnectionError::Db(e));
            }
        }
        self.view(&stored)
    }

    pub async fn patch(
        &self,
        id: &str,
        req: PatchConnection,
    ) -> Result<ConnectionView, ConnectionError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        let mut row = self
            .db
            .connection(id)?
            .ok_or_else(|| ConnectionError::Db(DbError::NotFound(id.into())))?;
        if let Some(v) = req.name {
            row.name = v;
        }
        if let Some(v) = req.scheme {
            row.scheme = Some(v);
        }
        if let Some(v) = req.endpoint {
            row.endpoint = Some(v);
        }
        if let Some(v) = req.env_name {
            row.env_name = Some(v);
        }
        if let Some(v) = req.state {
            if v != "enabled" && v != "disabled" {
                return Err(ConnectionError::Validation("state must be enabled|disabled".into()));
            }
            row.state = v;
        }
        if let Some(tok) = req.token {
            match tok {
                Some(t) => {
                    if !self.secrets.is_available() {
                        return Err(ConnectionError::NoSecretStore);
                    }
                    self.secrets
                        .set(&self.secret_ref(id), &t)
                        .map_err(|e| ConnectionError::SecretUnreadable(e.to_string()))?;
                    row.secret_ref = Some(self.secret_ref(id));
                }
                None => {
                    self.secrets
                        .delete(&self.secret_ref(id))
                        .map_err(|e| ConnectionError::SecretUnreadable(e.to_string()))?;
                    row.secret_ref = None;
                }
            }
        }
        row.revision += 1;
        self.db.save_connection(&row)?;
        self.view(&row)
    }

    /// Delete the connection AND its stored credential.
    pub async fn delete(&self, id: &str) -> Result<(), ConnectionError> {
        let lock = self.lock_for(id);
        let _guard = lock.lock().await;
        let row = self
            .db
            .connection(id)?
            .ok_or_else(|| ConnectionError::Db(DbError::NotFound(id.into())))?;
        if let Some(r) = &row.secret_ref {
            self.secrets
                .delete(r)
                .map_err(|e| ConnectionError::SecretUnreadable(e.to_string()))?;
        }
        self.db.delete_connection(id)?;
        Ok(())
    }

    /// Resolve every ENABLED connection's credential for a session's adapter, as
    /// `(envName, value)`. A disabled connection materialises NOTHING (zero), and a
    /// connection with no envName or no credential is skipped (never an empty value).
    pub fn materialize(&self) -> Result<Vec<(String, String)>, ConnectionError> {
        let mut out = Vec::new();
        for row in self.db.list_connections()? {
            if row.state != "enabled" {
                continue;
            }
            let (Some(env), Some(r)) = (row.env_name.clone(), row.secret_ref.clone()) else {
                continue;
            };
            let value = self
                .secrets
                .get(&r)
                .map_err(|e| ConnectionError::SecretUnreadable(e.to_string()))?;
            if let Some(v) = value {
                out.push((env, v));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_hub_db::Db;

    fn conns() -> Connections {
        Connections::new(
            Db::open_in_memory().unwrap(),
            Arc::new(agent_hub_secrets::SecretStore::probe_unavailable_for_test()),
            "test",
        )
    }

    /// A connection with no secret store still stores its ROW (no credential).
    #[tokio::test]
    async fn create_without_token_stores_the_row() {
        let c = conns();
        let v = c
            .create(CreateConnection {
                id: Some("a".into()),
                name: Some("A".into()),
                endpoint: Some("https://x.test".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(v.id, "a");
        assert!(!v.credential_configured);
        assert_eq!(c.list().unwrap().len(), 1);
    }

    /// A credential is REFUSED when the secret store is unavailable (never
    /// plaintext).
    #[tokio::test]
    async fn a_token_is_refused_without_a_secret_store() {
        let c = conns();
        let e = c
            .create(CreateConnection {
                id: Some("a".into()),
                name: Some("A".into()),
                token: Some("secret".into()),
                ..Default::default()
            })
            .await;
        assert!(matches!(e, Err(ConnectionError::NoSecretStore)));
    }

    /// A disabled connection materialises NOTHING (zero), and an unknown
    /// connection delete is `unknown_connection`.
    #[tokio::test]
    async fn disabled_materialises_nothing_and_delete_is_guarded() {
        let c = conns();
        c.create(CreateConnection {
            id: Some("a".into()),
            name: Some("A".into()),
            state: Some("disabled".into()),
            env_name: Some("X_TOKEN".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(c.materialize().unwrap().is_empty(), "disabled => zero materialization");
        assert!(c.delete("missing").await.is_err());
    }

    #[tokio::test]
    async fn a_duplicate_id_is_already_exists() {
        let c = conns();
        c.create(CreateConnection { id: Some("a".into()), name: Some("A".into()), ..Default::default() })
            .await
            .unwrap();
        let e = c
            .create(CreateConnection { id: Some("a".into()), name: Some("B".into()), ..Default::default() })
            .await;
        assert_eq!(e.unwrap_err().code(), "already_exists");
    }
}
