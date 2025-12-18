//! Integration tests for Operational Transformation
//!
//! These tests demonstrate OT with real-world scenarios showing:
//! - Original JSON document
//! - User A's changes
//! - User B's changes
//! - Final merged result

use json_patch::{AddOperation, Patch, PatchOperation, RemoveOperation, ReplaceOperation};
use replicant_core::patches::{apply_patch, transform_patches, TransformStrategy};
use serde_json::json;

#[test]
fn test_task_list_concurrent_additions() {
    // SCENARIO: Two users add tasks to different positions in a shared task list

    // Original document
    let original = json!({
        "tasks": [
            {"id": 1, "text": "Buy groceries", "done": false},
            {"id": 2, "text": "Walk dog", "done": false},
            {"id": 3, "text": "Write report", "done": false}
        ]
    });

    // User A (offline): Adds urgent task at position 0
    let user_a_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/tasks/0".into(),
        value: json!({"id": 4, "text": "URGENT: Call client", "done": false}),
    })]);

    // User B (offline): Adds task at position 2
    let user_b_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/tasks/2".into(),
        value: json!({"id": 5, "text": "Review PRs", "done": false}),
    })]);

    // When they sync, transform the patches
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Apply both patches to get final result
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    // Expected final state
    let expected = json!({
        "tasks": [
            {"id": 4, "text": "URGENT: Call client", "done": false},  // A's addition at 0
            {"id": 1, "text": "Buy groceries", "done": false},
            {"id": 2, "text": "Walk dog", "done": false},
            {"id": 5, "text": "Review PRs", "done": false},            // B's addition at 3 (adjusted from 2)
            {"id": 3, "text": "Write report", "done": false}
        ]
    });

    assert_eq!(result, expected);
    println!("\n=== TASK LIST CONCURRENT ADDITIONS ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A adds at index 0");
    println!("User B adds at index 2");
    println!(
        "\nFinal result: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}

#[test]
fn test_task_list_add_and_remove() {
    // SCENARIO: One user adds a task while another removes a different task

    // Original document
    let original = json!({
        "tasks": [
            {"id": 1, "text": "Task 1", "done": false},
            {"id": 2, "text": "Task 2", "done": false},
            {"id": 3, "text": "Task 3", "done": false},
            {"id": 4, "text": "Task 4", "done": false},
            {"id": 5, "text": "Task 5", "done": false}
        ]
    });

    // User A: Adds new task at position 1
    let user_a_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/tasks/1".into(),
        value: json!({"id": 6, "text": "New Task", "done": false}),
    })]);

    // User B: Removes task at position 3
    let user_b_patch = Patch(vec![PatchOperation::Remove(RemoveOperation {
        path: "/tasks/3".into(),
    })]);

    // Transform patches
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Apply both patches
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    // Expected: Task added at 1, Task 4 removed (was at index 3, now at 4 after add)
    let expected = json!({
        "tasks": [
            {"id": 1, "text": "Task 1", "done": false},
            {"id": 6, "text": "New Task", "done": false},  // A's addition
            {"id": 2, "text": "Task 2", "done": false},
            {"id": 3, "text": "Task 3", "done": false},
            // Task 4 removed (B's deletion, adjusted from index 3 to 4)
            {"id": 5, "text": "Task 5", "done": false}
        ]
    });

    assert_eq!(result, expected);
    println!("\n=== TASK LIST ADD AND REMOVE ===");
    println!(
        "Original (5 tasks): {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A adds at index 1");
    println!("User B removes at index 3");
    println!(
        "\nFinal result (5 tasks): {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}

#[test]
fn test_shopping_list_concurrent_edits() {
    // SCENARIO: Two shoppers editing a shared shopping list

    // Original document
    let original = json!({
        "items": [
            "Milk",
            "Bread",
            "Eggs",
            "Butter"
        ]
    });

    // Shopper A: Adds "Coffee" at position 2
    let shopper_a_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/items/2".into(),
        value: json!("Coffee"),
    })]);

    // Shopper B: Adds "Cheese" at position 4
    let shopper_b_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/items/4".into(),
        value: json!("Cheese"),
    })]);

    // Transform
    let (a_transformed, b_transformed) = transform_patches(
        &shopper_a_patch,
        &shopper_b_patch,
        TransformStrategy::Operational,
    )
    .unwrap();

    // Apply both
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    let expected = json!({
        "items": [
            "Milk",
            "Bread",
            "Coffee",   // A's addition at 2
            "Eggs",
            "Butter",
            "Cheese"    // B's addition at 5 (adjusted from 4)
        ]
    });

    assert_eq!(result, expected);
    println!("\n=== SHOPPING LIST CONCURRENT EDITS ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nShopper A adds 'Coffee' at index 2");
    println!("Shopper B adds 'Cheese' at index 4");
    println!(
        "\nFinal result: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}

#[test]
fn test_document_settings_concurrent_updates() {
    // SCENARIO: Two users update different settings simultaneously

    // Original document
    let original = json!({
        "settings": {
            "theme": "light",
            "fontSize": 14,
            "notifications": true
        }
    });

    // User A: Changes theme
    let user_a_patch = Patch(vec![PatchOperation::Replace(ReplaceOperation {
        path: "/settings/theme".into(),
        value: json!("dark"),
    })]);

    // User B: Changes font size
    let user_b_patch = Patch(vec![PatchOperation::Replace(ReplaceOperation {
        path: "/settings/fontSize".into(),
        value: json!(16),
    })]);

    // Transform (no conflict - different paths)
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Apply both
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    let expected = json!({
        "settings": {
            "theme": "dark",         // A's change
            "fontSize": 16,          // B's change
            "notifications": true
        }
    });

    assert_eq!(result, expected);
    println!("\n=== DOCUMENT SETTINGS CONCURRENT UPDATES ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A changes theme to 'dark'");
    println!("User B changes fontSize to 16");
    println!(
        "\nFinal result (both changes applied): {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}

#[test]
fn test_conflicting_edits_same_field() {
    // SCENARIO: Two users change the same field (conflict!)

    // Original document
    let original = json!({
        "document": {
            "title": "Original Title",
            "content": "Original content"
        }
    });

    // User A: Changes title
    let user_a_patch = Patch(vec![PatchOperation::Replace(ReplaceOperation {
        path: "/document/title".into(),
        value: json!("User A's Title"),
    })]);

    // User B: Also changes title
    let user_b_patch = Patch(vec![PatchOperation::Replace(ReplaceOperation {
        path: "/document/title".into(),
        value: json!("User B's Title"),
    })]);

    // Transform - both operations returned (CONFLICT)
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Check that both patches were returned (indicates conflict)
    assert!(
        !a_transformed.0.is_empty(),
        "User A's patch should be returned"
    );
    assert!(
        !b_transformed.0.is_empty(),
        "User B's patch should be returned"
    );

    println!("\n=== CONFLICTING EDITS (Same Field) ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A changes title to 'User A's Title'");
    println!("User B changes title to 'User B's Title'");
    println!("\n⚠️  CONFLICT DETECTED!");
    println!("Both operations returned unchanged.");
    println!("Caller must use timestamps or ConflictResolver to decide winner.");

    // Simulate resolution: User B wins (newer timestamp)
    let mut result_b_wins = original.clone();
    apply_patch(&mut result_b_wins, &b_transformed).unwrap();

    println!(
        "\nIf User B wins (newer): {}",
        serde_json::to_string_pretty(&result_b_wins).unwrap()
    );
}

#[test]
fn test_array_multiple_removals() {
    // SCENARIO: Two users remove different items from an array

    // Original document
    let original = json!({
        "participants": [
            "Alice",
            "Bob",
            "Charlie",
            "David",
            "Eve"
        ]
    });

    // User A: Removes "Bob" (index 1)
    let user_a_patch = Patch(vec![PatchOperation::Remove(RemoveOperation {
        path: "/participants/1".into(),
    })]);

    // User B: Removes "David" (index 3)
    let user_b_patch = Patch(vec![PatchOperation::Remove(RemoveOperation {
        path: "/participants/3".into(),
    })]);

    // Transform
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Apply both
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    let expected = json!({
        "participants": [
            "Alice",
            // "Bob" removed by A
            "Charlie",
            // "David" removed by B (index adjusted from 3 to 2 after A's removal)
            "Eve"
        ]
    });

    assert_eq!(result, expected);
    println!("\n=== ARRAY MULTIPLE REMOVALS ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A removes 'Bob' at index 1");
    println!("User B removes 'David' at index 3");
    println!(
        "\nFinal result: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}

#[test]
fn test_nested_object_updates() {
    // SCENARIO: Updates to nested structures

    // Original document
    let original = json!({
        "user": {
            "profile": {
                "name": "Alice",
                "email": "alice@example.com"
            },
            "settings": {
                "theme": "light",
                "language": "en"
            }
        }
    });

    // User A: Updates profile name
    let user_a_patch = Patch(vec![PatchOperation::Replace(ReplaceOperation {
        path: "/user/profile/name".into(),
        value: json!("Alice Smith"),
    })]);

    // User B: Updates settings theme
    let user_b_patch = Patch(vec![PatchOperation::Replace(ReplaceOperation {
        path: "/user/settings/theme".into(),
        value: json!("dark"),
    })]);

    // Transform (no conflict - different branches)
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Apply both
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    let expected = json!({
        "user": {
            "profile": {
                "name": "Alice Smith",           // A's change
                "email": "alice@example.com"
            },
            "settings": {
                "theme": "dark",                 // B's change
                "language": "en"
            }
        }
    });

    assert_eq!(result, expected);
    println!("\n=== NESTED OBJECT UPDATES ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A updates /user/profile/name");
    println!("User B updates /user/settings/theme");
    println!(
        "\nFinal result (both changes): {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}

#[test]
fn test_root_level_array() {
    // SCENARIO: Document root is an array

    // Original document (root is array)
    let original = json!(["Item 1", "Item 2", "Item 3"]);

    // User A: Adds at index 0
    let user_a_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/0".into(),
        value: json!("First Item"),
    })]);

    // User B: Adds at index 2
    let user_b_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/2".into(),
        value: json!("Middle Item"),
    })]);

    // Transform (root-level array paths are siblings!)
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Apply both
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    let expected = json!([
        "First Item", // A's addition at 0
        "Item 1",
        "Item 2",
        "Middle Item", // B's addition at 3 (adjusted from 2)
        "Item 3"
    ]);

    assert_eq!(result, expected);
    println!("\n=== ROOT LEVEL ARRAY ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A adds at index 0 (root path /0)");
    println!("User B adds at index 2 (root path /2)");
    println!(
        "\nFinal result: {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}

#[test]
fn test_same_index_additions() {
    // SCENARIO: Both users add at the exact same index

    // Original document
    let original = json!({
        "queue": [
            "Job 1",
            "Job 2",
            "Job 3"
        ]
    });

    // User A: Adds at index 1
    let user_a_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/queue/1".into(),
        value: json!("User A's Job"),
    })]);

    // User B: Also adds at index 1
    let user_b_patch = Patch(vec![PatchOperation::Add(AddOperation {
        path: "/queue/1".into(),
        value: json!("User B's Job"),
    })]);

    // Transform (both succeed with adjusted indices)
    let (a_transformed, b_transformed) =
        transform_patches(&user_a_patch, &user_b_patch, TransformStrategy::Operational).unwrap();

    // Apply both
    let mut result = original.clone();
    apply_patch(&mut result, &a_transformed).unwrap();
    apply_patch(&mut result, &b_transformed).unwrap();

    let expected = json!({
        "queue": [
            "Job 1",
            "User A's Job",     // A at index 1
            "User B's Job",     // B at index 2 (adjusted from 1)
            "Job 2",
            "Job 3"
        ]
    });

    assert_eq!(result, expected);
    println!("\n=== SAME INDEX ADDITIONS ===");
    println!(
        "Original: {}",
        serde_json::to_string_pretty(&original).unwrap()
    );
    println!("\nUser A adds at index 1");
    println!("User B also adds at index 1");
    println!(
        "\nFinal result (both added, B adjusted to 2): {}",
        serde_json::to_string_pretty(&result).unwrap()
    );
}
