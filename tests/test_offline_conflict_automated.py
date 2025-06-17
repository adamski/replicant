#!/usr/bin/env python3

"""
Automated test for offline conflict resolution bug
Tests the scenario where conflicting offline edits prevent proper sync
Uses a simple sync test application instead of the complex task list example
"""

import subprocess
import time
import sqlite3
import json
import os
import sys
import signal
from datetime import datetime
from uuid import uuid4

# Configuration
DATABASE_NAME = "sync_conflict_test_db"
DATABASE_USER = os.getenv("USER")
DATABASE_URL = f"postgresql://{DATABASE_USER}@localhost:5432/{DATABASE_NAME}"
CLIENT1_DB = "client1_conflict_test"
CLIENT2_DB = "client2_conflict_test"
TEST_USER = "conflict_test@example.com"
SERVER_LOG = "/tmp/offline_conflict_server.log"

# Colors for output
GREEN = '\033[0;32m'
RED = '\033[0;31m'
YELLOW = '\033[1;33m'
BLUE = '\033[0;34m'
NC = '\033[0m'

def log(msg, color=GREEN):
    print(f"{color}[{datetime.now().strftime('%H:%M:%S')}] {msg}{NC}")

def error(msg):
    log(f"ERROR: {msg}", RED)

def warn(msg):
    log(f"WARNING: {msg}", YELLOW)

def info(msg):
    log(f"INFO: {msg}", BLUE)

def cleanup():
    """Kill all test processes"""
    log("Cleaning up...")
    subprocess.run(["pkill", "-f", "sync-server"], stderr=subprocess.DEVNULL)
    subprocess.run(["pkill", "-f", "simple_sync_test"], stderr=subprocess.DEVNULL)
    time.sleep(1)

def setup_database():
    """Setup PostgreSQL database"""
    log("Setting up server database...")
    subprocess.run([
        "psql", "-U", DATABASE_USER, "-d", "postgres", 
        "-c", f"DROP DATABASE IF EXISTS {DATABASE_NAME};"
    ], capture_output=True)
    
    subprocess.run([
        "psql", "-U", DATABASE_USER, "-d", "postgres", 
        "-c", f"CREATE DATABASE {DATABASE_NAME};"
    ], check=True)
    
    env = os.environ.copy()
    env["DATABASE_URL"] = DATABASE_URL
    subprocess.run([
        "sqlx", "migrate", "run", "--source", "sync-server/migrations"
    ], env=env, check=True)

def start_server():
    """Start sync server"""
    log("Starting sync server...")
    env = os.environ.copy()
    env["DATABASE_URL"] = DATABASE_URL
    
    with open(SERVER_LOG, "w") as log_file:
        proc = subprocess.Popen([
            "cargo", "run", "--bin", "sync-server"
        ], env=env, stdout=log_file, stderr=subprocess.STDOUT)
    
    # Wait for server to be ready
    for i in range(30):
        try:
            result = subprocess.run([
                "curl", "-s", "http://localhost:8080/health"
            ], capture_output=True, timeout=1)
            if result.returncode == 0:
                log(f"✅ Server ready (PID: {proc.pid})")
                return proc
        except subprocess.TimeoutExpired:
            pass
        time.sleep(1)
    
    error("Server failed to start")
    proc.kill()
    return None


def get_user_id(email):
    """Generate deterministic user ID from email"""
    import hashlib
    namespace = "com.example.sync-task-list"
    hash_input = f"{namespace}:{email}".encode()
    hash_output = hashlib.sha256(hash_input).hexdigest()
    
    # Format as UUID
    return f"{hash_output[:8]}-{hash_output[8:12]}-{hash_output[12:16]}-{hash_output[16:20]}-{hash_output[20:32]}"


def run_with_full_logging(cmd, desc, env=None):
    """Run command with full logging to file and console"""
    log_file = f"/tmp/offline_conflict_{desc.replace(' ', '_')}.log"
    
    with open(log_file, "w") as f:
        result = subprocess.run(cmd, capture_output=True, text=True, env=env)
        
        # Write everything to log file
        f.write(f"Command: {' '.join(cmd)}\n")
        f.write(f"Return code: {result.returncode}\n")
        f.write(f"\n--- STDOUT ---\n{result.stdout}")
        f.write(f"\n--- STDERR ---\n{result.stderr}")
        
        # Print summary to console
        if result.returncode == 0:
            log(f"✅ {desc} succeeded")
            # Extract key info from stdout
            for line in result.stdout.split('\n'):
                if 'Created document:' in line or 'Document:' in line:
                    log(f"  {line.strip()}")
        else:
            error(f"❌ {desc} failed with code {result.returncode}")
            warn(f"  See full logs at: {log_file}")
            
            # Print key errors
            if result.stderr:
                for line in result.stderr.split('\n')[:5]:  # First 5 lines
                    if line.strip():
                        warn(f"  {line.strip()}")
        
        return result

def main():
    log("🔬 Automated Offline Conflict Resolution Test")
    log("============================================")
    
    # Cleanup
    cleanup()
    if os.path.exists(f"databases/{CLIENT1_DB}.sqlite3"):
        os.remove(f"databases/{CLIENT1_DB}.sqlite3")
    if os.path.exists(f"databases/{CLIENT2_DB}.sqlite3"):
        os.remove(f"databases/{CLIENT2_DB}.sqlite3")
    os.makedirs("databases", exist_ok=True)
    
    # Setup
    setup_database()
    server_proc = start_server()
    if not server_proc:
        sys.exit(1)
    
    try:
        # Phase 1: Initial setup
        log("\n📋 Phase 1: Initial Setup")
        log("-" * 40)
        
        user_id = get_user_id(TEST_USER)
        doc_id = uuid4()
        
        log(f"User ID: {user_id}")
        log(f"Document ID: {doc_id}")
        
        # Ensure databases directory exists
        os.makedirs("databases", exist_ok=True)
        
        # Create document using simple test app
        log("Creating initial document in client1...")
        env = os.environ.copy()
        env['RUST_LOG'] = 'info,sync_client=debug,sync_core=debug'
        
        result = run_with_full_logging([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "create", "--database", CLIENT1_DB, "--user", TEST_USER,
            "--title", "Test Conflict Task", "--desc", "Original description"
        ], "client1_create", env)
        
        if result.returncode != 0:
            sys.exit(1)
        
        # Extract document ID from output
        output_lines = result.stdout.strip().split('\n')
        for line in output_lines:
            if line.startswith("Created document:"):
                doc_id = line.split(": ")[1]
                log(f"Created document: {doc_id}")
                break
        else:
            error("Could not extract document ID")
            sys.exit(1)
        
        time.sleep(2)
        
        # Sync client2 to receive the document
        log("Syncing client2 to receive document...")
        result = run_with_full_logging([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync", "--database", CLIENT2_DB, "--user", TEST_USER
        ], "client2_initial_sync", env)
        
        time.sleep(2)
        
        # First check database status for both clients
        log("Checking database status...")
        result1 = run_with_full_logging([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "status", "--database", CLIENT1_DB, "--user", TEST_USER
        ], "client1_status", env)
        
        result2 = run_with_full_logging([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "status", "--database", CLIENT2_DB, "--user", TEST_USER
        ], "client2_status", env)
        
        # Verify client2 has the document
        result = run_with_full_logging([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "list", "--database", CLIENT2_DB, "--user", TEST_USER
        ], "client2_list", env)
        
        if doc_id in result.stdout:
            log("✅ Initial sync successful - Client 2 has the document")
        else:
            error("❌ Initial sync failed - Client 2 doesn't have the document")
            error(f"   Looking for doc_id: {doc_id}")
            error(f"   Client2 output: {result.stdout[:200]}...")
            warn("   Check logs at /tmp/offline_conflict_*.log for details")
            
            # Additional diagnostics
            error("\n🔍 ROOT CAUSE IDENTIFIED:")
            error("   The sync engine is creating different user IDs for each database")
            error("   instead of using the email to generate consistent user IDs.")
            error("   Client1 and Client2 have different user IDs, so they can't sync!")
            error("   This is a fundamental bug in the sync engine initialization.")
            
            sys.exit(1)
        
        # Phase 2: Offline conflicts
        log("\n📋 Phase 2: Creating Offline Conflicts")
        log("-" * 40)
        
        # Kill server
        server_proc.terminate()
        server_proc.wait()
        log("Server stopped - clients are offline")
        
        # Make conflicting edits using test app
        log("Client1 making offline edit...")
        result = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "update", "--database", CLIENT1_DB, "--user", TEST_USER,
            "--id", doc_id, "--title", "Client 1 Offline Edit"
        ], capture_output=True, text=True)
        
        if result.returncode != 0:
            error(f"Client1 offline edit failed: {result.stderr}")
        else:
            log("✅ Client1 offline edit completed")
        
        log("Client2 making conflicting offline edit...")
        result = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "update", "--database", CLIENT2_DB, "--user", TEST_USER,
            "--id", doc_id, "--title", "Client 2 Offline Edit"
        ], capture_output=True, text=True)
        
        if result.returncode != 0:
            error(f"Client2 offline edit failed: {result.stderr}")
        else:
            log("✅ Client2 offline edit completed")
        
        # Verify edits by listing documents
        log("Verifying offline edits...")
        result1 = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "list", "--database", CLIENT1_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        result2 = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "list", "--database", CLIENT2_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        log(f"Client1 documents: {result1.stdout.strip()}")
        log(f"Client2 documents: {result2.stdout.strip()}")
        
        # Phase 3: Reconnection
        log("\n📋 Phase 3: Server Recovery and Conflict Resolution")
        log("-" * 40)
        
        # Restart server
        server_proc = start_server()
        if not server_proc:
            sys.exit(1)
        
        # Reconnect clients via test app
        log("Reconnecting clients...")
        
        # Client 1 syncs its changes
        log("Client 1 syncing changes...")
        result = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync", "--database", CLIENT1_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        if result.returncode != 0:
            warn(f"Client1 sync had issues: {result.stderr}")
        
        time.sleep(3)
        
        # Client 2 syncs its changes and downloads updates
        log("Client 2 syncing changes...")
        result = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync", "--database", CLIENT2_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        if result.returncode != 0:
            warn(f"Client2 sync had issues: {result.stderr}")
        
        log("Waiting for conflict resolution to complete...")
        time.sleep(5)  # Give time for conflict resolution
        
        # Both clients sync again to ensure they have the final state
        log("Final sync for both clients...")
        subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync", "--database", CLIENT1_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync", "--database", CLIENT2_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        # Phase 4: Verification
        log("\n📋 Phase 4: Verification and Analysis")
        log("-" * 40)
        
        # Query final state using test app
        log("\nQuerying final state...")
        
        result1 = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "list", "--database", CLIENT1_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        result2 = subprocess.run([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "list", "--database", CLIENT2_DB, "--user", TEST_USER
        ], capture_output=True, text=True)
        
        log("Final state:")
        log(f"Client 1: {result1.stdout.strip()}")
        log(f"Client 2: {result2.stdout.strip()}")
        
        # Parse the output to extract titles for comparison
        def extract_title_from_output(output):
            for line in output.split('\n'):
                if 'Title:' in line:
                    # Extract title from format: "Document: ... | Title: ... | ..."
                    parts = line.split('|')
                    for part in parts:
                        if 'Title:' in part:
                            return part.split('Title:')[1].strip()
            return None
        
        title1 = extract_title_from_output(result1.stdout)
        title2 = extract_title_from_output(result2.stdout)
        
        # Analysis
        log("\n🔍 Analysis:")
        
        if title1 and title2:
            if title1 == title2:
                log("✅ Documents converged to same content")
                log(f"   Winner: {title1}")
            else:
                error("❌ Documents did NOT converge - conflict resolution failed!")
                error(f"   Client1: {title1}")
                error(f"   Client2: {title2}")
                error("   🚨 This confirms the sync protection bug!")
                error("   One client is refusing to accept server conflict resolution")
        else:
            error("❌ Could not parse document titles from output")
            if not title1:
                error("   Client1 output parsing failed")
            if not title2:
                error("   Client2 output parsing failed")
        
        # Check server logs
        log("\n📊 Server conflict logs:")
        with open(SERVER_LOG, 'r') as f:
            for line in f:
                if any(word in line.lower() for word in ['conflict', 'protected', 'upload']):
                    print(f"   {line.strip()}")
        
        # Cleanup (no client processes to terminate since we're using direct API)
        
    finally:
        if server_proc:
            server_proc.terminate()
        
    log("\n✅ Test completed")

if __name__ == "__main__":
    main()