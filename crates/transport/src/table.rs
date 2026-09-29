//! One declaration per route: the router and the surface come from the SAME
//! registration (TASK-048 C5).
//!
//! A domain registers routes with `.get(path, handler)`, `.post(path, handler)`,
//! etc. The table records the method and path **as it registers them** and merges
//! several methods on one path itself. There is no separate `methods: &[&str]`
//! argument to keep in step with the `MethodRouter` chain, and no second
//! hand-written surface array: `surface()` is derived from what was registered.

use axum::handler::Handler;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

/// A router builder that remembers each method+path as it registers.
pub struct RouteTable<S> {
    router: Router<S>,
    /// `(method_upper, contract_path)` for every route registered.
    surface: Vec<(String, String)>,
}

impl<S> RouteTable<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        RouteTable { router: Router::new(), surface: Vec::new() }
    }

    fn mount<H, T>(mut self, method: &str, axum_path: &str, contract_path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        let method_router = match method {
            "GET" => get(handler),
            "POST" => post(handler),
            "PUT" => put(handler),
            "PATCH" => patch(handler),
            "DELETE" => delete(handler),
            other => panic!("unsupported method `{other}` in RouteTable"),
        };
        // Merge with an existing route on the same axum path so many methods
        // share one path.
        self.router = self.router.route(axum_path, method_router);
        self.surface.push((method.to_string(), contract_path.to_string()));
        self
    }

    /// Register a GET; the contract path may differ from the axum path (a
    /// catch-all is `{*file}` in axum but `{file...}` in the contract).
    pub fn get<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.mount("GET", path, path, handler)
    }
    pub fn post<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.mount("POST", path, path, handler)
    }
    pub fn patch<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.mount("PATCH", path, path, handler)
    }
    pub fn delete<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.mount("DELETE", path, path, handler)
    }
    pub fn put<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.mount("PUT", path, path, handler)
    }

    /// Register a method where the axum path and the contract path differ.
    pub fn get_as<H, T>(self, axum_path: &str, contract_path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.mount("GET", axum_path, contract_path, handler)
    }
    pub fn put_as<H, T>(self, axum_path: &str, contract_path: &str, handler: H) -> Self
    where
        H: Handler<T, S>,
        T: 'static,
    {
        self.mount("PUT", axum_path, contract_path, handler)
    }

    pub fn surface(&self) -> Vec<String> {
        self.surface
            .iter()
            .map(|(m, p)| format!("{m} {p}"))
            .collect()
    }

    pub fn router(self) -> Router<S> {
        self.router
    }
}

impl<S> Default for RouteTable<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}
