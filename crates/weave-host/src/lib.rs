//! Shared trusted embedding. Host authority and durability remain explicit.
pub mod artifacts;
pub mod cluster_journal;
pub mod host;
#[cfg(feature = "browser-image-experiment")]
pub mod image_host;
/// Duplicate-aware byte preflight shared by the Rust facade and C configuration wrappers.
#[doc(hidden)]
pub mod strict_json;
