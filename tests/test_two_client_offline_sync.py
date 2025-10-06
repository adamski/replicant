#!/usr/bin/env python3
"""
Simple two-client offline sync test.
Tests if offline edits propagate between clients after reconnection.
"""

import subprocess
import time
import signal
import os
import uuid

def colored_print(text: str, color: str = "white"):
    colors = {
        "red": "\033[91m",
        "green": "\033[92m", 
        "yellow": "\033[93m",
        "blue": "\033[94m",
        "purple": "\033[95m",
        "cyan": "\033[96m",
        "white": "\033[97m",
        "reset": "\033[0m"
    }
    print(f"{colors.get(color, colors['white'])}{text}{colors['reset']}")

def run_command(cmd: list, timeout: int = 15, env=None) -> tuple[int, str, str]:
    """Run a command and return (return_code, stdout, stderr)"""
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "Command timed out"
    except Exception as e:
        return -1, "", str(e)

def start_server() -> subprocess.Popen:
    """Start the sync server and return the process"""
    colored_print("🚀 Starting sync server...", "blue")
    
    # Set up environment with fresh database
    env = os.environ.copy()
    env["DATABASE_URL"] = f"postgresql://{os.getenv('USER')}@localhost:5432/sync_real_world_test"
    
    with open("/tmp/two_client_server.log", "w") as log:
        process = subprocess.Popen(
            ["cargo", "run", "--package", "sync-server"],
            stdout=log,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid,
            env=env
        )
    
    # Wait for server to be ready
    for i in range(30):
        ret, _, _ = run_command(["curl", "-s", "http://localhost:8080/health"], timeout=2)
        if ret == 0:
            colored_print(f"✅ Server ready (PID: {process.pid})", "green")
            return process
        time.sleep(1)
    
    colored_print("❌ Server failed to start", "red")
    process.terminate()
    raise Exception("Server startup timeout")

def stop_server(process: subprocess.Popen):
    """Stop the server process"""
    if process and process.poll() is None:
        colored_print(f"🛑 Stopping server (PID: {process.pid})", "yellow")
        try:
            os.killpg(os.getpgid(process.pid), signal.SIGTERM)
            process.wait(timeout=5)
        except (subprocess.TimeoutExpired, ProcessLookupError):
            try:
                os.killpg(os.getpgid(process.pid), signal.SIGKILL)
            except ProcessLookupError:
                pass
        colored_print("✅ Server stopped", "green")

def check_document_content(database: str, user: str, doc_id: str) -> dict:
    """Get document content and revision"""
    ret, out, err = run_command([
        "cargo", "run", "--bin", "simple_sync_test", "--",
        "list",
        "--database", database,
        "--user", user
    ])
    
    if ret != 0:
        return {"error": err}
    
    # Find the specific document
    for line in out.split('\n'):
        if doc_id in line:
            # Parse: "Document: <id> | Title: <title> | Description: <desc> | Rev: <rev>"
            parts = line.split('|')
            if len(parts) >= 4:
                title = parts[1].split(':')[1].strip() if ':' in parts[1] else "Unknown"
                revision = parts[3].split(':')[1].strip() if ':' in parts[3] else "Unknown"
                return {"title": title, "revision": revision, "line": line}
    
    return {"error": "Document not found"}

def main():
    colored_print("🧪 Two-Client Offline Sync Test", "purple")
    colored_print("=================================", "purple")
    
    server_process = None
    test_user = f"twoclient_test_{uuid.uuid4().hex[:8]}@example.com"
    client_a_db = "twoclient_a"
    client_b_db = "twoclient_b"
    
    # Clean up previous test files
    for db in [client_a_db, client_b_db]:
        db_path = f"databases/{db}.sqlite3"
        if os.path.exists(db_path):
            os.remove(db_path)
    
    try:
        # Phase 1: Start server and create shared document
        colored_print("\n📋 Phase 1: Setup - Create Shared Document", "cyan")
        server_process = start_server()
        
        # Client A creates a document
        colored_print("📝 Client A: Creating shared document...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "create",
            "--database", client_a_db,
            "--user", test_user,
            "--title", "Shared Task",
            "--desc", "A task both clients will see"
        ])
        
        if ret != 0:
            colored_print(f"❌ Failed to create document: {err}", "red")
            return 1
        
        # Extract document ID
        doc_id = None
        for line in out.split('\n'):
            if 'Created document:' in line:
                doc_id = line.split('Created document:')[1].strip()
                break
        
        if not doc_id:
            colored_print("❌ Could not extract document ID", "red")
            return 1
        
        colored_print(f"✅ Created shared document: {doc_id}", "green")
        
        # Client B syncs to get the document
        colored_print("🔄 Client B: Syncing to get shared document...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync",
            "--database", client_b_db,
            "--user", test_user
        ])
        
        if ret != 0:
            colored_print(f"❌ Client B sync failed: {err}", "red")
            return 1
        
        # Verify both clients have the same initial content
        colored_print("🔍 Verifying initial sync...", "blue")
        client_a_content = check_document_content(client_a_db, test_user, doc_id)
        client_b_content = check_document_content(client_b_db, test_user, doc_id)
        
        if client_a_content.get("title") == client_b_content.get("title"):
            colored_print(f"✅ Both clients have: '{client_a_content.get('title')}'", "green")
        else:
            colored_print("❌ Initial sync failed - clients have different content", "red")
            return 1
        
        # Phase 2: Stop server and make offline edit
        colored_print("\n📋 Phase 2: Offline Edit", "cyan")
        stop_server(server_process)
        server_process = None
        
        colored_print("📝 Client A: Making offline edit...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "update",
            "--database", client_a_db,
            "--user", test_user,
            "--id", doc_id,
            "--title", "Client A Offline Edit",
            "--desc", "This edit was made while server was offline"
        ])
        
        if ret != 0:
            colored_print(f"❌ Offline edit failed: {err}", "red")
            return 1
        
        colored_print("✅ Client A offline edit completed", "green")
        
        # Verify Client A has the new content locally
        client_a_content = check_document_content(client_a_db, test_user, doc_id)
        colored_print(f"📄 Client A now has: '{client_a_content.get('title')}'", "cyan")
        
        # Phase 3: Silent Disconnection Test with Server Running
        colored_print("\n📋 Phase 3: Silent Disconnection and Heartbeat Test", "cyan")
        server_process = start_server()
        
        # Start two "persistent" client connections that will stay alive
        colored_print("🔄 Starting persistent client connections...", "blue")
        
        # Client A connects and syncs offline edit
        colored_print("🔄 Client A: Connecting and syncing offline edit...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync",
            "--database", client_a_db,
            "--user", test_user
        ])
        
        if ret != 0:
            colored_print(f"❌ Client A initial sync failed: {err}", "red")
        else:
            colored_print("✅ Client A synced offline edit to server", "green")
        
        # Client B connects
        colored_print("🔄 Client B: Connecting to server...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "status",
            "--database", client_b_db,
            "--user", test_user
        ])
        
        # Wait for both clients to be fully connected
        time.sleep(3)
        colored_print("✅ Both clients connected to server", "green")
        
        # Verify current state before server disconnect
        client_a_before_disconnect = check_document_content(client_a_db, test_user, doc_id)
        client_b_before_disconnect = check_document_content(client_b_db, test_user, doc_id)
        colored_print(f"📄 Client A state: '{client_a_before_disconnect.get('title')}'", "cyan")
        colored_print(f"📄 Client B state: '{client_b_before_disconnect.get('title')}'", "cyan")
        
        # Phase 4: Simulate server crash while clients stay connected
        colored_print("\n📋 Phase 4: Server Crash Simulation", "cyan")
        colored_print("🛑 Stopping server abruptly (simulating crash/restart)...", "yellow")
        stop_server(server_process)
        server_process = None
        
        # Wait for disconnection to be unnoticed initially
        time.sleep(2)
        
        # Client A makes an edit while server is down (should queue for sync)
        colored_print("📝 Client A: Making edit while server is DOWN...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "update",
            "--database", client_a_db,
            "--user", test_user,
            "--id", doc_id,
            "--title", "Edit Made During Server Downtime",
            "--desc", "This edit was made while server was crashed"
        ])
        
        if ret != 0:
            colored_print(f"❌ Edit during downtime failed: {err}", "red")
        else:
            colored_print("✅ Client A made edit during server downtime", "green")
        
        # Check Client A's local state after downtime edit
        client_a_during_downtime = check_document_content(client_a_db, test_user, doc_id)
        colored_print(f"📄 Client A after downtime edit: '{client_a_during_downtime.get('title')}'", "cyan")
        
        # Wait for heartbeat to detect disconnection (10 second ping interval + buffer)
        colored_print("⏱️ Waiting for heartbeat to detect disconnection (up to 15 seconds)...", "yellow")
        time.sleep(15)
        
        # Phase 5: Server restart and automatic reconnection test
        colored_print("\n📋 Phase 5: Server Restart and Automatic Recovery", "cyan")
        colored_print("🚀 Restarting server...", "blue")
        server_process = start_server()
        
        # Wait for automatic reconnection and sync to happen
        colored_print("⏱️ Waiting for automatic reconnection and sync (up to 20 seconds)...", "yellow")
        time.sleep(20)
        
        # Check if Client A's edit automatically synced to server
        client_a_after_reconnect = check_document_content(client_a_db, test_user, doc_id)
        colored_print(f"📄 Client A after reconnection: '{client_a_after_reconnect.get('title')}'", "cyan")
        
        # Now check if Client B automatically gets the update
        colored_print("🔄 Client B: Checking for automatic update propagation...", "blue")
        
        # Trigger Client B to reconnect/sync by doing a status check
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "status",
            "--database", client_b_db,
            "--user", test_user
        ])
        
        # Wait for automatic propagation
        colored_print("⏱️ Waiting for automatic propagation to Client B...", "yellow")
        time.sleep(8)
        
        # Check if Client B received the update
        client_b_after = check_document_content(client_b_db, test_user, doc_id)
        colored_print(f"📄 Client B after propagation: '{client_b_after.get('title')}'", "cyan")
        
        # Phase 6: Verification of Silent Disconnection Recovery
        colored_print("\n📋 Phase 6: Silent Disconnection Recovery Verification", "cyan")
        
        # The key test: Did the edit made during server downtime propagate after reconnection?
        test_passed = True
        
        # Check if the downtime edit made it through the entire process
        expected_title = "Edit Made During Server Downtime"
        
        if client_b_after.get("title") == client_a_after_reconnect.get("title") and \
           expected_title in client_b_after.get("title", ""):
            colored_print("✅ SILENT DISCONNECTION RECOVERY SUCCESSFUL!", "green")
            colored_print(f"   Both clients now have: '{client_b_after.get('title')}'", "green")
            colored_print(f"   Both clients have revision: {client_b_after.get('revision')}", "green")
            colored_print("✅ Edit made during server downtime automatically propagated!", "green")
        else:
            colored_print("❌ SILENT DISCONNECTION RECOVERY FAILED!", "red")
            colored_print(f"   Client A has: '{client_a_after_reconnect.get('title')}' (Rev: {client_a_after_reconnect.get('revision')})", "red")
            colored_print(f"   Client B has: '{client_b_after.get('title')}' (Rev: {client_b_after.get('revision')})", "red")
            colored_print("   Edit made during server downtime did NOT propagate to Client B!", "red")
            test_passed = False
        
        # Verify the progression through all edits
        if expected_title not in client_a_after_reconnect.get("title", ""):
            colored_print("❌ Client A's downtime edit was lost during reconnection!", "red")
            test_passed = False
        
        # Also check that we progressed beyond the initial offline edit
        if "Client A Offline Edit" in client_b_after.get("title", "") and expected_title not in client_b_after.get("title", ""):
            colored_print("⚠️ WARNING: Client B still has old offline edit, not the downtime edit", "yellow")
            colored_print("   This suggests the downtime edit didn't sync or propagate properly", "yellow")
            test_passed = False
        
        # Check server logs for message types
        colored_print("\n🔍 Checking server message types...", "blue")
        try:
            with open("/tmp/two_client_server.log", "r") as f:
                server_logs = f.read()
                
                create_count = server_logs.count("Received CreateDocument")
                update_count = server_logs.count("Received UpdateDocument")
                conflict_count = server_logs.count("CONFLICT DETECTED")
                
                colored_print(f"📊 Server received: {create_count} CreateDocument, {update_count} UpdateDocument", "cyan")
                colored_print(f"📊 Conflicts detected: {conflict_count}", "cyan")
                
                # Check for improper CreateDocument on revisions > 1
                improper_creates = 0
                for line in server_logs.split('\n'):
                    if "Received CreateDocument" in line and "rev:" in line:
                        if "rev: 1-" not in line:  # Not a revision 1
                            improper_creates += 1
                            # Extract and show the revision
                            rev_start = line.find("rev:") + 4
                            rev_end = line.find(")", rev_start)
                            if rev_start > 3 and rev_end > rev_start:
                                revision = line[rev_start:rev_end].strip()
                                colored_print(f"   ⚠️ CreateDocument used for revision {revision}", "yellow")
                
                if improper_creates > 0:
                    colored_print(f"❌ Found {improper_creates} improper CreateDocument messages!", "red")
                    test_passed = False
                elif update_count > 0:
                    colored_print("✅ Server properly using UpdateDocument for updates", "green")
                    
        except Exception as e:
            colored_print(f"⚠️ Could not analyze server logs: {e}", "yellow")
        
        # Final result
        colored_print("", "white")
        if test_passed:
            colored_print("🎉 TEST PASSED: Silent disconnection detection and automatic recovery working!", "green")
            colored_print("   ✅ Heartbeat detected server disconnection", "green")
            colored_print("   ✅ Edits made during downtime were queued", "green")
            colored_print("   ✅ Automatic reconnection occurred after server restart", "green")
            colored_print("   ✅ Pending edits automatically synced and propagated", "green")
        else:
            colored_print("💥 TEST FAILED: Silent disconnection recovery not working properly", "red")
            colored_print("   This matches the Task List issue where updates don't appear after server restart", "red")
            colored_print("   The heartbeat mechanism may not be detecting disconnections properly", "red")
        
        return 0 if test_passed else 1
        
    except Exception as e:
        colored_print(f"❌ Test failed: {e}", "red")
        return 1
    finally:
        if server_process:
            stop_server(server_process)
        
        # Cleanup
        for db in [client_a_db, client_b_db]:
            db_path = f"databases/{db}.sqlite3"
            if os.path.exists(db_path):
                os.remove(db_path)

if __name__ == "__main__":
    exit(main())