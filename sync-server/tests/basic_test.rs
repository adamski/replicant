#[cfg(test)]
mod tests {
    use serde_json::json;
    use sync_core::patches::{apply_patch, calculate_checksum, create_patch};

    #[test]
    fn test_json_patch() {
        let from = json!({
            "name": "John",
            "age": 30
        });

        let to = json!({
            "name": "John",
            "age": 31,
            "city": "New York"
        });

        let patch = create_patch(&from, &to).unwrap();
        let mut doc = from.clone();
        apply_patch(&mut doc, &patch).unwrap();

        assert_eq!(doc, to);
    }

    #[test]
    fn test_checksum() {
        let data = json!({
            "test": "data"
        });

        let checksum1 = calculate_checksum(&data);
        let checksum2 = calculate_checksum(&data);

        assert_eq!(checksum1, checksum2);
    }
}
