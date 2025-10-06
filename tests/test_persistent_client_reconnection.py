#!/usr/bin/env python3
"""
Test persistent client reconnection - simulates Task List scenario.

This test creates LONG-RUNNING sync engine instances (like Task List) 
rather than fresh instances for each command (like simple_sync_test).

The goal is to reproduce the exact failure seen in Task List where:
1. Client has persistent sync engine instance
2. Makes edit while connected  
3. Server stops unexpectedly
4. Client makes edit while disconnected (heartbeat should detect)
5. Server restarts
6. SAME sync engine instance should auto-reconnect and upload pending changes
7. Other clients should receive the update automatically

This should FAIL initially, proving we have the same issue as Task List.
"""

import asyncio
import os
import signal
import subprocess
import sys
import time
import uuid
from pathlib import Path

def colored_print(message, color):
    colors = {
        'red': '\033[91m',
        'green': '\033[92m',
        'yellow': '\033[93m',
        'blue': '\033[94m',
        'magenta': '\033[95m',
        'cyan': '\033[96m',
        'white': '\033[97m'
    }
    reset = '\033[0m'
    print(f"{colors.get(color, '')}{message}{reset}")

def run_command(command, cwd=None, env=None):
    """Run a command and return the result."""
    try:
        result = subprocess.run(
            command, 
            shell=True, 
            capture_output=True, 
            text=True, 
            cwd=cwd,
            env=env,
            timeout=30
        )
        return result.returncode == 0, result.stdout, result.stderr
    except subprocess.TimeoutExpired:
        return False, "", "Command timed out"

async def wait_for_server_ready(max_attempts=30):
    """Wait for server to be ready by checking if port 8080 is listening."""
    for attempt in range(max_attempts):
        success, _, _ = run_command("lsof -i :8080")
        if success:
            time.sleep(1)
            return True
        time.sleep(1)
    return False

async def start_server():
    """Start the sync server and return the process."""
    # Clean up any existing server
    run_command("pkill -f sync-server")
    time.sleep(2)
    
    # Start new server
    env = os.environ.copy()
    env['DATABASE_URL'] = "postgresql://$USER@localhost:5432/sync_persistent_test_db"
    
    process = subprocess.Popen(
        ["cargo", "run", "--package", "sync-server"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
        preexec_fn=os.setsid
    )
    
    if await wait_for_server_ready():
        colored_print(f"✅ Server ready (PID: {process.pid})", "green")
        return process
    else:
        colored_print("❌ Server failed to start", "red")
        process.kill()
        return None

def stop_server(process):
    """Stop the server process."""
    if process and process.poll() is None:
        try:
            os.killpg(os.getpgid(process.pid), signal.SIGTERM)
            process.wait(timeout=5)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(os.getpgid(process.pid), signal.SIGKILL)
            except ProcessLookupError:
                pass
        colored_print("🛑 Server stopped", "yellow")

class PersistentSyncClient:
    """A sync client that maintains the same Rust process/sync engine throughout the test."""
    
    def __init__(self, client_id, db_name):
        self.client_id = client_id
        self.db_name = db_name
        self.process = None
        self.commands_sent = 0
        
    async def start(self):
        """Start the persistent sync client process using daemon mode."""
        env = os.environ.copy()
        env['RUST_LOG'] = "info,sync_client=debug"
        
        # Start the CLI tool in daemon mode (keeps same sync engine alive)
        self.process = subprocess.Popen(
            [
                "cargo", "run", "--bin", "simple_sync_test", "--",
                "daemon",
                "--database", self.db_name,
                "--user", f"persistent-test-user-{self.client_id}"
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
            bufsize=1  # Line buffered
        )
        
        # Wait for daemon to be ready
        for _ in range(30):  # 30 second timeout
            line = self.process.stdout.readline()
            if "DAEMON_READY" in line:
                colored_print(f"📱 Persistent Client {self.client_id} ready (PID: {self.process.pid})", "blue")
                return True
            if self.process.poll() is not None:
                # Process died
                stderr = self.process.stderr.read()
                colored_print(f"❌ Client {self.client_id} failed to start: {stderr}", "red")
                return False
        
        colored_print(f"❌ Client {self.client_id} startup timeout", "red")
        return False
        
    def send_command(self, command):
        """Send a command to the persistent client."""
        if self.process and self.process.poll() is None:
            print(f"📤 Client {self.client_id} sending: {command}")
            self.process.stdin.write(command + "\\n")
            self.process.stdin.flush()
            self.commands_sent += 1
        else:
            print(f"❌ Client {self.client_id}: Cannot send command, process not running")
            
    def read_response(self, timeout=10):
        """Read response from the client."""
        if not self.process:
            print(f"❌ Client {self.client_id}: No process")
            return None
            
        # Simple timeout mechanism
        start_time = time.time()
        while time.time() - start_time < timeout:
            if self.process.poll() is not None:
                print(f"❌ Client {self.client_id}: Process died")
                # Show stderr for debugging
                if self.process.stderr:
                    stderr = self.process.stderr.read()
                    if stderr:
                        print(f"❌ Client {self.client_id} stderr: {stderr}")
                return None  # Process died
                
            line = self.process.stdout.readline()
            if line:
                response = line.strip()
                print(f"📋 Client {self.client_id} raw output: {response}")
                if response.startswith("RESPONSE:"):
                    # Remove the RESPONSE: prefix and return the actual response
                    actual_response = response[9:]  # Remove "RESPONSE:"
                    print(f"🔍 Client {self.client_id} response: {actual_response}")
                    return actual_response
                else:
                    # This is a log message, continue waiting for actual response
                    continue
            time.sleep(0.1)
        
        print(f"⏰ Client {self.client_id}: Response timeout after {timeout}s")
        return None
        
    def create_document(self, title, description):
        """Create a document and return the document ID."""
        self.send_command(f"CREATE:{title}:{description}")
        response = self.read_response()
        if response and response.startswith("CREATED:"):
            return response.split(":", 1)[1]
        return None
        
    def update_document(self, doc_id, title, description):
        """Update a document."""
        self.send_command(f"UPDATE:{doc_id}:{title}:{description}")
        response = self.read_response()
        return response and response.startswith("UPDATED:")
        
    def get_status(self):
        """Get client status: (doc_count, pending_count, connected)."""
        self.send_command("STATUS")
        response = self.read_response()
        if response and response.startswith("STATUS:"):
            parts = response.split(":")
            if len(parts) >= 4:
                return {
                    "doc_count": int(parts[1]),
                    "pending_count": int(parts[2]), 
                    "connected": parts[3] == "True"
                }
        return None
        
    def list_documents(self):
        """List all documents."""
        self.send_command("LIST")
        docs = []
        while True:
            response = self.read_response()
            if not response:
                break
            if response == "LIST_END":
                break
            if response.startswith("DOC:"):
                parts = response.split(":", 3)
                if len(parts) >= 4:
                    docs.append({
                        "id": parts[1],
                        "title": parts[2], 
                        "revision": parts[3]
                    })
        return docs
        
    def stop(self):
        """Stop the persistent client."""
        if self.process and self.process.poll() is None:
            self.send_command("QUIT")
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                try:
                    self.process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.process.kill()
            colored_print(f"📱 Persistent Client {self.client_id} stopped", "yellow")

async def main():
    colored_print("🧪 Persistent Client Reconnection Test", "magenta")
    colored_print("=======================================", "magenta")
    colored_print("This test simulates the EXACT Task List scenario", "white")
    colored_print("using persistent sync engine instances.", "white")
    
    # Cleanup any existing processes first
    colored_print("🧹 Cleaning up existing processes...", "cyan")
    run_command("pkill -f 'simple_sync_test.*daemon'")
    run_command("pkill -f sync-server")
    time.sleep(2)
    
    # Setup test database
    colored_print("📋 Setting up test database...", "cyan")
    success, _, _ = run_command('psql -U $USER -d postgres -c "DROP DATABASE IF EXISTS sync_persistent_test_db;"')
    success, _, _ = run_command('psql -U $USER -d postgres -c "CREATE DATABASE sync_persistent_test_db;"')
    
    if not success:
        colored_print("❌ Failed to create test database", "red")
        return False
        
    os.makedirs("databases", exist_ok=True)
    
    # Clean up old database files
    for db_file in ["persistent_client_a.db", "persistent_client_b.db"]:
        db_path = f"databases/{db_file}"
        if os.path.exists(db_path):
            os.remove(db_path)
    
    colored_print("\\n📋 Phase 1: Start Server and Connect Persistent Clients", "cyan")
    
    server_process = await start_server()
    if not server_process:
        return False
        
    try:
        # Create persistent clients (like Task List instances)
        client_a = PersistentSyncClient("A", "persistent_client_a") 
        client_b = PersistentSyncClient("B", "persistent_client_b")
        
        # Start both clients
        colored_print("🚀 Starting persistent Client A...", "blue")
        if not await client_a.start():
            colored_print("❌ Failed to start Client A", "red")
            return False
            
        colored_print("🚀 Starting persistent Client B...", "blue") 
        if not await client_b.start():
            colored_print("❌ Failed to start Client B", "red")
            return False
        
        # Let clients connect and stabilize
        colored_print("⏱️ Letting clients connect and stabilize...", "yellow")
        await asyncio.sleep(5)
        
        # Check initial status
        status_a = client_a.get_status()
        status_b = client_b.get_status()
        
        if status_a:
            colored_print(f"📊 Client A: {status_a['doc_count']} docs, {status_a['pending_count']} pending, connected: {status_a['connected']}", "cyan")
        else:
            colored_print("❌ Client A status failed", "red")
            
        if status_b:
            colored_print(f"📊 Client B: {status_b['doc_count']} docs, {status_b['pending_count']} pending, connected: {status_b['connected']}", "cyan")
        else:
            colored_print("❌ Client B status failed", "red")
        
        colored_print("\\n📋 Phase 2: Create Shared Document", "cyan")
        
        # Client A creates a document  
        colored_print("📝 Client A: Creating shared document...", "blue")
        shared_doc_id = client_a.create_document("Shared Task", "A task both clients will see")
        
        if not shared_doc_id:
            colored_print("❌ Failed to create shared document", "red")
            return False
            
        colored_print(f"✅ Created shared document: {shared_doc_id}", "green")
        
        # Wait for sync
        await asyncio.sleep(3)
        
        # Verify both clients have the document
        docs_a = client_a.list_documents()
        docs_b = client_b.list_documents()
        
        colored_print(f"📄 Client A has {len(docs_a)} documents", "cyan")
        colored_print(f"📄 Client B has {len(docs_b)} documents", "cyan")
        
        if len(docs_a) > 0 and len(docs_b) > 0:
            colored_print("✅ Both clients have documents - sync working", "green")
        else:
            colored_print("❌ Initial sync failed", "red")
            return False
            
        colored_print("\\n📋 Phase 3: Server Stop and Disconnection Detection", "cyan")
        
        # Make a connected edit first
        colored_print("📝 Client A: Making edit while connected...", "blue")
        client_a.update_document(shared_doc_id, "Connected Edit", "Edit made while server running")
        await asyncio.sleep(2)
        
        # Now stop server abruptly
        colored_print("🛑 Stopping server abruptly (simulating crash)...", "yellow")
        stop_server(server_process)
        server_process = None
        
        # Wait a moment for disconnection to be unnoticed
        await asyncio.sleep(3)
        
        # Client A makes edit while server is down (this should be queued)
        colored_print("📝 Client A: Making edit while server is DOWN...", "blue")
        success = client_a.update_document(shared_doc_id, "Persistent Client Offline Edit", "Edit made by persistent client while server down")
        
        if success:
            colored_print("✅ Client A made offline edit (should be queued)", "green")
        else:
            colored_print("⚠️ Client A offline edit returned false (but might still be queued)", "yellow")
        
        # Check status - should show pending sync
        status_a = client_a.get_status()
        colored_print(f"📊 Client A after offline edit: {status_a['doc_count']} docs, {status_a['pending_count']} pending, connected: {status_a['connected']}", "cyan")
        
        # Wait for heartbeat to detect disconnection
        colored_print("⏱️ Waiting for heartbeat to detect disconnection (up to 15 seconds)...", "yellow")
        await asyncio.sleep(15)
        
        # Check connection status after heartbeat
        status_a_after_heartbeat = client_a.get_status()
        status_b_after_heartbeat = client_b.get_status()
        colored_print(f"📊 Client A after heartbeat: connected: {status_a_after_heartbeat['connected']}", "cyan")
        colored_print(f"📊 Client B after heartbeat: connected: {status_b_after_heartbeat['connected']}", "cyan")
        
        colored_print("\\n📋 Phase 4: Server Restart and Auto-Reconnection Test", "cyan")
        
        # Restart server
        colored_print("🚀 Restarting server...", "blue")
        server_process = await start_server()
        if not server_process:
            return False
            
        # Wait for auto-reconnection and pending sync upload
        colored_print("⏱️ Waiting for auto-reconnection and pending sync upload (30 seconds)...", "yellow")
        await asyncio.sleep(30)
        
        # Check if pending sync was uploaded
        status_a_after_reconnect = client_a.get_status()
        colored_print(f"📊 Client A after reconnection: {status_a_after_reconnect['doc_count']} docs, {status_a_after_reconnect['pending_count']} pending, connected: {status_a_after_reconnect['connected']}", "cyan")
        
        colored_print("\\n📋 Phase 5: Test Automatic Propagation to Other Client", "cyan")
        
        # Check if Client B automatically received the update
        colored_print("🔍 Checking if Client B automatically received offline edit...", "blue")
        await asyncio.sleep(5)
        
        docs_a_final = client_a.list_documents()
        docs_b_final = client_b.list_documents()
        
        # Find the shared document in both clients
        shared_doc_a = next((doc for doc in docs_a_final if doc["id"] == shared_doc_id), None)
        shared_doc_b = next((doc for doc in docs_b_final if doc["id"] == shared_doc_id), None)
        
        colored_print(f"📄 Client A shared doc: {shared_doc_a['title'] if shared_doc_a else 'NOT FOUND'}", "cyan")
        colored_print(f"📄 Client B shared doc: {shared_doc_b['title'] if shared_doc_b else 'NOT FOUND'}", "cyan")
        
        colored_print("\\n📋 Phase 6: Results Analysis", "cyan")
        
        # Test results
        test_passed = True
        issues_found = []
        
        # Check if pending sync was cleared (should be 0 after successful upload)
        if status_a_after_reconnect['pending_count'] > 0:
            colored_print(f"❌ ISSUE: Client A still has {status_a_after_reconnect['pending_count']} pending syncs after reconnection!", "red")
            issues_found.append("Pending sync not uploaded during auto-reconnection")
            test_passed = False
            
        # Check if offline edit propagated to other client
        if shared_doc_a and shared_doc_b:
            if "Persistent Client Offline Edit" in shared_doc_a['title'] and "Persistent Client Offline Edit" in shared_doc_b['title']:
                colored_print("✅ Offline edit successfully propagated to both clients", "green")
            else:
                colored_print("❌ ISSUE: Offline edit did not propagate properly", "red")
                issues_found.append("Offline edit not propagated between clients")
                test_passed = False
        else:
            colored_print("❌ ISSUE: Shared document missing from one or both clients", "red")
            issues_found.append("Document sync completely broken")
            test_passed = False
            
        # Check reconnection
        if not status_a_after_reconnect['connected'] or not client_b.get_status()['connected']:
            colored_print("❌ ISSUE: Clients did not reconnect properly", "red")
            issues_found.append("Auto-reconnection failed")
            test_passed = False
            
        colored_print("\\n" + "="*50, "white")
        if test_passed:
            colored_print("🎉 TEST PASSED: Persistent client reconnection working!", "green")
        else:
            colored_print("💥 TEST FAILED: Found issues with persistent client reconnection", "red")
            colored_print("Issues found:", "white")
            for issue in issues_found:
                colored_print(f"  • {issue}", "red")
            colored_print("\\nThis matches the Task List behavior!", "yellow")
            
        return test_passed
        
    finally:
        # Cleanup
        if server_process:
            stop_server(server_process)
        
        try:
            client_a.stop()
            client_b.stop()
        except:
            pass

if __name__ == "__main__":
    success = asyncio.run(main())
    sys.exit(0 if success else 1)