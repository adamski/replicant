# Authentication and Session Management

## Production User/Session Flow

In production, the user/session flow would typically work like this:

### 1. User Registration (One-time)
```
Client → POST /api/auth/register
        {email, password}
         ↓
Server → Hash password
       → INSERT user into DB
       → Return user_id + auth_token
```

### 2. User Login (When starting a session)
```
Client → POST /api/auth/login
        {email, password}
         ↓
Server → Verify password against DB
       → Generate session token (JWT/UUID)
       → Store session (Redis/DB/Memory)
       → Return user_id + session_token
```

### 3. WebSocket Authentication
```
Client → Connect WebSocket
       → Send: Authenticate {user_id, session_token}
         ↓
Server → Verify session token
       → Check token expiry
       → Load user from DB (if needed)
       → Cache in memory for performance
       → Return: AuthSuccess
```

### 4. Session Management
- **Short-lived sessions**: Expire after inactivity (e.g., 24 hours)
- **Refresh tokens**: Long-lived tokens to get new session tokens
- **Revocation**: Ability to invalidate sessions (logout, security)

### 5. Typical Production Implementation

```rust
pub struct AuthState {
    sessions: Arc<DashMap<Uuid, AuthSession>>,  // In-memory cache
    db: Arc<ServerDatabase>,
    redis: Arc<RedisClient>,  // Optional: distributed sessions
}

pub async fn verify_token(&self, user_id: &Uuid, token: &str) -> Result<bool> {
    // 1. Check in-memory cache first (fast path)
    if let Some(session) = self.sessions.get(user_id) {
        if session.token == token && !session.is_expired() {
            return Ok(true);
        }
    }
    
    // 2. Check Redis/distributed cache
    if let Some(session) = self.redis.get_session(user_id).await? {
        if session.verify_token(token) {
            // Cache locally
            self.sessions.insert(*user_id, session);
            return Ok(true);
        }
    }
    
    // 3. Check database (slow path)
    if let Some(user) = self.db.get_user_with_valid_session(user_id, token).await? {
        // Create new session
        let session = AuthSession::new(user_id, token);
        
        // Cache in Redis and memory
        self.redis.set_session(&session).await?;
        self.sessions.insert(*user_id, session);
        
        return Ok(true);
    }
    
    Ok(false)
}
```

### 6. Security Considerations

#### Token Types
- **JWT tokens**: Self-contained, stateless
- **Opaque tokens**: Random UUIDs, require server lookup
- **Refresh tokens**: For getting new access tokens

#### Storage
- Never store plain passwords
- Use secure hashing (Argon2, bcrypt)
- Session tokens in httpOnly cookies or secure storage

#### Validation
- Check token expiry
- Verify token signatures (for JWT)
- Rate limiting on auth endpoints

### 7. Our Current Implementation

Our current implementation is a simplified version:
- **Demo mode**: Auto-creates users with any token
- **No registration**: Users are created on first auth
- **Simple sessions**: In-memory only, no expiry
- **Basic security**: Argon2 hashing for tokens

For production, you'd want to:
1. Add proper registration/login endpoints
2. Implement session expiry
3. Use distributed session storage (Redis)
4. Add refresh token mechanism
5. Implement proper user management (password reset, etc.)

The key difference is that production systems separate **authentication** (who are you?) from **authorization** (what can you do?), and have proper user lifecycle management.

## Additional Considerations

### Multi-Factor Authentication (MFA)
- TOTP (Time-based One-Time Passwords)
- SMS verification
- Email verification
- Hardware tokens

### OAuth2 / OpenID Connect
- Third-party authentication (Google, GitHub, etc.)
- Standardized token format
- Delegated authorization

### Rate Limiting
- Prevent brute force attacks
- Per-IP and per-user limits
- Exponential backoff for failed attempts

### Audit Logging
- Track all authentication events
- Failed login attempts
- Session creation/deletion
- Permission changes

### GDPR Compliance
- Right to be forgotten
- Data portability
- Consent management
- Privacy by design