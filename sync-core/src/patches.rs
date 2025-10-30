use crate::errors::SyncError;
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

pub fn transform_patches(
    local: &Patch,
    remote: &Patch,
    strategy: TransformStrategy,
) -> SyncResult<(Patch, Patch)> {
    // Operational transformation implementation
    match strategy {
        TransformStrategy::LastWriteWins => {
            // Simple strategy: remote wins
            Ok((Patch(vec![]), remote.clone()))
        }
        TransformStrategy::Operational => {
            // Complex OT algorithm would go here
            transform_operations(&local.0, &remote.0)
        }
    }
}

pub enum TransformStrategy {
    LastWriteWins,
    Operational,
}

fn transform_operations(
    local_ops: &[PatchOperation],
    remote_ops: &[PatchOperation],
) -> SyncResult<(Patch, Patch)> {
    // Simplified OT - would need full implementation
    // This is a placeholder for the actual OT algorithm
    let transformed_local = Patch(local_ops.to_vec());
    let transformed_remote = Patch(remote_ops.to_vec());
    Ok((transformed_local, transformed_remote))
}
