#!/usr/bin/env python3
"""
Focused test to verify offline update sync behavior.
This test specifically checks if offline updates are properly synced after reconnection.
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

def run_command(cmd: list, timeout: int = 15) -> tuple[int, str, str]:
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
    
    with open("/tmp/offline_update_server.log", "w") as log:
        process = subprocess.Popen(
            ["cargo", "run", "--package", "sync-server"],
            stdout=log,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid,
            env=os.environ.copy()
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

def main():
    colored_print("🧪 Offline Update Sync Test", "purple")
    colored_print("============================", "purple")
    
    # Change to sync-workspace directory to run cargo commands
    original_dir = os.getcwd()
    # If we're in tests/, go to parent (sync-workspace)
    if os.path.basename(os.getcwd()) == "tests":
        parent_dir = os.path.dirname(os.getcwd())
        os.chdir(parent_dir)
    else:
        # If we're already in sync-workspace or its parent, find sync-workspace
        sync_workspace_path = None
        if os.path.basename(os.getcwd()) == "sync-workspace":
            sync_workspace_path = os.getcwd()
        elif os.path.exists("sync-workspace"):
            sync_workspace_path = os.path.join(os.getcwd(), "sync-workspace")
        
        if sync_workspace_path and os.path.exists(os.path.join(sync_workspace_path, "Cargo.toml")):
            os.chdir(sync_workspace_path)
        else:
            colored_print("❌ Could not find sync-workspace directory with Cargo.toml", "red")
            return 1
    
    server_process = None
    
    try:
        # Setup test database
        test_db_name = f"offline_update_test_{uuid.uuid4().hex[:8]}"
        colored_print(f"🗄️ Setting up test database: {test_db_name}...", "blue")
        
        run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"DROP DATABASE IF EXISTS {test_db_name};"])
        ret, out, err = run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"CREATE DATABASE {test_db_name};"])
        
        if ret != 0:
            colored_print(f"❌ Failed to create test database: {err}", "red")
            return 1
        
        os.environ["DATABASE_URL"] = f"postgresql://{os.getenv('USER')}@localhost:5432/{test_db_name}"
        os.environ["RUST_LOG"] = "debug"
        
        # Clean up client database
        client_db = "offline_update_test"
        client_db_path = f"databases/{client_db}.sqlite3"
        if os.path.exists(client_db_path):
            os.remove(client_db_path)
        
        test_user = f"offline_test_{uuid.uuid4().hex[:8]}@example.com"
        
        # Phase 1: Start server and create a document
        colored_print("\n📋 Phase 1: Create Initial Document", "cyan")
        server_process = start_server()
        
        colored_print("📝 Creating initial document...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "create",
            "--database", client_db,
            "--user", test_user,
            "--title", "Original Title",
            "--desc", "Original description"
        ], timeout=30)
        
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
        
        colored_print(f"✅ Created document: {doc_id}", "green")
        
        # Phase 2: Stop server and make offline update
        colored_print("\n📋 Phase 2: Offline Update", "cyan")
        stop_server(server_process)
        server_process = None
        
        colored_print("📝 Making offline update...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "update",
            "--database", client_db,
            "--user", test_user,
            "--id", doc_id,
            "--title", "Updated Title While Offline",
            "--desc", "Updated description while offline"
        ], timeout=30)
        
        if ret != 0:
            colored_print(f"❌ Failed to update document offline: {err}", "red")
            return 1
        
        colored_print("✅ Offline update successful", "green")
        
        # Check local status
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "status",
            "--database", client_db,
            "--user", test_user
        ], timeout=15)
        
        if ret == 0:
            colored_print(f"📊 Local status after offline update:", "cyan")
            print(out)
        
        # Phase 3: Restart server and check sync
        colored_print("\n📋 Phase 3: Server Restart and Sync Check", "cyan")
        server_process = start_server()
        
        colored_print("⏱️ Waiting 10 seconds for automatic reconnection and sync...", "yellow")
        time.sleep(10)
        
        # Check server logs for evidence of UpdateDocument vs CreateDocument
        colored_print("📊 Server logs (looking for UpdateDocument vs CreateDocument):", "cyan")
        try:
            with open("/tmp/offline_update_server.log", "r") as f:
                logs = f.read()
                if "UpdateDocument" in logs:
                    colored_print("✅ Found UpdateDocument in logs - patch-based sync working", "green")
                elif "CreateDocument" in logs:
                    colored_print("⚠️ Found CreateDocument in logs - may be using old logic", "yellow")
                else:
                    colored_print("❓ No document operations found in logs", "yellow")
                
                # Show relevant lines
                for line in logs.split('\n'):
                    if any(keyword in line for keyword in ["UpdateDocument", "CreateDocument", "Patch content", "reconnection"]):
                        print(f"  {line}")
        except Exception as e:
            colored_print(f"❌ Could not read server logs: {e}", "red")
        
        # Final verification - check if the update reached the server
        colored_print("\n📋 Phase 4: Verification", "cyan")
        
        # Sync again to make sure
        colored_print("🔄 Triggering explicit sync...", "blue")
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "sync",
            "--database", client_db,
            "--user", test_user
        ], timeout=30)
        
        if ret != 0:
            colored_print(f"❌ Explicit sync failed: {err}", "red")
        else:
            colored_print("✅ Explicit sync completed", "green")
        
        # Check final status
        ret, out, err = run_command([
            "cargo", "run", "--bin", "simple_sync_test", "--",
            "status",
            "--database", client_db,
            "--user", test_user
        ], timeout=15)
        
        if ret == 0:
            colored_print("📊 Final status:", "cyan")
            print(out)
            
            if "Updated Title While Offline" in out:
                colored_print("✅ Offline update successfully synced to server", "green")
            else:
                colored_print("❌ Offline update not found in synced documents", "red")
        else:
            colored_print(f"❌ Final status check failed: {err}", "red")
        
    except Exception as e:
        colored_print(f"❌ Test failed: {e}", "red")
        return 1
    finally:
        if server_process:
            stop_server(server_process)
        
        # Cleanup database
        if 'test_db_name' in locals():
            run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"DROP DATABASE IF EXISTS {test_db_name};"])
        
        # Restore original directory
        os.chdir(original_dir)
    
    return 0

if __name__ == "__main__":
    exit(main())