#!/bin/bash

echo "🚀 JSON Database Sync Demo"
echo "=========================="
echo
echo "This script will:"
echo "1. Start PostgreSQL (using Docker)"
echo "2. Run the monitoring server"
echo "3. Help you run the interactive client"
echo

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "❌ Docker is not running. Please start Docker and try again."
    exit 1
fi

# Start PostgreSQL if not already running
echo "🐘 Starting PostgreSQL..."
docker-compose up -d postgres 2>/dev/null || {
    echo "⚠️  Could not start PostgreSQL. It may already be running."
}

# Wait for PostgreSQL to be ready
echo "⏳ Waiting for PostgreSQL to be ready..."
sleep 5

# Export database URL
export DATABASE_URL="postgres://postgres:postgres@localhost:5432/sync_db"

# Build the examples
echo "🔨 Building examples..."
cargo build --examples

echo
echo "✅ Setup complete!"
echo
echo "📋 Instructions:"
echo "───────────────"
echo
echo "1. In this terminal, run the monitoring server:"
echo "   cargo run --example monitoring_server"
echo
echo "2. In another terminal, run the interactive client:"
echo "   cargo run --example interactive_client"
echo
echo "3. Try these actions in the client:"
echo "   - Create a new document"
echo "   - Edit an existing document"
echo "   - Watch the server logs to see real-time sync activity"
echo
echo "💡 Tip: You can run multiple clients with the same user ID to test real-time sync!"
echo