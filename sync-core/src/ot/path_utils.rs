//! Path manipulation utilities for JSON Pointer paths (RFC 6901)

use crate::ot::types::{ParsedPath, PathRelation, PathSegment};
use crate::SyncError;

/// Parse JSON Pointer into segments
///
/// # Examples
/// ```
/// # use sync_core::ot::parse_path;
/// let path = parse_path("/foo/bar").unwrap();
/// assert_eq!(path.segments.len(), 2);
/// ```
pub fn parse_path(path: &str) -> Result<ParsedPath, SyncError> {
    if path.is_empty() {
        return Err(SyncError::InvalidOperation("Empty path".to_string()));
    }

    if !path.starts_with('/') {
        return Err(SyncError::InvalidOperation(format!(
            "Path must start with /: {}",
            path
        )));
    }

    // Root path
    if path == "/" {
        return Ok(ParsedPath {
            raw: path.to_string(),
            segments: Vec::new(),
        });
    }

    let mut segments = Vec::new();

    // Split on / and process each segment
    for segment in path[1..].split('/') {
        // Unescape ~0 -> ~ and ~1 -> / (must be in this order!)
        let unescaped = segment.replace("~1", "/").replace("~0", "~");

        // Try to parse as array index
        if let Ok(index) = unescaped.parse::<usize>() {
            segments.push(PathSegment::Array(index));
        } else {
            segments.push(PathSegment::Object(unescaped));
        }
    }

    Ok(ParsedPath {
        raw: path.to_string(),
        segments,
    })
}

/// Extract the last array index from a path, if present
///
/// # Examples
/// ```
/// # use sync_core::ot::extract_array_index;
/// assert_eq!(extract_array_index("/items/5"), Some(5));
/// assert_eq!(extract_array_index("/users/0/posts/3"), Some(3));
/// assert_eq!(extract_array_index("/title"), None);
/// ```
pub fn extract_array_index(path: &str) -> Option<usize> {
    parse_path(path)
        .ok()?
        .segments
        .into_iter()
        .rev()
        .find_map(|seg| match seg {
            PathSegment::Array(idx) => Some(idx),
            _ => None,
        })
}

/// Adjust array index in path by delta
///
/// Only adjusts if the current index matches target_index.
///
/// # Examples
/// ```
/// # use sync_core::ot::adjust_array_index;
/// assert_eq!(adjust_array_index("/items/5", 5, 1).unwrap(), "/items/6");
/// assert_eq!(adjust_array_index("/items/5", 3, 1).unwrap(), "/items/5"); // No change
/// assert!(adjust_array_index("/items/2", 2, -3).is_err()); // Underflow
/// ```
pub fn adjust_array_index(
    path: &str,
    target_index: usize,
    delta: isize,
) -> Result<String, SyncError> {
    let parsed = parse_path(path)?;

    // Find last array index that matches target
    let last_array_idx = parsed
        .segments
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, seg)| match seg {
            PathSegment::Array(idx) if *idx == target_index => Some(i),
            _ => None,
        });

    let Some(idx_position) = last_array_idx else {
        // No matching index, return unchanged
        return Ok(path.to_string());
    };

    // Calculate new index
    let old_index = target_index as isize;
    let new_index = old_index + delta;

    if new_index < 0 {
        return Err(SyncError::InvalidOperation(format!(
            "Index adjustment would be negative: {} + {} = {}",
            old_index, delta, new_index
        )));
    }

    // Rebuild path with adjusted index
    let mut new_segments = parsed.segments;
    new_segments[idx_position] = PathSegment::Array(new_index as usize);

    Ok(reconstruct_path(&new_segments))
}

/// Helper: Reconstruct path from segments
fn reconstruct_path(segments: &[PathSegment]) -> String {
    if segments.is_empty() {
        return "/".to_string();
    }

    let mut path = String::new();
    for segment in segments {
        path.push('/');
        match segment {
            PathSegment::Object(key) => {
                // Re-escape special characters (~ must be escaped before /)
                let escaped = key.replace('~', "~0").replace('/', "~1");
                path.push_str(&escaped);
            }
            PathSegment::Array(idx) => {
                path.push_str(&idx.to_string());
            }
        }
    }
    path
}

/// Determine relationship between two paths
///
/// # Examples
/// ```
/// # use sync_core::ot::{compare_paths, PathRelation};
/// assert_eq!(compare_paths("/a", "/a"), PathRelation::Same);
/// assert_eq!(compare_paths("/a", "/a/b"), PathRelation::Parent);
/// assert_eq!(compare_paths("/a/b", "/a/c"), PathRelation::Sibling);
/// ```
pub fn compare_paths(path1: &str, path2: &str) -> PathRelation {
    if path1 == path2 {
        return PathRelation::Same;
    }

    // Check parent-child relationships
    if path2.starts_with(&format!("{}/", path1)) {
        return PathRelation::Parent; // path1 is parent of path2
    }

    if path1.starts_with(&format!("{}/", path2)) {
        return PathRelation::Child; // path1 is child of path2
    }

    // Check if siblings (same parent)
    // This includes root-level paths, which is important for root-level arrays
    // e.g., /0 and /1 on a root array are siblings and need index adjustment
    let parent1 = get_parent_path(path1);
    let parent2 = get_parent_path(path2);

    match (parent1, parent2) {
        (Some(p1), Some(p2)) if p1 == p2 => {
            // Same parent = siblings (including root level)
            PathRelation::Sibling
        }
        _ => PathRelation::Unrelated,
    }
}

/// Get parent path, or None if root
///
/// # Examples
/// ```
/// # use sync_core::ot::get_parent_path;
/// assert_eq!(get_parent_path("/a/b/c"), Some("/a/b".to_string()));
/// assert_eq!(get_parent_path("/a"), Some("/".to_string()));
/// assert_eq!(get_parent_path("/"), None);
/// ```
pub fn get_parent_path(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }

    path.rfind('/').and_then(|pos| {
        if pos == 0 {
            Some("/".to_string())
        } else {
            Some(path[..pos].to_string())
        }
    })
}

/// Check if two paths conflict (one would affect the other)
///
/// Conflicts occur when:
/// - Same path
/// - Parent-child relationship (modifying parent affects child)
///
/// # Examples
/// ```
/// # use sync_core::ot::paths_conflict;
/// assert!(paths_conflict("/a", "/a"));
/// assert!(paths_conflict("/a", "/a/b")); // Parent-child
/// assert!(!paths_conflict("/a", "/b")); // Unrelated
/// ```
pub fn paths_conflict(path1: &str, path2: &str) -> bool {
    matches!(
        compare_paths(path1, path2),
        PathRelation::Same | PathRelation::Parent | PathRelation::Child
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Parse Path Tests ===

    #[test]
    fn test_parse_simple_object_path() {
        let path = parse_path("/foo/bar").unwrap();
        assert_eq!(path.segments.len(), 2);
        assert!(matches!(&path.segments[0], PathSegment::Object(s) if s == "foo"));
        assert!(matches!(&path.segments[1], PathSegment::Object(s) if s == "bar"));
    }

    #[test]
    fn test_parse_array_path() {
        let path = parse_path("/items/0/name").unwrap();
        assert_eq!(path.segments.len(), 3);
        assert!(matches!(&path.segments[0], PathSegment::Object(s) if s == "items"));
        assert!(matches!(&path.segments[1], PathSegment::Array(0)));
        assert!(matches!(&path.segments[2], PathSegment::Object(s) if s == "name"));
    }

    #[test]
    fn test_parse_root_path() {
        let path = parse_path("/").unwrap();
        assert!(path.segments.is_empty());
    }

    #[test]
    fn test_parse_escaped_characters() {
        let path = parse_path("/foo~0bar/baz~1qux").unwrap();
        assert!(matches!(&path.segments[0], PathSegment::Object(s) if s == "foo~bar"));
        assert!(matches!(&path.segments[1], PathSegment::Object(s) if s == "baz/qux"));
    }

    #[test]
    fn test_parse_invalid_no_leading_slash() {
        assert!(parse_path("foo/bar").is_err());
    }

    #[test]
    fn test_parse_invalid_empty() {
        assert!(parse_path("").is_err());
    }

    // === Extract Array Index Tests ===

    #[test]
    fn test_extract_array_index_simple() {
        assert_eq!(extract_array_index("/items/0"), Some(0));
        assert_eq!(extract_array_index("/items/42"), Some(42));
        assert_eq!(extract_array_index("/items/999"), Some(999));
    }

    #[test]
    fn test_extract_array_index_nested() {
        // Should return the LAST array index
        assert_eq!(extract_array_index("/users/5/posts/3"), Some(3));
        assert_eq!(extract_array_index("/data/0/items/1/tags/2"), Some(2));
    }

    #[test]
    fn test_extract_array_index_none() {
        assert_eq!(extract_array_index("/title"), None);
        assert_eq!(extract_array_index("/metadata/author"), None);
        assert_eq!(extract_array_index("/"), None);
    }

    #[test]
    fn test_extract_array_index_not_a_number() {
        assert_eq!(extract_array_index("/items/abc"), None);
        assert_eq!(extract_array_index("/items/-"), None); // Special append marker
    }

    // === Adjust Array Index Tests ===

    #[test]
    fn test_adjust_array_index_increment() {
        assert_eq!(adjust_array_index("/items/5", 5, 1).unwrap(), "/items/6");
        assert_eq!(adjust_array_index("/items/0", 0, 3).unwrap(), "/items/3");
    }

    #[test]
    fn test_adjust_array_index_decrement() {
        assert_eq!(adjust_array_index("/items/5", 5, -2).unwrap(), "/items/3");
        assert_eq!(adjust_array_index("/items/10", 10, -1).unwrap(), "/items/9");
    }

    #[test]
    fn test_adjust_array_index_underflow() {
        assert!(adjust_array_index("/items/2", 2, -3).is_err());
        assert!(adjust_array_index("/items/0", 0, -1).is_err());
    }

    #[test]
    fn test_adjust_array_index_no_match() {
        // Index doesn't match target, no change
        assert_eq!(adjust_array_index("/items/3", 5, 1).unwrap(), "/items/3");
    }

    #[test]
    fn test_adjust_array_index_nested() {
        assert_eq!(
            adjust_array_index("/data/items/5/name", 5, 1).unwrap(),
            "/data/items/6/name"
        );
    }

    #[test]
    fn test_adjust_array_index_non_array_path() {
        assert_eq!(adjust_array_index("/title", 0, 1).unwrap(), "/title");
    }

    // === Compare Paths Tests ===

    #[test]
    fn test_compare_paths_same() {
        assert_eq!(compare_paths("/a", "/a"), PathRelation::Same);
        assert_eq!(compare_paths("/a/b/c", "/a/b/c"), PathRelation::Same);
        assert_eq!(compare_paths("/", "/"), PathRelation::Same);
    }

    #[test]
    fn test_compare_paths_parent_child() {
        assert_eq!(compare_paths("/a", "/a/b"), PathRelation::Parent);
        assert_eq!(compare_paths("/a/b", "/a"), PathRelation::Child);
        assert_eq!(compare_paths("/items", "/items/0"), PathRelation::Parent);
        assert_eq!(
            compare_paths("/users/0/posts/1", "/users/0"),
            PathRelation::Child
        );
    }

    #[test]
    fn test_compare_paths_siblings() {
        assert_eq!(compare_paths("/a/b", "/a/c"), PathRelation::Sibling);
        assert_eq!(compare_paths("/items/0", "/items/1"), PathRelation::Sibling);
        assert_eq!(compare_paths("/x/y/z", "/x/y/w"), PathRelation::Sibling);
    }

    #[test]
    fn test_compare_paths_root_level_array_siblings() {
        // Root-level array operations: paths /0, /1, /2 are siblings
        // This is critical for OT on root-level arrays
        assert_eq!(compare_paths("/0", "/1"), PathRelation::Sibling);
        assert_eq!(compare_paths("/0", "/2"), PathRelation::Sibling);
        assert_eq!(compare_paths("/10", "/20"), PathRelation::Sibling);

        // All have parent "/" so they're siblings
        // Transformation logic will check if they're array indices
    }

    #[test]
    fn test_compare_paths_unrelated() {
        // Root-level paths are actually siblings (same parent "/")
        // This was previously considered "unrelated" but that's incorrect for arrays
        assert_eq!(compare_paths("/a", "/b"), PathRelation::Sibling);

        // These are truly unrelated (different parents)
        assert_eq!(
            compare_paths("/users/1", "/posts/1"),
            PathRelation::Unrelated
        );
        assert_eq!(compare_paths("/x/y", "/a/b/c"), PathRelation::Unrelated);
    }

    // === Get Parent Path Tests ===

    #[test]
    fn test_get_parent_path_normal() {
        assert_eq!(get_parent_path("/a/b/c"), Some("/a/b".to_string()));
        assert_eq!(get_parent_path("/items/0"), Some("/items".to_string()));
        assert_eq!(get_parent_path("/x"), Some("/".to_string()));
    }

    #[test]
    fn test_get_parent_path_root() {
        assert_eq!(get_parent_path("/"), None);
    }

    // === Paths Conflict Tests ===

    #[test]
    fn test_paths_conflict_same() {
        assert!(paths_conflict("/a", "/a"));
        assert!(paths_conflict("/items/0", "/items/0"));
    }

    #[test]
    fn test_paths_conflict_parent_child() {
        assert!(paths_conflict("/a", "/a/b")); // Parent-child
        assert!(paths_conflict("/a/b", "/a")); // Child-parent
    }

    #[test]
    fn test_paths_no_conflict_siblings() {
        assert!(!paths_conflict("/a/b", "/a/c"));
        assert!(!paths_conflict("/items/0", "/items/1"));
    }

    #[test]
    fn test_paths_no_conflict_unrelated() {
        assert!(!paths_conflict("/a", "/b"));
        assert!(!paths_conflict("/users", "/posts"));
    }

    // === Reconstruct Path Tests ===

    #[test]
    fn test_reconstruct_path_simple() {
        let segments = vec![
            PathSegment::Object("foo".to_string()),
            PathSegment::Object("bar".to_string()),
        ];
        assert_eq!(reconstruct_path(&segments), "/foo/bar");
    }

    #[test]
    fn test_reconstruct_path_with_array() {
        let segments = vec![
            PathSegment::Object("items".to_string()),
            PathSegment::Array(5),
            PathSegment::Object("name".to_string()),
        ];
        assert_eq!(reconstruct_path(&segments), "/items/5/name");
    }

    #[test]
    fn test_reconstruct_path_with_escapes() {
        let segments = vec![
            PathSegment::Object("foo~bar".to_string()),
            PathSegment::Object("baz/qux".to_string()),
        ];
        assert_eq!(reconstruct_path(&segments), "/foo~0bar/baz~1qux");
    }

    #[test]
    fn test_reconstruct_path_empty() {
        let segments: Vec<PathSegment> = vec![];
        assert_eq!(reconstruct_path(&segments), "/");
    }
}
