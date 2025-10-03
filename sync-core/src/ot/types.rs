//! Core types for Operational Transformation

/// Relationship between two JSON Pointer paths
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRelation {
    /// Paths are identical
    Same,
    /// First path is parent of second
    Parent,
    /// First path is child of second
    Child,
    /// Paths share same parent (e.g., /a/b and /a/c)
    Sibling,
    /// Paths are unrelated
    Unrelated,
}

/// Represents a parsed JSON Pointer segment
#[derive(Debug, Clone, PartialEq)]
pub enum PathSegment {
    /// Object property key
    Object(String),
    /// Array index
    Array(usize),
}

/// Parsed JSON Pointer path with segments
#[derive(Debug, Clone)]
pub struct ParsedPath {
    /// Original path string
    pub raw: String,
    /// Parsed segments
    pub segments: Vec<PathSegment>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_relation_equality() {
        assert_eq!(PathRelation::Same, PathRelation::Same);
        assert_ne!(PathRelation::Parent, PathRelation::Child);
    }

    #[test]
    fn test_path_segment_equality() {
        assert_eq!(PathSegment::Object("foo".to_string()), PathSegment::Object("foo".to_string()));
        assert_eq!(PathSegment::Array(5), PathSegment::Array(5));
        assert_ne!(PathSegment::Object("foo".to_string()), PathSegment::Object("bar".to_string()));
        assert_ne!(PathSegment::Array(1), PathSegment::Array(2));
    }
}
