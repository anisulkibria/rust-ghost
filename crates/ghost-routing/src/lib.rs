//! ghost-routing: routes.yaml / redirects.yaml parsing, URL matching and
//! template resolution — the port of Ghost's RouterManager.
pub mod redirects;
pub mod routes_yaml;
pub mod tree;

pub use routes_yaml::{RouteSettings, DEFAULT_ROUTES_YAML};
pub use tree::{RouteMatch, RouterTree};
