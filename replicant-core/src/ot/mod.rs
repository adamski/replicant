//! Operational Transformation module for JSON Patch operations
//!
//! This module implements Operational Transformation (OT) algorithms for resolving
//! conflicts when multiple users concurrently edit the same JSON document using
//! JSON Patch operations (RFC 6902).

pub mod path_utils;
pub mod transform;
pub mod types;

pub use path_utils::*;
pub use transform::transform_operation_pair;
pub use types::*;
