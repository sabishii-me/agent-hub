//! Catalog fetch over HTTP (`ARCHITECTURE` §5): the hub owns the request, the
//! dialect and the field mapping. A failure keeps the previous catalog and
//! reports the error; it never clears the selection.

use serde_json::Value;

use crate::record::{Catalog, CatalogModel};

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("the provider answered {status}")]
    Status { status: u16 },
    #[error("the catalog is not in a shape the hub understands: {0}")]
    Shape(String),
}

/// Fetch a provider's model catalog. `api` selects the dialect; `url` is the
/// base; `token` is the bearer. Returns the models the vendor lists.
pub async fn fetch(
    api: Option<&str>,
    url: &str,
    token: Option<&str>,
) -> Result<Vec<CatalogModel>, CatalogError> {
    let client = reqwest::Client::new();
    let endpoint = models_endpoint(api, url);
    let mut req = client.get(&endpoint);
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(CatalogError::Status { status: resp.status().as_u16() });
    }
    let body: Value = resp.json().await?;
    parse_models(&body)
}

/// The models endpoint for a dialect. The hub owns this mapping; the provider
/// is data.
fn models_endpoint(api: Option<&str>, url: &str) -> String {
    let base = url.trim_end_matches('/');
    match api {
        Some("anthropic-messages") => format!("{base}/v1/models"),
        // openai-completions / responses / custom-compatible all expose /models.
        _ => format!("{base}/models"),
    }
}

/// Map a vendor payload to catalog models. Accepts the OpenAI shape
/// (`{data:[{id,...}]}`), the Anthropic shape (`{data:[{id,display_name}]}`),
/// and a bare `{models:[...]}` / `[...]`.
fn parse_models(body: &Value) -> Result<Vec<CatalogModel>, CatalogError> {
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.get("models").and_then(Value::as_array))
        .or_else(|| body.as_array())
        .ok_or_else(|| CatalogError::Shape("no `data` or `models` array".into()))?;

    let mut out = Vec::new();
    for it in items {
        let id = match it.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };
        out.push(CatalogModel {
            id,
            name: it
                .get("display_name")
                .or_else(|| it.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string),
            thinking_levels: None,
            context_window: it
                .get("context_window")
                .or_else(|| it.get("context_length"))
                .and_then(Value::as_u64),
            max_tokens: it
                .get("max_output_tokens")
                .or_else(|| it.get("max_tokens"))
                .and_then(Value::as_u64),
            input: it.get("input").cloned().and_then(|v| serde_json::from_value(v).ok()),
            cost: None,
        });
    }
    Ok(out)
}

/// Build a fresh catalog at a given revision.
pub fn catalog_at(models: Vec<CatalogModel>, revision: u64) -> Catalog {
    Catalog { fetched_at: Some(agent_hub_db::now_utc()), revision, models }
}
