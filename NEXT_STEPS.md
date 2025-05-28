# Next Steps: Implementing Reliable Message Delivery

*Date: December 28, 2024*

## Priority Order

Based on our investigation, here's the recommended implementation order to fix the message delivery issues and make the system production-ready.

## 🚨 Phase 1: Fix Message Delivery (Critical)

**Goal**: Ensure all messages reach all clients reliably

### Step 1.1: Add Message Sequence Numbers
```rust
// sync-core/src/protocol.rs
pub enum ServerMessage {
    DocumentCreated { 
        document: Document,
        sequence: u64,        // Add global sequence number
        user_sequence: u64,   // Add per-user sequence number
    },
    // ... update other message types
}
```

### Step 1.2: Implement Client Acknowledgments
```rust
// sync-core/src/protocol.rs
pub enum ClientMessage {
    AckMessage {
        sequence: u64,
        user_sequence: u64,
    },
    // ... existing messages
}
```

### Step 1.3: Server-side Delivery Tracking
```rust
// sync-server/src/sync_handler.rs
struct PendingMessage {
    message: ServerMessage,
    sequence: u64,
    user_sequence: u64,
    timestamp: Instant,
    retry_count: u32,
}

struct SyncHandler {
    // ... existing fields
    pending_messages: Arc<DashMap<Uuid, Vec<PendingMessage>>>, // Per client
    user_sequences: Arc<DashMap<Uuid, AtomicU64>>, // Per user
}
```

### Step 1.4: Implement Retry Logic
- Track unacknowledged messages per client
- Retry with exponential backoff (100ms, 200ms, 400ms, 800ms, 1.6s)
- Maximum 5 retries before marking client as disconnected
- Clear acknowledged messages from pending queue

### Step 1.5: Fix Test Infrastructure
- Update integration tests to wait for ACKs
- Add helper to verify message delivery
- Test with packet loss simulation

## 📊 Phase 2: Improve Consistency (Important)

**Goal**: Ensure data consistency across all clients

### Step 2.1: Implement Read Repair
```rust
// sync-client/src/sync_engine.rs
pub async fn get_document_with_repair(&self, doc_id: Uuid) -> Result<Document, ClientError> {
    let local_doc = self.db.get_document(&doc_id).await?;
    
    // Request version check from server (lightweight)
    self.ws_client.send(ClientMessage::CheckVersion { 
        document_id: doc_id,
        vector_clock: local_doc.vector_clock.clone(),
    }).await?;
    
    Ok(local_doc) // Server will send update if needed
}
```

### Step 2.2: Add Version Checking Protocol
```rust
pub enum ClientMessage {
    CheckVersion {
        document_id: Uuid,
        vector_clock: VectorClock,
    },
}

pub enum ServerMessage {
    VersionMismatch {
        document_id: Uuid,
        server_vector_clock: VectorClock,
        action: VersionAction,
    },
}

pub enum VersionAction {
    PullFromServer,  // Client is behind
    PushToServer,    // Server is behind
    Conflict,        // Diverged - need resolution
}
```

### Step 2.3: Implement Periodic Sync
- Background task to check consistency every 30 seconds
- Use Merkle tree for efficient comparison
- Only sync documents with mismatched hashes

## 🧪 Phase 3: Robust Testing (Important)

**Goal**: Prevent regressions and ensure reliability

### Step 3.1: Create Test Utilities
```rust
// tests/common/mod.rs
pub struct ReliableTestClient {
    engine: SyncEngine,
    ack_tracker: Arc<Mutex<HashSet<u64>>>,
}

impl ReliableTestClient {
    // Wait for all pending ACKs
    pub async fn wait_for_sync(&self, timeout: Duration) -> Result<()> {
        // ...
    }
    
    // Verify eventual consistency
    pub async fn assert_has_documents(&self, expected: &[Document]) -> Result<()> {
        // ...
    }
}
```

### Step 3.2: Add Chaos Testing
```rust
pub struct ChaosProxy {
    drop_rate: f32,      // Randomly drop messages
    delay_ms: u64,       // Add latency
    reorder: bool,       // Reorder messages
}
```

### Step 3.3: Test Scenarios
1. **Rapid fire messages** - 100 documents in 1 second
2. **Network partition** - Client disconnects and reconnects
3. **Slow consumer** - Client processes messages slowly
4. **Byzantine behavior** - Client sends invalid messages

## 🚀 Phase 4: Performance Optimization (Nice to Have)

**Goal**: Scale to thousands of clients

### Step 4.1: Message Batching
- Batch multiple updates into single WebSocket frame
- Compress with zstd for large payloads
- Implement Nagle's algorithm for small messages

### Step 4.2: Connection Pooling
- Reuse WebSocket connections
- Implement connection multiplexing
- Add circuit breaker for failing clients

### Step 4.3: Monitoring
- Prometheus metrics for:
  - Message delivery rate
  - Retry rate
  - Convergence time
  - Active connections
- Grafana dashboards
- Alerting for high retry rates

## Implementation Timeline

### Week 1: Message Delivery
- [ ] Add sequence numbers to protocol
- [ ] Implement acknowledgments
- [ ] Add retry logic with backoff
- [ ] Update tests to verify delivery

### Week 2: Consistency
- [ ] Implement read repair
- [ ] Add version checking
- [ ] Create periodic sync task
- [ ] Test convergence scenarios

### Week 3: Testing & Optimization
- [ ] Create test utilities
- [ ] Add chaos testing
- [ ] Implement message batching
- [ ] Add monitoring

## Success Criteria

1. **All integration tests pass** with 100% reliability
2. **Convergence time < 2 seconds** for 5 concurrent clients
3. **Zero message loss** under normal conditions
4. **Graceful degradation** under network issues
5. **Clear monitoring** of system health

## Alternative Approach: Use Existing Solutions

If implementing reliable delivery proves too complex, consider:

1. **Replace WebSocket with gRPC** - Built-in reliability
2. **Use Redis Streams** - Persistent message queue
3. **Adopt ElectricSQL** - Production-ready sync
4. **Use Yjs/Automerge** - Battle-tested CRDT libraries

## Conclusion

The message delivery issue is the critical blocker for our system. Once we implement acknowledgments and retries, the concurrent sessions test should pass reliably. The other improvements will make the system production-ready.

The key insight is that distributed systems require explicit handling of failure cases - we can't assume messages will always be delivered just because the send() call succeeded.