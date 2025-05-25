# Interactive Examples

This project includes two interactive examples that demonstrate the sync system in action.

## Prerequisites

1. PostgreSQL running (use Docker Compose or local installation)
2. Database created and accessible

## Running the Examples

### 1. Start the Monitoring Server

First, start the server with live monitoring:

```bash
# Set up the database
export DATABASE_URL="postgres://postgres:postgres@localhost/sync_db"

# Run the monitoring server
cargo run --example monitoring_server
```

The server will display:
- Real-time connection events
- All messages sent/received
- JSON patch diffs when documents are updated
- Conflict detection alerts

### 2. Run the Interactive Client

In another terminal, run the interactive client:

```bash
# Run with defaults (connects to localhost:8080)
cargo run --example interactive_client

# Or specify custom options
cargo run --example interactive_client -- \
  --database my_client.db \
  --server ws://localhost:8080/ws \
  --token my-auth-token
```

The client provides a menu-driven interface to:
- 📄 List all documents
- ➕ Create new JSON documents
- ✏️  Edit existing documents
- 🔍 View document details
- 🔄 Check sync status
- Works offline with automatic sync when reconnected

## Example Workflow

1. **Start the server** - You'll see it's ready when it displays the listening address
2. **Start a client** - It will create a new user ID automatically
3. **Create a document** - Use the menu to create a new document with JSON content
4. **Watch the server** - See the real-time logs showing the document creation
5. **Edit the document** - Make changes and see the JSON patch in the server logs
6. **Start another client** - Use the same user ID to see documents sync across clients

## Server Log Example

```
🚀 Sync Server Monitor
=====================

📊 Connecting to database: postgres://postgres:postgres@localhost/sync_db
✅ Created demo user: 123e4567-e89b-12d3-a456-426614174000

🌐 Server listening on: 0.0.0.0:8080
🔌 WebSocket endpoint: ws://0.0.0.0:8080/ws

📋 Activity Log:
────────────────────────────────────────────────────────────────────────────────
14:23:15.123 → Client connected: a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:15.456 ↓ Authenticate from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:15.457 ↑ AuthSuccess to a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:20.789 ↓ CreateDocument from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:20.790 ↑ DocumentCreated to a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:25.123 ↓ UpdateDocument from a1b2c3d4-e5f6-7890-abcd-ef1234567890
14:23:25.124 🔧 Patch applied to document 987fcdeb-51a2-43b7-8c9d-0e1f2a3b4c5d:
     [
       {
         "op": "replace",
         "path": "/content",
         "value": "Updated content"
       },
       {
         "op": "add",
         "path": "/tags",
         "value": ["example", "demo"]
       }
     ]
14:23:25.125 ↑ DocumentUpdated to a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

## Client Interface Example

```
🚀 JSON Database Sync Client
============================
👤 User ID: 123e4567-e89b-12d3-a456-426614174000
🌐 Server: ws://localhost:8080/ws

✅ Connected to sync server!

What would you like to do?
> 📄 List documents
  ➕ Create new document
  ✏️  Edit document
  🔍 View document
  🗑️  Delete document
  🔄 Sync status
  ❌ Exit

📚 Your Documents:
────────────────────────────────────────────────────────────────────────────────
✅ 987fcdeb-51a2-43b7-8c9d-0e1f2a3b4c5d My First Document (2024-01-15T14:23:20Z)
⏳ abc12345-6789-def0-1234-56789abcdef0 Work Notes (2024-01-15T14:25:00Z)
────────────────────────────────────────────────────────────────────────────────
```

## Tips

- The client works offline - documents are marked as "pending" (⏳) until synced
- You can run multiple clients with the same user ID to test real-time sync
- The server shows JSON patch diffs, making it easy to debug sync issues
- Use Ctrl+C to cleanly exit either application