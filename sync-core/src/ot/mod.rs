//! Operational Transformation module for JSON Patch operations
//!
//! This module implements Operational Transformation (OT) algorithms for resolving
//! conflicts when multiple users concurrently edit the same JSON document using
//! JSON Patch operations (RFC 6902).

pub mod types;
pub mod path_utils;
pub mod transform;

pub use types::*;
pub use path_utils::*;
