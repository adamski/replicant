#!/usr/bin/env python3

import subprocess
import time
import signal
import sys
import os

def log(msg):
    print(f"[{time.strftime('%H:%M:%S')}] {msg}")

def start_server():
    """Start the sync server"""
    log("🚀 Starting sync server...")
    
    # Set up environment
    env = os.environ.copy()
    env["DATABASE_URL"] = f"postgresql://{os.getenv('USER')}@localhost:5432/sync_immediate_test"
    
    # Create database
    subprocess.run([
        "psql", "-U", os.getenv('USER'), "-d", "postgres", 
        "-c", "DROP DATABASE IF EXISTS sync_immediate_test;"
    ], capture_output=True)
    
    subprocess.run([
        "psql", "-U", os.getenv('USER'), "-d", "postgres", 
        "-c", "CREATE DATABASE sync_immediate_test;"
    ], check=True, capture_output=True)
    
    # Run migrations
    subprocess.run([
        "sqlx", "migrate", "run", "--source", "sync-server/migrations"
    ], env=env, check=True, capture_output=True)
    
    # Start server
    proc = subprocess.Popen([
        "cargo", "run", "--bin", "sync-server"
    ], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    
    # Wait for server to be ready
    for i in range(30):
        try:
            result = subprocess.run([
                "curl", "-s", "http://localhost:8080/health"
            ], capture_output=True, timeout=1)
            if result.returncode == 0:
                log("✅ Server ready")
                return proc
        except subprocess.TimeoutExpired:
            pass
        time.sleep(1)
    
    log("❌ Server failed to start")
    proc.kill()
    return None

def test_immediate_sync():
    """Test that documents sync immediately when server is online"""
    log("🔄 Testing immediate sync with task list example...")
    
    # Start first instance
    proc1 = subprocess.Popen([
        "cargo", "run", "--example", "task_list_example", "--", 
        "--user", "user1@example.com"
    ], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    
    time.sleep(2)  # Let first instance start
    
    # Start second instance  
    proc2 = subprocess.Popen([
        "cargo", "run", "--example", "task_list_example", "--", 
        "--user", "user1@example.com"  # Same user
    ], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    
    time.sleep(5)  # Let them both run
    
    # Clean up
    proc1.terminate()
    proc2.terminate()
    
    log("✅ Immediate sync test completed")

def main():
    log("🧪 Simple Immediate Sync Test")
    log("=" * 40)
    
    server_proc = None
    try:
        # Start server
        server_proc = start_server()
        if not server_proc:
            sys.exit(1)
        
        # Test immediate sync
        test_immediate_sync()
        
        log("✅ All tests completed!")
        
    except KeyboardInterrupt:
        log("⚠️ Test interrupted")
    finally:
        if server_proc:
            log("🧹 Cleaning up server...")
            server_proc.terminate()
            server_proc.wait()

if __name__ == "__main__":
    main()