//! Inbound bearer authentication (`ARCHITECTURE` §7, o5).
//!
//! The hub is loopback, and a client holds the token from `endpoint.json`. Every
//! route requires it: there is **no anonymous discovery** (o5). A missing or wrong
//! token is `401 unauthorized` from the contract's error table.
//!
//! This is a **possession** check, not an authorization boundary: a bearer proves
//! the holder read `endpoint.json`. It does not prove the holder is the trusted
//! human, and it is not a substitute for an OS principal boundary (see §7).

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

/// A constant-time equality, so a token check does not leak the token by timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The bearer middleware. Wrap the whole router with it.
pub async fn require_bearer(
    axum::extract::State(expected): axum::extract::State<BearerToken>,
    req: Request,
    next: Next,
) -> Response {
    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    match presented {
        Some(t) if constant_time_eq(t.as_bytes(), expected.0.as_bytes()) => next.run(req).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response(),
    }
}

/// The expected bearer token.
#[derive(Clone)]
pub struct BearerToken(pub String);

impl BearerToken {
    /// Generate a fresh 32-byte hex token from the OS entropy source.
    pub fn generate() -> Self {
        use std::fmt::Write;
        let mut buf = [0u8; 32];
        // Not hand-rolled: the OS random source (getrandom). Infrastructure is a
        // crate, not a smear (ARCHITECTURE §2).
        getrandom::fill(&mut buf).expect("OS randomness");
        let mut s = String::with_capacity(64);
        for b in buf {
            let _ = write!(s, "{b:02x}");
        }
        BearerToken(s)
    }

    pub fn from_env_or_generate() -> Self {
        match std::env::var("AGENT_HUB_TOKEN") {
            Ok(t) if !t.is_empty() => BearerToken(t),
            _ => Self::generate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_equal() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn generated_token_is_64_hex_chars() {
        let t = BearerToken::generate();
        assert_eq!(t.0.len(), 64);
        assert!(t.0.chars().all(|c| c.is_ascii_hexdigit()));
        // Two calls differ (smear includes the address of a fresh buffer).
        assert_ne!(BearerToken::generate().0, t.0);
    }
}
