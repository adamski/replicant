use crate::errors::SyncError;
use crate::ot::transform_operation_pair;
use crate::SyncResult;
use json_patch::{Patch, PatchOperation};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn create_patch(from: &Value, to: &Value) -> SyncResult<Patch> {
    let diff = json_patch::diff(from, to);
    Ok(diff)
}

pub fn apply_patch(document: &mut Value, patch: &Patch) -> SyncResult<()> {
    json_patch::patch(document, patch).map_err(|e| SyncError::PatchFailed(e.to_string()))
}

pub fn calculate_checksum(value: &Value) -> String {
    let json_string = serde_json::to_string(value).unwrap();
    let mut hasher = Sha256::new();
    hasher.update(json_string.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn merge_patches(patch1: &Patch, patch2: &Patch) -> Patch {
    let mut operations = patch1.0.clone();
    operations.extend(patch2.0.clone());
    Patch(operations)
}

/// Compute a reverse patch that can undo the given forward patch
/// Requires the original document state before the patch was applied
pub fn compute_reverse_patch(original: &Value, forward_patch: &Patch) -> SyncResult<Patch> {
    // Apply the forward patch to get the new state
    let mut new_state = original.clone();
    apply_patch(&mut new_state, forward_patch)?;

    // The reverse patch is the diff from new state back to original
    let reverse_patch = create_patch(&new_state, original)?;
    Ok(reverse_patch)
}

/// Transform two patches using Operational Transformation
///
/// # OT Strategy (MVP)
/// The OT algorithm handles:
/// - Array index adjustments when operations affect the same array
/// - Conflict detection when operations target the same path
/// - Pass-through for Test operations (read-only)
/// - Conflict marking for Move/Copy operations (Post-MVP)
///
/// # Conflict Resolution
/// When conflicts are detected, both operations are returned unchanged.
/// The caller must use `ConflictResolver` or timestamps to decide which wins.
///
/// # Example
/// ```
/// use replicant_core::patches::{transform_patches, TransformStrategy};
/// use json_patch::{Patch, PatchOperation, AddOperation};
/// use serde_json::json;
///
/// let local = Patch(vec![
///     PatchOperation::Add(AddOperation {
///         path: "/items/2".into(),
///         value: json!("local item"),
///     })
/// ]);
///
/// let remote = Patch(vec![
///     PatchOperation::Add(AddOperation {
///         path: "/items/5".into(),
///         value: json!("remote item"),
///     })
/// ]);
///
/// let (local_t, remote_t) = transform_patches(&local, &remote, TransformStrategy::Operational).unwrap();
/// // remote index adjusted from 5 to 6 to account for local's addition
/// ```
pub fn transform_patches(
    local: &Patch,
    remote: &Patch,
    strategy: TransformStrategy,
) -> SyncResult<(Patch, Patch)> {
    match strategy {
        TransformStrategy::LastWriteWins => {
            // Simple strategy: remote wins
            Ok((Patch(vec![]), remote.clone()))
        }
        TransformStrategy::Operational => transform_operations(&local.0, &remote.0),
    }
}

pub enum TransformStrategy {
    /// Remote patch wins, local patch is discarded
    LastWriteWins,
    /// Use Operational Transformation to merge non-conflicting changes
    Operational,
}

/// Transform arrays of patch operations pairwise
///
/// This implements the OT algorithm by transforming each pair of operations
/// from local and remote patches.
fn transform_operations(
    local_ops: &[PatchOperation],
    remote_ops: &[PatchOperation],
) -> SyncResult<(Patch, Patch)> {
    // For MVP: single operation patches (most common case)
    // Full implementation would handle multiple operations with proper ordering

    if local_ops.is_empty() {
        return Ok((Patch(vec![]), Patch(remote_ops.to_vec())));
    }

    if remote_ops.is_empty() {
        return Ok((Patch(local_ops.to_vec()), Patch(vec![])));
    }

    // Transform each pair of operations
    let mut transformed_local = Vec::new();
    let mut transformed_remote = Vec::new();

    for local_op in local_ops {
        for remote_op in remote_ops {
            let (l_result, r_result) = transform_operation_pair(local_op, remote_op)?;

            if let Some(l_op) = l_result {
                if !transformed_local.contains(&l_op) {
                    transformed_local.push(l_op);
                }
            }

            if let Some(r_op) = r_result {
                if !transformed_remote.contains(&r_op) {
                    transformed_remote.push(r_op);
                }
            }
        }
    }

    Ok((Patch(transformed_local), Patch(transformed_remote)))
}
