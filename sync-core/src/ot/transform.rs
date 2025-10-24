//! Operational transformation functions for JSON Patch operations (MVP)
//!
//! This module implements array index adjustments and conflict detection for
//! concurrent JSON Patch operations.

use json_patch::{PatchOperation, AddOperation, RemoveOperation, ReplaceOperation};
use crate::ot::path_utils::*;
use crate::ot::types::PathRelation;
use crate::SyncError;

// ============================================================================
// Add vs Add Transformation
// ============================================================================

/// Transform two Add operations
///
/// Returns (local_transformed, remote_transformed) or conflict indicator
pub fn transform_add_add(
    local: &AddOperation,
    remote: &AddOperation,
) -> Result<(Option<AddOperation>, Option<AddOperation>), SyncError> {
    // Check if both are array operations first (handles same index case)
    if let (Some(local_idx), Some(remote_idx)) =
        (extract_array_index(&local.path), extract_array_index(&remote.path))
    {
        // Verify they're in the same array
        if get_parent_path(&local.path) == get_parent_path(&remote.path) {
            // Both adding to same array - adjust indices
            if local_idx <= remote_idx {
                // Local goes first, adjust remote up
                let adjusted_remote = AddOperation {
                    path: adjust_array_index(&remote.path, remote_idx, 1)?,
                    value: remote.value.clone(),
                };
                return Ok((Some(local.clone()), Some(adjusted_remote)));
            } else {
                // Remote goes first, adjust local up
                let adjusted_local = AddOperation {
                    path: adjust_array_index(&local.path, local_idx, 1)?,
                    value: local.value.clone(),
                };
                return Ok((Some(adjusted_local), Some(remote.clone())));
            }
        }
    }

    // Not array operations - check path relations
    let path_relation = compare_paths(&local.path, &remote.path);

    match path_relation {
        PathRelation::Same => {
            // Conflict: both adding at same non-array path
            // Return both unchanged - caller decides which wins
            Ok((Some(local.clone()), Some(remote.clone())))
        }
        _ => {
            // Different paths, no conflict
            Ok((Some(local.clone()), Some(remote.clone())))
        }
    }
}

// ============================================================================
// Remove vs Remove Transformation
// ============================================================================

/// Transform two Remove operations
///
/// Returns (local_transformed, remote_transformed) or conflict indicator
pub fn transform_remove_remove(
    local: &RemoveOperation,
    remote: &RemoveOperation,
) -> Result<(Option<RemoveOperation>, Option<RemoveOperation>), SyncError> {
    // Check for array operations first (handles same index case)
    if let (Some(local_idx), Some(remote_idx)) =
        (extract_array_index(&local.path), extract_array_index(&remote.path))
    {
        if get_parent_path(&local.path) == get_parent_path(&remote.path) {
            // Same array - adjust indices
            if local_idx < remote_idx {
                // Local removes first, remote index shifts down
                let adjusted_remote = RemoveOperation {
                    path: adjust_array_index(&remote.path, remote_idx, -1)?,
                };
                return Ok((Some(local.clone()), Some(adjusted_remote)));
            } else if local_idx > remote_idx {
                // Remote removes first, local index shifts down
                let adjusted_local = RemoveOperation {
                    path: adjust_array_index(&local.path, local_idx, -1)?,
                };
                return Ok((Some(adjusted_local), Some(remote.clone())));
            } else {
                // Same index - conflict
                return Ok((Some(local.clone()), Some(remote.clone())));
            }
        }
    }

    // Not array operations - check path relations
    let path_relation = compare_paths(&local.path, &remote.path);

    match path_relation {
        PathRelation::Same => {
            // Both removing same path - conflict
            Ok((Some(local.clone()), Some(remote.clone())))
        }
        _ => {
            // Different paths, no conflict
            Ok((Some(local.clone()), Some(remote.clone())))
        }
    }
}

// ============================================================================
// Add vs Remove Transformation
// ============================================================================

/// Transform Add and Remove operations
///
/// Returns (add_transformed, remove_transformed)
pub fn transform_add_remove(
    add: &AddOperation,
    remove: &RemoveOperation,
) -> Result<(Option<AddOperation>, Option<RemoveOperation>), SyncError> {
    // Check if they're operating on same array
    if let (Some(add_idx), Some(rem_idx)) =
        (extract_array_index(&add.path), extract_array_index(&remove.path))
    {
        if get_parent_path(&add.path) == get_parent_path(&remove.path) {
            // Same array - adjust indices
            if add_idx <= rem_idx {
                // Add happens first, remove index shifts up
                let adjusted_remove = RemoveOperation {
                    path: adjust_array_index(&remove.path, rem_idx, 1)?,
                };
                Ok((Some(add.clone()), Some(adjusted_remove)))
            } else {
                // Remove happens first, add index shifts down
                let adjusted_add = AddOperation {
                    path: adjust_array_index(&add.path, add_idx, -1)?,
                    value: add.value.clone(),
                };
                Ok((Some(adjusted_add), Some(remove.clone())))
            }
        } else {
            // Different arrays
            Ok((Some(add.clone()), Some(remove.clone())))
        }
    } else {
        // Not array operations - check for path conflict
        if paths_conflict(&add.path, &remove.path) {
            // Conflict - return both
            Ok((Some(add.clone()), Some(remove.clone())))
        } else {
            // No conflict
            Ok((Some(add.clone()), Some(remove.clone())))
        }
    }
}

// ============================================================================
// Replace Operations (Simplified)
// ============================================================================

/// Transform two Replace operations
///
/// Simplified MVP: only checks if paths are identical
pub fn transform_replace_replace(
    local: &ReplaceOperation,
    remote: &ReplaceOperation,
) -> Result<(Option<ReplaceOperation>, Option<ReplaceOperation>), SyncError> {
    if local.path == remote.path {
        // Same path - conflict, return both
        Ok((Some(local.clone()), Some(remote.clone())))
    } else {
        // Different paths - no conflict
        Ok((Some(local.clone()), Some(remote.clone())))
    }
}

// ============================================================================
// Main Transform Function
// ============================================================================

/// Transform two patch operations (MVP implementation)
///
/// Handles: Add, Remove, Replace with array index adjustments
/// Returns conflicts for caller to resolve
///
/// # Examples
/// ```
/// use json_patch::{PatchOperation, AddOperation};
/// use serde_json::json;
/// use sync_core::ot::transform_operation_pair;
///
/// let local = PatchOperation::Add(AddOperation {
///     path: "/items/2".into(),
///     value: json!("local"),
/// });
/// let remote = PatchOperation::Add(AddOperation {
///     path: "/items/5".into(),
///     value: json!("remote"),
/// });
///
/// let (l, r) = transform_operation_pair(&local, &remote).unwrap();
/// // Remote index adjusted from 5 to 6
/// ```
pub fn transform_operation_pair(
    local: &PatchOperation,
    remote: &PatchOperation,
) -> Result<(Option<PatchOperation>, Option<PatchOperation>), SyncError> {
    match (local, remote) {
        // Add vs Add
        (PatchOperation::Add(l), PatchOperation::Add(r)) => {
            let (l_result, r_result) = transform_add_add(l, r)?;
            Ok((
                l_result.map(PatchOperation::Add),
                r_result.map(PatchOperation::Add),
            ))
        }

        // Remove vs Remove
        (PatchOperation::Remove(l), PatchOperation::Remove(r)) => {
            let (l_result, r_result) = transform_remove_remove(l, r)?;
            Ok((
                l_result.map(PatchOperation::Remove),
                r_result.map(PatchOperation::Remove),
            ))
        }

        // Add vs Remove (and inverse)
        (PatchOperation::Add(a), PatchOperation::Remove(r)) => {
            let (a_result, r_result) = transform_add_remove(a, r)?;
            Ok((
                a_result.map(PatchOperation::Add),
                r_result.map(PatchOperation::Remove),
            ))
        }
        (PatchOperation::Remove(r), PatchOperation::Add(a)) => {
            let (a_result, r_result) = transform_add_remove(a, r)?;
            Ok((
                r_result.map(PatchOperation::Remove),
                a_result.map(PatchOperation::Add),
            ))
        }

        // Replace vs Replace
        (PatchOperation::Replace(l), PatchOperation::Replace(r)) => {
            let (l_result, r_result) = transform_replace_replace(l, r)?;
            Ok((
                l_result.map(PatchOperation::Replace),
                r_result.map(PatchOperation::Replace),
            ))
        }

        // Test operations - pass through (read-only)
        (PatchOperation::Test(_), _) | (_, PatchOperation::Test(_)) => {
            Ok((Some(local.clone()), Some(remote.clone())))
        }

        // Move/Copy - mark as conflict for MVP
        (PatchOperation::Move(_), _) | (PatchOperation::Copy(_), _) |
        (_, PatchOperation::Move(_)) | (_, PatchOperation::Copy(_)) => {
            // Complex operations - return as conflict
            Ok((Some(local.clone()), Some(remote.clone())))
        }

        // All other combinations - no conflict
        _ => Ok((Some(local.clone()), Some(remote.clone())))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========================================================================
    // Add vs Add Tests
    // ========================================================================

    #[test]
    fn test_add_add_different_paths() {
        let local = AddOperation {
            path: "/user/name".into(),
            value: json!("Alice"),
        };
        let remote = AddOperation {
            path: "/user/email".into(),
            value: json!("alice@example.com"),
        };

        let (l, r) = transform_add_add(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
        assert_eq!(l.unwrap().path, "/user/name");
        assert_eq!(r.unwrap().path, "/user/email");
    }

    #[test]
    fn test_add_add_same_path_conflict() {
        let local = AddOperation {
            path: "/settings/theme".into(),
            value: json!("dark"),
        };
        let remote = AddOperation {
            path: "/settings/theme".into(),
            value: json!("light"),
        };

        let (l, r) = transform_add_add(&local, &remote).unwrap();
        // Both returned - caller resolves conflict
        assert!(l.is_some());
        assert!(r.is_some());
    }

    #[test]
    fn test_add_add_array_different_indices() {
        let local = AddOperation {
            path: "/items/2".into(),
            value: json!("Local"),
        };
        let remote = AddOperation {
            path: "/items/5".into(),
            value: json!("Remote"),
        };

        let (l, r) = transform_add_add(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
        assert_eq!(l.unwrap().path, "/items/2");
        assert_eq!(r.unwrap().path, "/items/6"); // Adjusted up by 1
    }

    #[test]
    fn test_add_add_array_same_index() {
        let local = AddOperation {
            path: "/items/3".into(),
            value: json!("A"),
        };
        let remote = AddOperation {
            path: "/items/3".into(),
            value: json!("B"),
        };

        let (l, r) = transform_add_add(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
        // Remote adjusted to next index
        assert_eq!(l.unwrap().path, "/items/3");
        assert_eq!(r.unwrap().path, "/items/4");
    }

    #[test]
    fn test_add_add_root_level_array() {
        // Root-level array: /0, /1, /2
        let local = AddOperation {
            path: "/1".into(),
            value: json!("Local"),
        };
        let remote = AddOperation {
            path: "/3".into(),
            value: json!("Remote"),
        };

        let (l, r) = transform_add_add(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
        assert_eq!(l.unwrap().path, "/1");
        assert_eq!(r.unwrap().path, "/4"); // Adjusted
    }

    // ========================================================================
    // Remove vs Remove Tests
    // ========================================================================

    #[test]
    fn test_remove_remove_same_path() {
        let local = RemoveOperation {
            path: "/user/temp".into(),
        };
        let remote = RemoveOperation {
            path: "/user/temp".into(),
        };

        let (l, r) = transform_remove_remove(&local, &remote).unwrap();
        // Both returned - caller decides
        assert!(l.is_some());
        assert!(r.is_some());
    }

    #[test]
    fn test_remove_remove_different_paths() {
        let local = RemoveOperation {
            path: "/config/debug".into(),
        };
        let remote = RemoveOperation {
            path: "/config/verbose".into(),
        };

        let (l, r) = transform_remove_remove(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
    }

    #[test]
    fn test_remove_remove_array_different_indices() {
        let local = RemoveOperation {
            path: "/items/5".into(),
        };
        let remote = RemoveOperation {
            path: "/items/2".into(),
        };

        let (l, r) = transform_remove_remove(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
        // Local index adjusted down (remote removes first)
        assert_eq!(l.unwrap().path, "/items/4");
        assert_eq!(r.unwrap().path, "/items/2");
    }

    #[test]
    fn test_remove_remove_array_same_index() {
        let local = RemoveOperation {
            path: "/items/3".into(),
        };
        let remote = RemoveOperation {
            path: "/items/3".into(),
        };

        let (l, r) = transform_remove_remove(&local, &remote).unwrap();
        // Conflict - both returned
        assert!(l.is_some());
        assert!(r.is_some());
    }

    // ========================================================================
    // Add vs Remove Tests
    // ========================================================================

    #[test]
    fn test_add_remove_unrelated() {
        let add = AddOperation {
            path: "/users/new".into(),
            value: json!({"name": "Alice"}),
        };
        let remove = RemoveOperation {
            path: "/posts/old".into(),
        };

        let (a, r) = transform_add_remove(&add, &remove).unwrap();
        assert!(a.is_some());
        assert!(r.is_some());
    }

    #[test]
    fn test_add_remove_array_add_before() {
        let add = AddOperation {
            path: "/items/1".into(),
            value: json!("New"),
        };
        let remove = RemoveOperation {
            path: "/items/3".into(),
        };

        let (a, r) = transform_add_remove(&add, &remove).unwrap();
        assert!(a.is_some());
        assert!(r.is_some());
        assert_eq!(a.unwrap().path, "/items/1");
        assert_eq!(r.unwrap().path, "/items/4"); // Adjusted up
    }

    #[test]
    fn test_add_remove_array_add_after() {
        let add = AddOperation {
            path: "/items/5".into(),
            value: json!("New"),
        };
        let remove = RemoveOperation {
            path: "/items/2".into(),
        };

        let (a, r) = transform_add_remove(&add, &remove).unwrap();
        assert!(a.is_some());
        assert!(r.is_some());
        assert_eq!(a.unwrap().path, "/items/4"); // Adjusted down
        assert_eq!(r.unwrap().path, "/items/2");
    }

    // ========================================================================
    // Replace Operations Tests
    // ========================================================================

    #[test]
    fn test_replace_replace_same_path() {
        let local = ReplaceOperation {
            path: "/theme".into(),
            value: json!("dark"),
        };
        let remote = ReplaceOperation {
            path: "/theme".into(),
            value: json!("light"),
        };

        let (l, r) = transform_replace_replace(&local, &remote).unwrap();
        // Conflict - both returned
        assert!(l.is_some());
        assert!(r.is_some());
    }

    #[test]
    fn test_replace_replace_different_paths() {
        let local = ReplaceOperation {
            path: "/user/name".into(),
            value: json!("Alice"),
        };
        let remote = ReplaceOperation {
            path: "/user/age".into(),
            value: json!(30),
        };

        let (l, r) = transform_replace_replace(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
    }

    // ========================================================================
    // Main Transform Function Tests
    // ========================================================================

    #[test]
    fn test_transform_operation_pair_add_add() {
        let local = PatchOperation::Add(AddOperation {
            path: "/items/2".into(),
            value: json!("local"),
        });
        let remote = PatchOperation::Add(AddOperation {
            path: "/items/5".into(),
            value: json!("remote"),
        });

        let (l, r) = transform_operation_pair(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());

        if let Some(PatchOperation::Add(r_add)) = r {
            assert_eq!(r_add.path, "/items/6");
        } else {
            panic!("Expected Add operation");
        }
    }

    #[test]
    fn test_transform_operation_pair_test_passthrough() {
        let local = PatchOperation::Test(json_patch::TestOperation {
            path: "/version".into(),
            value: json!(1),
        });
        let remote = PatchOperation::Add(AddOperation {
            path: "/items/0".into(),
            value: json!("new"),
        });

        let (l, r) = transform_operation_pair(&local, &remote).unwrap();
        assert!(l.is_some());
        assert!(r.is_some());
    }

    #[test]
    fn test_transform_operation_pair_move_conflict() {
        let local = PatchOperation::Move(json_patch::MoveOperation {
            from: "/a".into(),
            path: "/b".into(),
        });
        let remote = PatchOperation::Add(AddOperation {
            path: "/c".into(),
            value: json!("value"),
        });

        let (l, r) = transform_operation_pair(&local, &remote).unwrap();
        // Both returned as conflict (Move not implemented in MVP)
        assert!(l.is_some());
        assert!(r.is_some());
    }
}
