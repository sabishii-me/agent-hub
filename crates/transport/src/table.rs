//! One declaration per route: the router and the surface come from the SAME
//! list, so they cannot drift (TASK-048 C5).
//!
//! A domain builds its `Router<State>` with a [`RouteTable`] that records each
//! `path` and the HTTP methods mounted on it **as they are registered**. The
//! recorded surface is what the boot self-check compares to the contract; there
//! is no second hand-written array to forget to update.

use axum::routing::MethodRouter;
use axum::Router;

/// A router builder that remembers what it mounted.
pub struct RouteTable<S> {
    router: Router<S>,
    /// `(method_upper, path)` for every method mounted.
    surface: Vec<(String, String)>,
}

impl<S: Clone + Send + Sync + 'static> RouteTable<S> {
    pub fn new() -> Self {
        RouteTable { router: Router::new(), surface: Vec::new() }
    }

    /// Mount a `MethodRouter` on `path`, recording the methods it serves.
    ///
    /// The caller states the methods it chained (axum's `MethodRouter` does not
    /// expose them), so the recorded surface reflects the actual chain.
    pub fn mount(mut self, path: &str, methods: &[&str], router: MethodRouter<S>) -> Self {
        self.router = self.router.route(path, router);
        for m in methods {
            self.surface.push((m.to_uppercase(), path.to_string()));
        }
        self
    }

    /// The recorded surface as `"METHOD /path"`.
    pub fn surface(&self) -> Vec<String> {
        self.surface
            .iter()
            .map(|(m, p)| format!("{m} {p}"))
            .collect()
    }

    /// Mount where the **recorded** path differs from the axum path (a catch-all
    /// is `{*file}` in axum but `{file...}` in the contract).
    pub fn mount_as(
        mut self,
        axum_path: &str,
        contract_path: &str,
        methods: &[&str],
        router: MethodRouter<S>,
    ) -> Self {
        self.router = self.router.route(axum_path, router);
        for m in methods {
            self.surface.push((m.to_uppercase(), contract_path.to_string()));
        }
        self
    }

    pub fn router(self) -> Router<S> {
        self.router
    }
}

impl<S: Clone + Send + Sync + 'static> Default for RouteTable<S> {
    fn default() -> Self {
        Self::new()
    }
}
