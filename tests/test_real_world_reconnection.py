#!/usr/bin/env python3
"""
Real-world reconnection test that simulates the Task List example usage pattern.

This test creates long-running sync engines that persist through server restarts,
just like the Task List TUI application would behave in practice.
"""

import subprocess
import time
import signal
import os
import sys
from dataclasses import dataclass
from typing import Optional
import uuid

@dataclass
class TestClient:
    name: str
    database: str
    user: str
    process: Optional[subprocess.Popen] = None
    last_output: str = ""

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

def run_command(cmd: list, timeout: int = 10) -> tuple[int, str, str]:
    """Run a command and return (return_code, stdout, stderr)"""
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return -1, "", "Command timed out"
    except Exception as e:
        return -1, "", str(e)

def start_server() -> subprocess.Popen:
    """Start the sync server and return the process"""
    colored_print("🚀 Starting sync server...", "blue")
    
    # Start server and redirect output to log file
    with open("/tmp/reconnection_server.log", "w") as log:
        process = subprocess.Popen(
            ["cargo", "run", "--package", "sync-server"],
            stdout=log,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid,  # Create new process group
            env=os.environ.copy()  # Pass environment variables including DATABASE_URL
        )
    
    # Wait for server to be ready
    for i in range(30):  # 30 second timeout
        ret, _, _ = run_command(["curl", "-s", "http://localhost:8080/health"], timeout=2)
        if ret == 0:
            colored_print(f"✅ Server ready (PID: {process.pid})", "green")
            return process
        time.sleep(1)
    
    colored_print("❌ Server failed to start within 30 seconds", "red")
    process.terminate()
    raise Exception("Server startup timeout")

def stop_server(process: subprocess.Popen):
    """Stop the server process"""
    if process and process.poll() is None:
        colored_print(f"🛑 Stopping server (PID: {process.pid})", "yellow")
        try:
            # Send SIGTERM to the process group
            os.killpg(os.getpgid(process.pid), signal.SIGTERM)
            process.wait(timeout=5)
        except (subprocess.TimeoutExpired, ProcessLookupError):
            # Force kill if it doesn't respond
            try:
                os.killpg(os.getpgid(process.pid), signal.SIGKILL)
            except ProcessLookupError:
                pass
        colored_print("✅ Server stopped", "green")

def create_long_running_client(name: str, database: str, user: str, shared_doc_id: str) -> TestClient:
    """
    Create a long-running client that simulates the Task List example.
    Uses a simple loop to keep the engine alive and perform operations.
    """
    colored_print(f"📱 Creating long-running client: {name}", "cyan")
    
    # Create a simple script that keeps the engine alive
    script_content = f'''
import asyncio
import json
import time
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "target/debug"))

async def main():
    print(f"🔧 {name}: Initializing sync engine...", flush=True)
    
    # Use cargo to run operations through simple_sync_test
    import subprocess
    
    # Keep running and periodically try operations
    operation_count = 0
    server_offline_edit_done = False
    
    while True:
        await asyncio.sleep(8)  # Wait 8 seconds between operations
        operation_count += 1
        
        print(f"🔄 {name}: Performing operation #{{operation_count}}...", flush=True)
        
        # Check if we should do an offline conflict edit
        # We'll detect server offline by trying a quick status check
        status_result = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "status",
            "--database", "{database}",
            "--user", "{user}"
        ], capture_output=True, text=True, timeout=10)
        
        server_seems_offline = "Connection refused" in status_result.stderr or "timeout" in status_result.stderr.lower()
        
        if server_seems_offline and not server_offline_edit_done:
            print(f"🔌 {name}: Server appears offline - making conflicting edit to shared document", flush=True)
            
            # Make conflicting edit to the shared document while offline
            result = subprocess.run([
                "cargo", "run", "--bin", "simple_sync_test", "--",
                "update",
                "--database", "{database}",
                "--user", "{user}",
                "--id", "{shared_doc_id}",
                "--title", f"{name} Offline Edit",
                "--desc", f"Conflicting edit made by {name} while server was offline"
            ], capture_output=True, text=True, timeout=30)
            
            if result.returncode == 0:
                print(f"✅ {name}: Offline conflicting edit successful!", flush=True)
                server_offline_edit_done = True
            else:
                print(f"❌ {name}: Offline edit failed: {{result.stderr}}", flush=True)
        
        elif not server_seems_offline:
            # Server is online - try normal operations
            
            # Try to create a regular document
            result = subprocess.run([
                "cargo", "run", "--bin", "simple_sync_test", "--",
                "create", 
                "--database", "{database}",
                "--user", "{user}",
                "--title", f"{name} Task #{{operation_count}}",
                "--desc", f"Operation #{{operation_count}} from {name}"
            ], capture_output=True, text=True, timeout=30)
            
            if result.returncode == 0:
                print(f"✅ {name}: Operation #{{operation_count}} successful", flush=True)
            else:
                print(f"❌ {name}: Operation #{{operation_count}} failed: {{result.stderr}}", flush=True)
        
        # Always try to get status
        if status_result.returncode == 0:
            doc_count = status_result.stdout.count("| Rev:")
            print(f"📊 {name}: Status check successful - {{doc_count}} documents found", flush=True)
        else:
            print(f"❌ {name}: Status check failed: {{status_result.stderr}}", flush=True)

if __name__ == "__main__":
    asyncio.run(main())
'''
    
    # Write the script to a temporary file
    script_path = f"/tmp/client_{name.lower()}.py"
    with open(script_path, "w") as f:
        f.write(script_content)
    
    # Start the client process
    log_path = f"/tmp/client_{name.lower()}.log"
    with open(log_path, "w") as log:
        process = subprocess.Popen(
            ["python3", script_path],
            stdout=log,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid
        )
    
    client = TestClient(name=name, database=database, user=user, process=process)
    colored_print(f"✅ {name}: Long-running client started (PID: {process.pid})", "green")
    return client

def stop_client(client: TestClient):
    """Stop a long-running client"""
    if client.process and client.process.poll() is None:
        colored_print(f"🛑 Stopping client: {client.name}", "yellow")
        try:
            os.killpg(os.getpgid(client.process.pid), signal.SIGTERM)
            client.process.wait(timeout=5)
        except (subprocess.TimeoutExpired, ProcessLookupError):
            try:
                os.killpg(os.getpgid(client.process.pid), signal.SIGKILL)
            except ProcessLookupError:
                pass
        colored_print(f"✅ {client.name}: Client stopped", "green")

def read_client_log(client: TestClient, lines: int = 10) -> str:
    """Read the last N lines from a client's log"""
    log_path = f"/tmp/client_{client.name.lower()}.log"
    try:
        result = subprocess.run(["tail", f"-{lines}", log_path], capture_output=True, text=True)
        return result.stdout
    except:
        return "No log available"

def main():
    colored_print("🧪 Real-World Reconnection Test", "purple")
    colored_print("================================", "purple")
    colored_print("This test simulates the Task List example behavior:", "white")
    colored_print("1. Creates long-running sync engines", "white") 
    colored_print("2. Stops and restarts the server", "white")
    colored_print("3. Verifies engines reconnect and continue working", "white")
    colored_print("", "white")
    
    server_process = None
    clients = []
    
    try:
        # Clean up any previous test files
        colored_print("🧹 Cleaning up previous test files...", "yellow")
        for db in ["client1_realworld", "client2_realworld"]:
            db_path = f"databases/{db}.sqlite3"
            if os.path.exists(db_path):
                os.remove(db_path)
        
        # Setup PostgreSQL test database with unique name
        test_db_name = f"realworld_test_{uuid.uuid4().hex[:8]}"
        colored_print(f"🗄️ Setting up PostgreSQL test database: {test_db_name}...", "blue")
        
        # Drop and create PostgreSQL test database
        run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"DROP DATABASE IF EXISTS {test_db_name};"])
        ret, out, err = run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"CREATE DATABASE {test_db_name};"])
        
        if ret != 0:
            colored_print(f"❌ Failed to create test database: {err}", "red")
            return 1
        
        # Set environment variable for server to use this database  
        os.environ["DATABASE_URL"] = f"postgresql://{os.getenv('USER')}@localhost:5432/{test_db_name}"
        os.environ["MONITORING"] = "true"
        os.environ["RUST_LOG"] = "debug"
        
        # Start server
        server_process = start_server()
        
        # Phase 1: Create shared document and long-running clients
        colored_print("", "white")
        colored_print("📋 Phase 1: Creating Shared Document and Long-Running Clients", "cyan")
        colored_print("------------------------------------------------------------", "cyan")
        
        test_user = f"realworld_test_{uuid.uuid4().hex[:8]}@example.com"
        
        # Create a shared document that both clients will edit offline
        colored_print("📝 Creating shared document for conflict testing...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "create",
            "--database", "client1_realworld",
            "--user", test_user,
            "--title", "Shared Conflict Task",
            "--desc", "This document will be edited by both clients offline"
        ], timeout=30)
        
        if ret != 0:
            colored_print(f"❌ Failed to create shared document: {err}", "red")
            return 1
        
        # Extract document ID from output
        shared_doc_id = None
        for line in out.split('\n'):
            if 'Created document:' in line:
                shared_doc_id = line.split('Created document:')[1].strip()
                break
        
        if not shared_doc_id:
            colored_print("❌ Could not extract shared document ID", "red")
            return 1
        
        colored_print(f"✅ Created shared document: {shared_doc_id}", "green")
        
        # Sync the document to client2's database
        colored_print("🔄 Syncing shared document to client2...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync",
            "--database", "client2_realworld",
            "--user", test_user
        ], timeout=30)
        
        if ret != 0:
            colored_print(f"❌ Failed to sync to client2: {err}", "red")
        else:
            colored_print("✅ Shared document synced to client2", "green")
        
        # Now create the long-running clients with the shared document ID
        client1 = create_long_running_client("Client1", "client1_realworld", test_user, shared_doc_id)
        clients.append(client1)
        
        client2 = create_long_running_client("Client2", "client2_realworld", test_user, shared_doc_id)  
        clients.append(client2)
        
        # Let clients run for a bit
        colored_print("⏱️ Letting clients run for 30 seconds...", "yellow")
        time.sleep(30)
        
        # Check client status
        colored_print("📊 Client1 recent activity:", "cyan")
        print(read_client_log(client1, 5))
        colored_print("📊 Client2 recent activity:", "cyan") 
        print(read_client_log(client2, 5))
        
        # Phase 2: Stop server (simulate server crash/restart)
        colored_print("", "white")
        colored_print("📋 Phase 2: Server Restart Simulation", "cyan")
        colored_print("--------------------------------------", "cyan")
        
        stop_server(server_process)
        server_process = None
        
        colored_print("⏱️ Server offline for 20 seconds (clients should continue running)...", "yellow")
        time.sleep(20)
        
        # Check that clients are still trying to work
        colored_print("📊 Client1 during server downtime:", "cyan")
        print(read_client_log(client1, 3))
        colored_print("📊 Client2 during server downtime:", "cyan")
        print(read_client_log(client2, 3))
        
        # Phase 3: Restart server
        colored_print("", "white")
        colored_print("📋 Phase 3: Server Recovery", "cyan")
        colored_print("----------------------------", "cyan")
        
        server_process = start_server()
        
        colored_print("⏱️ Letting clients reconnect and sync for 60 seconds...", "yellow")
        time.sleep(60)
        
        # Phase 4: Verification
        colored_print("", "white") 
        colored_print("📋 Phase 4: Reconnection Verification", "cyan")
        colored_print("--------------------------------------", "cyan")
        
        colored_print("📊 Client1 after server recovery:", "cyan")
        print(read_client_log(client1, 10))
        colored_print("📊 Client2 after server recovery:", "cyan")
        print(read_client_log(client2, 10))
        
        # Check final document counts and content
        colored_print("🔍 Final verification - checking document sync...", "blue")
        
        shared_doc_content = {}
        test_passed = True
        
        for client in clients:
            ret, out, err = run_command([
                "cargo", "run", "--bin", "simple_sync_test", "--",
                "list",
                "--database", client.database,
                "--user", client.user
            ], timeout=15)
            
            if ret == 0:
                doc_count = out.count("| Rev:")
                colored_print(f"📄 {client.name}: {doc_count} documents in database", "green")
                
                # Extract the shared document's content
                for line in out.split('\n'):
                    if shared_doc_id in line:
                        # Parse the line to get title and revision
                        # Format: "Document: <id> | Title: <title> | Description: <desc> | Rev: <rev>"
                        parts = line.split('|')
                        if len(parts) >= 4:
                            title = parts[1].split(':')[1].strip() if ':' in parts[1] else "Unknown"
                            revision = parts[3].split(':')[1].strip() if ':' in parts[3] else "Unknown"
                            shared_doc_content[client.name] = {
                                "title": title,
                                "revision": revision,
                                "full_line": line
                            }
                            colored_print(f"   {client.name} shared doc: Title='{title}', Rev={revision}", "cyan")
                            break
            else:
                colored_print(f"❌ {client.name}: Failed to list documents: {err}", "red")
                test_passed = False
        
        # Verify both clients have the same content for the shared document
        if len(shared_doc_content) == 2:
            client1_content = shared_doc_content.get("Client1", {})
            client2_content = shared_doc_content.get("Client2", {})
            
            if client1_content.get("title") == client2_content.get("title") and \
               client1_content.get("revision") == client2_content.get("revision"):
                colored_print(f"✅ Both clients have synchronized content!", "green")
                colored_print(f"   Shared document title: '{client1_content.get('title')}'", "cyan")
                colored_print(f"   Shared document revision: {client1_content.get('revision')}", "cyan")
                
                # Check if it's one of the offline edits
                if "Offline Edit" in client1_content.get("title", ""):
                    colored_print("✅ Offline edit successfully propagated between clients!", "green")
                else:
                    colored_print("⚠️ WARNING: Shared document doesn't contain offline edit", "yellow")
                    test_passed = False
            else:
                colored_print("❌ SYNC FAILURE: Clients have different content for shared document!", "red")
                colored_print(f"   Client1: {client1_content}", "red")
                colored_print(f"   Client2: {client2_content}", "red")
                test_passed = False
        else:
            colored_print("❌ Could not extract shared document content from both clients", "red")
            test_passed = False
        
        # Check server logs for proper message types
        colored_print("", "white")
        colored_print("🔍 Checking server logs for message types...", "blue")
        try:
            with open("/tmp/reconnection_server.log", "r") as f:
                server_logs = f.read()
                
                # Count message types
                create_count = server_logs.count("Received CreateDocument")
                update_count = server_logs.count("Received UpdateDocument")
                conflict_count = server_logs.count("CONFLICT DETECTED")
                
                colored_print(f"📊 Server message statistics:", "cyan")
                colored_print(f"   CreateDocument messages: {create_count}", "cyan")
                colored_print(f"   UpdateDocument messages: {update_count}", "cyan")
                colored_print(f"   Conflicts detected: {conflict_count}", "cyan")
                
                # Check for improper CreateDocument usage on updates
                improper_creates = 0
                for line in server_logs.split('\n'):
                    if "Received CreateDocument" in line and "rev:" in line:
                        # Extract revision from log line
                        rev_start = line.find("rev:") + 4
                        rev_end = line.find(")", rev_start)
                        if rev_start > 3 and rev_end > rev_start:
                            revision = line[rev_start:rev_end].strip()
                            # Check if revision > 1 (indicating an update, not create)
                            if revision and revision.split('-')[0] != '1':
                                improper_creates += 1
                                colored_print(f"   ⚠️ Found CreateDocument for revision {revision} (should be UpdateDocument)", "yellow")
                
                if improper_creates > 0:
                    colored_print(f"❌ Found {improper_creates} improper CreateDocument messages for updates!", "red")
                    test_passed = False
                elif update_count > 0:
                    colored_print("✅ Server properly using UpdateDocument for updates", "green")
                
        except Exception as e:
            colored_print(f"⚠️ Could not analyze server logs: {e}", "yellow")
        
        colored_print("", "white")
        if all(c.process and c.process.poll() is None for c in clients) and test_passed:
            colored_print("✅ TEST PASSED: All clients survived server restart and offline edits synced correctly", "green")
            colored_print("🔄 Reconnection and sync logic working correctly", "green")
        else:
            if not all(c.process and c.process.poll() is None for c in clients):
                colored_print("❌ TEST FAILED: Some clients died during server restart", "red")
            if not test_passed:
                colored_print("❌ TEST FAILED: Offline edits did not sync correctly between clients", "red")
        
    except KeyboardInterrupt:
        colored_print("🛑 Test interrupted by user", "yellow")
    except Exception as e:
        colored_print(f"❌ Test failed with error: {e}", "red")
        return 1
    finally:
        # Cleanup
        colored_print("", "white")
        colored_print("🧹 Cleaning up...", "yellow")
        
        for client in clients:
            stop_client(client)
        
        if server_process:
            stop_server(server_process)
        
        # Clean up test database
        if 'test_db_name' in locals():
            colored_print(f"🧹 Cleaning up test database: {test_db_name}...", "yellow")
            run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"DROP DATABASE IF EXISTS {test_db_name};"])
    
    return 0

if __name__ == "__main__":
    sys.exit(main())