# Interactive Examples

This project includes interactive examples that demonstrate the sync system in action.

## Prerequisites

1. PostgreSQL running (use Docker Compose or local installation)
2. Database created and accessible

## Running the Examples

### 1. Start the Sync Server

First, start the server:

```bash
# Option 1: Using Docker (recommended)
docker-compose up -d

# Option 2: Manual setup
export DATABASE_URL="postgres://postgres:postgres@localhost/sync_db"
cargo run --bin sync-server

# Option 3: With monitoring enabled (shows real-time activity)
export DATABASE_URL="postgres://postgres:postgres@localhost/sync_db"
MONITORING=true cargo run --bin sync-server
```

With monitoring enabled, the server displays:
- Real-time connection events
- All messages sent/received
- JSON patch diffs when documents are updated
- Conflict detection alerts
- Colorized output for easy debugging

### 2. Run the Interactive Task Manager Client

In another terminal, run the interactive client:

```bash
# Run with defaults (creates databases/alice.sqlite3)
cargo run --package sync-client --example interactive_client

# Or specify a different database name
cargo run --package sync-client --example interactive_client -- --database bob

# Or specify custom options
cargo run --package sync-client --example interactive_client -- \
  --database my_tasks \
  --server ws://localhost:8080/ws \
  --token my-auth-token
```

The client provides a modern task management interface with:
- 📋 List tasks with status and priority indicators
- ➕ Create new tasks with form-based input
- ✏️  Edit tasks with guided field editing
- 🔍 View detailed task information
- ✅ Mark tasks as completed
- 🗑️  Delete tasks with confirmation
- 🔄 Check sync status
- 📱 Works offline with automatic sync when reconnected

## Example Workflow

1. **Start the server** - You'll see it's ready when it displays the listening address
2. **Start a client** - It will create a new user ID automatically
3. **Create a task** - Use the guided interface to create a task with title, description, priority, and tags
4. **Watch the server** - If monitoring is enabled, see real-time logs showing the task creation
5. **Edit the task** - Make changes through the form interface and see JSON patches in server logs
6. **Start another client** - Use the same database name to see tasks sync across clients
7. **Try different operations** - Mark tasks complete, change priorities, add tags

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
📁 Database: databases/alice.sqlite3
👤 User ID: 123e4567-e89b-12d3-a456-426614174000
🌐 Server: ws://localhost:8080/ws

✅ Connected to sync server!

What would you like to do?
> 📋 List tasks
  ➕ Create new task
  ✏️  Edit task
  🔍 View task details
  ✅ Mark task completed
  🗑️  Delete task
  🔄 Sync status
  ❌ Exit

📋 Your Tasks:
────────────────────────────────────────────────────────────────────────────────────────────────
⏳ 🔴 987fcdeb Fix critical bug - Investigate database connection issues  📤
✅ 🟡 abc12345 Complete project documentation - Write API documentation  
🔄 🟢 def67890 Code review - Review pull request #123  
────────────────────────────────────────────────────────────────────────────────────────────────
Legend: ✅=done 🔄=progress ⏳=pending | 🔴=high 🟡=med 🟢=low | 📤=sync pending
```

## Authentication

The system supports demo mode for easy testing and development:

### Demo Mode (Default)

```bash
# Uses demo-token by default - no setup required
cargo run --package sync-client --example interactive_client

# Server automatically creates users for demo-token
# Each client gets a unique user ID
```

### Custom Authentication

```bash
# Use your own auth token
cargo run --package sync-client --example interactive_client -- \
  --token my-custom-token \
  --user-id 550e8400-e29b-41d4-a716-446655440000

# Server will auto-register users with custom tokens in demo mode
```

## Tips

- The client works offline - tasks show sync status indicators (📤 = pending sync)
- You can run multiple clients with the same database name to test real-time sync
- Use monitoring mode (`MONITORING=true`) to see JSON patch diffs and debug sync issues
- The task interface provides guided input - no need to write raw JSON
- Tasks support rich metadata: priorities, tags, descriptions, and completion status
- Use Ctrl+C to cleanly exit either application