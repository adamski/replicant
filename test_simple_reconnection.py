#!/usr/bin/env python3
"""
Simple reconnection test that verifies the specific issue:
Does the sync engine properly reconnect after server restart?
"""

import subprocess
import time
import signal
import os
import sys
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
    
    # Start server and redirect output to log file
    with open("/tmp/simple_reconnection_server.log", "w") as log:
        process = subprocess.Popen(
            ["cargo", "run", "--package", "sync-server"],
            stdout=log,
            stderr=subprocess.STDOUT,
            preexec_fn=os.setsid,
            env=os.environ.copy()  # Pass environment variables including DATABASE_URL
        )
    
    # Wait for server to be ready
    for i in range(30):
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
            os.killpg(os.getpgid(process.pid), signal.SIGTERM)
            process.wait(timeout=5)
        except (subprocess.TimeoutExpired, ProcessLookupError):
            try:
                os.killpg(os.getpgid(process.pid), signal.SIGKILL)
            except ProcessLookupError:
                pass
        colored_print("✅ Server stopped", "green")

def test_sync_operation(database: str, user: str, operation: str, **kwargs) -> bool:
    """Test a sync operation and return True if successful"""
    cmd = [
        "cargo", "run", "--bin", "simple_sync_test", "--",
        operation,
        "--database", database,
        "--user", user
    ]
    
    for key, value in kwargs.items():
        cmd.extend([f"--{key}", str(value)])
    
    colored_print(f"🧪 Testing {operation} operation...", "cyan")
    ret, out, err = run_command(cmd, timeout=30)
    
    if ret == 0:
        colored_print(f"✅ {operation} operation successful", "green")
        if "created document" in out.lower() or "documents:" in out.lower():
            # Extract useful info from output
            lines = out.strip().split('\n')
            for line in lines:
                if 'Created document:' in line or 'Documents:' in line:
                    colored_print(f"   📄 {line.strip()}", "white")
        return True
    else:
        colored_print(f"❌ {operation} operation failed", "red")
        if err:
            colored_print(f"   Error: {err.strip()}", "red")
        return False

def main():
    colored_print("🧪 Simple Reconnection Test", "purple")
    colored_print("============================", "purple")
    colored_print("This test verifies that sync operations work after server restart", "white")
    colored_print("", "white")
    
    server_process = None
    
    try:
        # Setup
        test_user = f"reconnect_test_{uuid.uuid4().hex[:8]}@example.com" 
        database = "reconnection_test"
        
        # Clean up previous test files
        db_path = f"databases/{database}.sqlite3"
        if os.path.exists(db_path):
            os.remove(db_path)
        
        # Setup PostgreSQL test database with unique name
        test_db_name = f"simple_reconnect_{uuid.uuid4().hex[:8]}"
        
        # Drop and create PostgreSQL test database
        colored_print(f"🗄️ Setting up PostgreSQL test database: {test_db_name}...", "blue")
        run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"DROP DATABASE IF EXISTS {test_db_name};"])
        ret, out, err = run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"CREATE DATABASE {test_db_name};"])
        
        if ret != 0:
            colored_print(f"❌ Failed to create test database: {err}", "red")
            return 1
        
        # Set environment variable for server to use this database
        os.environ["DATABASE_URL"] = f"postgresql://{os.getenv('USER')}@localhost:5432/{test_db_name}"
        os.environ["MONITORING"] = "true"
        os.environ["RUST_LOG"] = "debug"
        
        # Phase 1: Initial operations with server online
        colored_print("📋 Phase 1: Initial Operations (Server Online)", "cyan")
        colored_print("------------------------------------------------", "cyan")
        
        server_process = start_server()
        
        # Create initial document
        success1 = test_sync_operation(database, test_user, "create", 
                                     title="Initial Task", desc="Created before server restart")
        
        # Check status
        success2 = test_sync_operation(database, test_user, "status")
        
        if not (success1 and success2):
            colored_print("❌ Initial operations failed - aborting test", "red")
            return 1
            
        colored_print("✅ Phase 1 completed successfully", "green")
        
        # Phase 2: Stop server
        colored_print("", "white")
        colored_print("📋 Phase 2: Server Restart", "cyan") 
        colored_print("---------------------------", "cyan")
        
        stop_server(server_process)
        server_process = None
        
        colored_print("⏱️ Server offline for 10 seconds...", "yellow")
        time.sleep(10)
        
        # Phase 3: Restart server  
        colored_print("🚀 Restarting server...", "blue")
        server_process = start_server()
        
        # Wait a bit for any reconnection to happen
        colored_print("⏱️ Waiting 15 seconds for reconnection...", "yellow")
        time.sleep(15)
        
        # Phase 4: Test operations after restart
        colored_print("", "white")
        colored_print("📋 Phase 3: Operations After Server Restart", "cyan")
        colored_print("---------------------------------------------", "cyan")
        
        # Try to create another document
        success3 = test_sync_operation(database, test_user, "create",
                                     title="Post-Restart Task", desc="Created after server restart")
        
        # Check status again
        success4 = test_sync_operation(database, test_user, "status")
        
        # Try sync operation  
        success5 = test_sync_operation(database, test_user, "sync")
        
        # Final status check
        success6 = test_sync_operation(database, test_user, "status")
        
        # Results
        colored_print("", "white")
        colored_print("📊 Test Results", "purple")
        colored_print("===============", "purple")
        
        if success3 and success4 and success5 and success6:
            colored_print("✅ ALL TESTS PASSED: Reconnection working correctly!", "green")
            colored_print("🔄 The sync engine successfully reconnected after server restart", "green")
            return 0
        else:
            colored_print("❌ TESTS FAILED: Reconnection not working properly", "red")
            colored_print(f"   Post-restart create: {'✅' if success3 else '❌'}", "white")
            colored_print(f"   Post-restart status: {'✅' if success4 else '❌'}", "white") 
            colored_print(f"   Post-restart sync: {'✅' if success5 else '❌'}", "white")
            colored_print(f"   Final status: {'✅' if success6 else '❌'}", "white")
            
            # Show recent server logs for debugging
            colored_print("", "white")
            colored_print("📋 Recent Server Logs (for debugging):", "yellow")
            try:
                with open("/tmp/simple_reconnection_server.log", "r") as f:
                    lines = f.readlines()
                    for line in lines[-20:]:  # Last 20 lines
                        print(f"   {line.rstrip()}")
            except:
                colored_print("   Could not read server logs", "red")
            
            return 1
        
    except KeyboardInterrupt:
        colored_print("🛑 Test interrupted by user", "yellow")
        return 1
    except Exception as e:
        colored_print(f"❌ Test failed with error: {e}", "red")
        return 1
    finally:
        if server_process:
            stop_server(server_process)
        
        # Clean up test database
        if 'test_db_name' in locals():
            colored_print(f"🧹 Cleaning up test database: {test_db_name}...", "yellow")
            run_command(["psql", "-U", os.getenv("USER"), "-d", "postgres", "-c", f"DROP DATABASE IF EXISTS {test_db_name};"])

if __name__ == "__main__":
    sys.exit(main())