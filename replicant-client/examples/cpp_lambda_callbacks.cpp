/**
 * @file cpp_lambda_callbacks.cpp
 * @brief Modern C++ example demonstrating event callbacks with lambda functions
 *
 * This example shows how to use the replicant event system with:
 * - Lambda functions for callbacks
 * - The replicant::Client C++ wrapper
 * - RAII patterns for cleanup
 * - Modern C++ features (C++14)
 * - snake_case naming conventions for consistency with Rust
 */

#include <iostream>
#include <vector>
#include <string>
#include <chrono>
#include <thread>
#include <mutex>
#include <atomic>

#include "../include/replicant.hpp"

/**
 * @brief Event statistics collector
 */
struct EventStats
{
    std::atomic<int> documents_created{0};
    std::atomic<int> documents_updated{0};
    std::atomic<int> documents_deleted{0};
    std::atomic<int> sync_events{0};
    std::atomic<int> errors{0};
    std::atomic<int> connection_events{0};

    void print_summary() const
    {
        std::cout << "\n=== Event Statistics ===\n";
        std::cout << "Documents created: " << documents_created << "\n";
        std::cout << "Documents updated: " << documents_updated << "\n";
        std::cout << "Documents deleted: " << documents_deleted << "\n";
        std::cout << "Sync events: " << sync_events << "\n";
        std::cout << "Connection events: " << connection_events << "\n";
        std::cout << "Errors: " << errors << "\n";
        std::cout << "========================\n";
    }
};

/**
 * @brief Document event callback using C-style function
 */
void on_document_event(EventType event_type, const char* doc_id,
                       const char* title, const char* content, void* context)
{
    auto* stats = static_cast<EventStats*>(context);
    const std::string id = doc_id ? doc_id : "";
    const std::string doc_title = title ? title : "";

    switch (event_type)
    {
        case DocumentCreated:
            stats->documents_created++;
            std::cout << "Document created: " << id << " - " << doc_title << "\n";
            break;

        case DocumentUpdated:
            stats->documents_updated++;
            std::cout << "Document updated: " << id << " - " << doc_title << "\n";
            break;

        case DocumentDeleted:
            stats->documents_deleted++;
            std::cout << "Document deleted: " << id << "\n";
            break;

        default:
            break;
    }
}

/**
 * @brief Connection event callback
 */
void on_connection_event(EventType event_type, bool is_connected,
                         uint32_t attempt_number, void* context)
{
    auto* stats = static_cast<EventStats*>(context);
    stats->connection_events++;

    switch (event_type)
    {
        case ConnectionLost:
            std::cout << "Connection lost\n";
            break;

        case ConnectionAttempted:
            std::cout << "Connection attempt #" << attempt_number << "\n";
            break;

        case ConnectionSucceeded:
            std::cout << "Connected to server\n";
            break;

        default:
            break;
    }
}

/**
 * @brief Error event callback
 */
void on_error_event(EventType event_type, const char* error_message, void* context)
{
    auto* stats = static_cast<EventStats*>(context);
    stats->errors++;
    std::cerr << "Sync error: " << (error_message ? error_message : "unknown") << "\n";
}

/**
 * @brief Sync event callback
 */
void on_sync_event(EventType event_type, uint64_t documents_synced, void* context)
{
    auto* stats = static_cast<EventStats*>(context);
    stats->sync_events++;

    if (event_type == SyncStarted)
    {
        std::cout << "Sync started\n";
    }
    else if (event_type == SyncCompleted)
    {
        std::cout << "Sync completed: " << documents_synced << " documents\n";
    }
}

/**
 * @brief Test function demonstrating callback registration and document operations
 */
void test_replicant_callbacks()
{
    std::cout << "=== Replicant C++ Callbacks Demo ===\n";
    std::cout << "Version: " << replicant::Client::get_version() << "\n\n";

    try
    {
        // Create client with RAII
        replicant::Client client(
            "sqlite::memory:",
            "ws://localhost:8080/ws",
            "demo@example.com",
            "rpa_demo_key",
            "rps_demo_secret"
        );
        std::cout << "Client created\n";

        // Create statistics collector
        EventStats stats;

        // Register callbacks
        client.register_document_callback(on_document_event, &stats);
        client.register_connection_callback(on_connection_event, &stats);
        client.register_error_callback(on_error_event, &stats);
        client.register_sync_callback(on_sync_event, &stats);
        std::cout << "Callbacks registered\n";

        // Test document operations
        std::cout << "\n--- Testing Document Operations ---\n";

        auto doc_id = client.create_document(
            R"({"title": "Test Document", "content": "Hello from C++"})"
        );
        std::cout << "Created document: " << doc_id << "\n";

        // Process events to trigger callbacks
        client.process_events();

        // Update the document
        client.update_document(doc_id,
            R"({"title": "Updated Document", "content": "Modified content"})"
        );
        std::cout << "Updated document\n";

        client.process_events();

        // Get document count
        std::cout << "Document count: " << client.count_documents() << "\n";
        std::cout << "Pending sync: " << client.count_pending_sync() << "\n";
        std::cout << "Connected: " << (client.is_connected() ? "yes" : "no") << "\n";

        // Delete the document
        client.delete_document(doc_id);
        std::cout << "Deleted document\n";

        client.process_events();

        // Print statistics
        stats.print_summary();

        std::cout << "\nDemo completed successfully!\n";
    }
    catch (const replicant::SyncException& e)
    {
        std::cerr << "Replicant error: " << e.what() << "\n";
    }
    catch (const std::exception& e)
    {
        std::cerr << "Error: " << e.what() << "\n";
    }
}

int main()
{
    test_replicant_callbacks();
    return 0;
}

/*
 * Compilation instructions:
 *
 * 1. Build the Rust library first:
 *    cd sync-workspace
 *    cargo build --release
 *
 * 2. Compile the C++ example:
 *    g++ -std=c++14 \
 *        -I./replicant-client/include \
 *        -I./target/include \
 *        -L./target/release \
 *        -lreplicant_client \
 *        -framework CoreFoundation -framework Security \
 *        -ldl -lpthread -lm \
 *        replicant-client/examples/cpp_lambda_callbacks.cpp \
 *        -o cpp_callback_demo
 *
 * 3. Run the example:
 *    ./cpp_callback_demo
 */
