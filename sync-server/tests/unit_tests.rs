use sync_server::auth::AuthState;

#[test]
fn test_token_generation_format() {
    let token = AuthState::generate_auth_token();
    
    // Should be a valid UUID
    assert_eq!(token.len(), 36);
    assert!(uuid::Uuid::parse_str(&token).is_ok());
}

#[test]
fn test_token_hashing() {
    let token = "test-token";
    let hash1 = AuthState::hash_token(token).unwrap();
    let hash2 = AuthState::hash_token(token).unwrap();
    
    // Same token should produce different hashes (due to salt)
    assert_ne!(hash1, hash2);
    
    // Both hashes should verify correctly
    assert!(AuthState::verify_token_hash(token, &hash1).unwrap());
    assert!(AuthState::verify_token_hash(token, &hash2).unwrap());
}

#[test]
fn test_invalid_hash_verification() {
    let token = "test-token";
    
    // Invalid hash format should return error
    assert!(AuthState::verify_token_hash(token, "invalid-hash").is_err());
    
    // Wrong token should fail verification
    let hash = AuthState::hash_token(token).unwrap();
    assert!(!AuthState::verify_token_hash("wrong-token", &hash).unwrap());
}