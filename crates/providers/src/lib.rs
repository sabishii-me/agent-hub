//! The providers domain (`ARCHITECTURE` §5): a model provider is **data**.
//!
//! One provider is one JSON file in the hub's data dir. The hub owns the HTTP
//! request, the auth, the catalog fetch and the field mapping; **no plugin runs
//! code inside the hub's process**.

pub mod catalog;
pub mod record;
pub mod routes;
pub mod service;
pub mod store;

pub use record::{Catalog, CatalogModel, Declaration, ProviderRecord};
pub use service::{CreateProvider, PatchProvider, ProviderError, Providers};
pub use store::{Broken, ProviderStore};
