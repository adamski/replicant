/**
 * @file callback_example.cpp
 * @brief C++ example demonstrating type-safe event callbacks
 *
 * This example shows how to use the replicant event system with the new
 * type-specific callback model that provides compile-time type safety and
 * eliminates unused fields.
 *
 * Key features:
 * - Separate callbacks for different event categories
 * - No mutexes or locks required in user code
 * - Events are automatically queued from any thread
 * - Callbacks only execute on the thread that registered them
 * - Simple main loop pattern with process_events()
 * - snake_case naming conventions for consistency
 */

#include <iostream>
#include <string>
#include <vector>
#include <chrono>
#include <thread>

// Include the replicant header
#include "replicant.h"

// Simple event statistics - no thread synchronization needed!
struct event_stats
{
    int total_events = 0;
    int document_events = 0;
    int sync_events = 0;
    int error_events = 0;
    int connection_events = 0;
    int conflict_events = 0;
    std::vector<std::string> recent_event_names;

    void add_event(const std::string& event_name)
    {
        total_events++;
        recent_event_names.push_back(event_name);

        // Keep only the last 5 events
        if (recent_event_names.size() > 5)
        {
            recent_event_names.erase(recent_event_names.begin());
        }
    }

    void print_summary() const
    {
        std::cout << "\n=== Event Summary ===\n";
        std::cout << "Total events: " << total_events << "\n";
        std::cout << "Document events: " << document_events << "\n";
        std::cout << "Sync events: " << sync_events << "\n";
        std::cout << "Error events: " << error_events << "\n";
        std::cout << "Connection events: " << connection_events << "\n";
        std::cout << "Conflict events: " << conflict_events << "\n";

        if (!recent_event_names.empty())
        {
            std::cout << "Recent events: ";
            for (const auto& name : recent_event_names)
            {
                std::cout << name << " ";
            }
            std::cout << "\n";
        }
        std::cout << "=====================\n";
    }
};

// Global stats (safe because callbacks only run on main thread)
event_stats g_stats;

// Helper function to get event type name
std::string get_event_type_name(EventType type)
{
    switch (type)
    {
        case DocumentCreated: return "DocumentCreated";
        case DocumentUpdated: return "DocumentUpdated";
        case DocumentDeleted: return "DocumentDeleted";
        case SyncStarted: return "SyncStarted";
        case SyncCompleted: return "SyncCompleted";
        case SyncError: return "SyncError";
        case ConflictDetected: return "ConflictDetected";
        case ConnectionLost: return "ConnectionLost";
        case ConnectionAttempted: return "ConnectionAttempted";
        case ConnectionSucceeded: return "ConnectionSucceeded";
        default: return "Unknown";
    }
}

// =============================================================================
// Type-specific callbacks - each receives only relevant parameters!
// =============================================================================

// Document callback - receives doc_id, title, content
void document_event_callback(
    EventType event_type,
    const char* document_id,
    const char* title,
    const char* content,
    void* context)
{
    std::string event_name = get_event_type_name(event_type);
    g_stats.add_event(event_name);
    g_stats.document_events++;

    std::cout << "📄 " << event_name;
    if (document_id)
    {
        std::cout << " - Doc ID: " << document_id;
    }
    if (title)
    {
        std::cout << " - Title: '" << title << "'";
    }
    std::cout << "\n";
}

// Sync callback - receives document count
void sync_event_callback(
    EventType event_type,
    uint64_t document_count,
    void* context)
{
    std::string event_name = get_event_type_name(event_type);
    g_stats.add_event(event_name);
    g_stats.sync_events++;

    std::cout << "🔄 " << event_name;
    if (document_count > 0)
    {
        std::cout << " - Documents: " << document_count;
    }
    std::cout << "\n";
}

// Error callback - receives error message
void error_event_callback(
    EventType event_type,
    const char* error,
    void* context)
{
    std::string event_name = get_event_type_name(event_type);
    g_stats.add_event(event_name);
    g_stats.error_events++;

    std::cout << "🚨 " << event_name;
    if (error)
    {
        std::cout << " - Error: " << error;
    }
    std::cout << "\n";
}

// Connection callback - receives connected state and attempt number
void connection_event_callback(
    EventType event_type,
    bool connected,
    uint32_t attempt_number,
    void* context)
{
    std::string event_name = get_event_type_name(event_type);
    g_stats.add_event(event_name);
    g_stats.connection_events++;

    std::cout << "🔗 " << event_name;
    std::cout << " - Connected: " << (connected ? "Yes" : "No");
    if (event_type == ConnectionAttempted)
    {
        std::cout << " - Attempt: " << attempt_number;
    }
    std::cout << "\n";
}

// Conflict callback - receives doc_id, winning content, losing content
void conflict_event_callback(
    EventType event_type,
    const char* document_id,
    const char* winning_content,
    const char* losing_content,
    void* context)
{
    std::string event_name = get_event_type_name(event_type);
    g_stats.add_event(event_name);
    g_stats.conflict_events++;

    std::cout << "⚠️ " << event_name;
    if (document_id)
    {
        std::cout << " - Doc ID: " << document_id;
    }
    std::cout << "\n";
}

// Simple RAII wrapper for sync engine
class simple_sync_engine
{
private:
    Replicant* engine_;

public:
    simple_sync_engine(const std::string& database_url,
                      const std::string& server_url,
                      const std::string& email,
                      const std::string& api_key,
                      const std::string& api_secret)
    {
        engine_ = replicant_create(database_url.c_str(), server_url.c_str(), email.c_str(), api_key.c_str(), api_secret.c_str());
        if (!engine_)
        {
            throw std::runtime_error("Failed to create sync engine");
        }
    }

    ~simple_sync_engine()
    {
        if (engine_)
        {
            replicant_destroy(engine_);
        }
    }

    // Non-copyable
    simple_sync_engine(const simple_sync_engine&) = delete;
    simple_sync_engine& operator=(const simple_sync_engine&) = delete;

    Replicant* get() const { return engine_; }

    // Register type-specific callbacks
    SyncResult register_document_callback(DocumentEventCallback callback, void* context = nullptr, int event_filter = -1)
    {
        return replicant_register_document_callback(engine_, callback, context, event_filter);
    }

    SyncResult register_sync_callback(SyncEventCallback callback, void* context = nullptr)
    {
        return replicant_register_sync_callback(engine_, callback, context);
    }

    SyncResult register_error_callback(ErrorEventCallback callback, void* context = nullptr)
    {
        return replicant_register_error_callback(engine_, callback, context);
    }

    SyncResult register_connection_callback(ConnectionEventCallback callback, void* context = nullptr)
    {
        return replicant_register_connection_callback(engine_, callback, context);
    }

    SyncResult register_conflict_callback(ConflictEventCallback callback, void* context = nullptr)
    {
        return replicant_register_conflict_callback(engine_, callback, context);
    }

    SyncResult process_events(uint32_t* processed_count = nullptr)
    {
        return replicant_process_events(engine_, processed_count);
    }

    SyncResult create_document(const std::string& content_json, std::string& out_doc_id)
    {
        char doc_id[37] = {0};
        auto result = replicant_create_document(engine_, content_json.c_str(), doc_id);
        if (result == Success)
        {
            out_doc_id = std::string(doc_id);
        }
        return result;
    }

    SyncResult update_document(const std::string& document_id, const std::string& content_json)
    {
        return replicant_update_document(engine_, document_id.c_str(), content_json.c_str());
    }

    SyncResult delete_document(const std::string& document_id)
    {
        return replicant_delete_document(engine_, document_id.c_str());
    }

    #ifdef DEBUG
    SyncResult emit_test_event(int event_type)
    {
        return replicant_emit_test_event(engine_, event_type);
    }

    SyncResult emit_test_event_burst(int count)
    {
        return replicant_emit_test_event_burst(engine_, count);
    }
    #endif
};

int main()
{
    std::cout << "=== Type-Safe Callbacks Demo ===\n";
    std::cout << "This demo shows separate callbacks for different event types!\n\n";

    try
    {
        // Create sync engine with HMAC authentication
        simple_sync_engine engine("sqlite::memory:", "ws://localhost:8080/ws", "callback-test@example.com", "rpa_test_api_key_example_12345", "rps_test_api_secret_example_67890");
        std::cout << "✓ Sync engine created\n";

        // Register type-specific callbacks
        auto result = engine.register_document_callback(document_event_callback);
        if (result != Success)
        {
            std::cout << "❌ Failed to register document callback: " << result << "\n";
            return 1;
        }
        std::cout << "✓ Document callback registered\n";

        result = engine.register_sync_callback(sync_event_callback);
        if (result != Success)
        {
            std::cout << "❌ Failed to register sync callback: " << result << "\n";
            return 1;
        }
        std::cout << "✓ Sync callback registered\n";

        result = engine.register_error_callback(error_event_callback);
        if (result != Success)
        {
            std::cout << "❌ Failed to register error callback: " << result << "\n";
            return 1;
        }
        std::cout << "✓ Error callback registered\n";

        result = engine.register_connection_callback(connection_event_callback);
        if (result != Success)
        {
            std::cout << "❌ Failed to register connection callback: " << result << "\n";
            return 1;
        }
        std::cout << "✓ Connection callback registered\n";

        result = engine.register_conflict_callback(conflict_event_callback);
        if (result != Success)
        {
            std::cout << "❌ Failed to register conflict callback: " << result << "\n";
            return 1;
        }
        std::cout << "✓ Conflict callback registered\n";

        // Test document operations
        std::cout << "\n--- Testing Document Operations ---\n";

        std::string doc_id;
        auto create_result = engine.create_document(
            R"({"title": "Type-Safe Document", "language": "C++", "type_safe": true})",
            doc_id
        );

        if (create_result == Success)
        {
            std::cout << "✓ Document created: " << doc_id << "\n";

            // Process events - this is where callbacks are invoked!
            uint32_t processed;
            engine.process_events(&processed);
            std::cout << "✓ Processed " << processed << " events\n";

            // Update the document
            auto update_result = engine.update_document(doc_id,
                R"({"title": "Type-Safe Document", "language": "C++", "type_safe": true, "updated": true})");

            if (update_result == Success)
            {
                std::cout << "✓ Document updated\n";
                engine.process_events(&processed);
                std::cout << "✓ Processed " << processed << " events\n";

                // Delete the document
                auto delete_result = engine.delete_document(doc_id);
                if (delete_result == Success)
                {
                    std::cout << "✓ Document deleted\n";
                    engine.process_events(&processed);
                    std::cout << "✓ Processed " << processed << " events\n";
                }
            }
        }
        else
        {
            std::cout << "ℹ️ Document creation failed (expected in offline mode): " << create_result << "\n";
        }

        #ifdef DEBUG
        // Test with debug events
        std::cout << "\n--- Testing Debug Events ---\n";

        engine.emit_test_event(SyncStarted);
        engine.emit_test_event(SyncCompleted);
        engine.emit_test_event(SyncError);
        engine.emit_test_event(ConnectionLost);
        engine.emit_test_event(ConnectionAttempted);
        engine.emit_test_event(ConnectionSucceeded);
        engine.emit_test_event(ConflictDetected);

        // Process all queued events
        uint32_t total_processed = 0;
        uint32_t batch_processed;
        do
        {
            engine.process_events(&batch_processed);
            total_processed += batch_processed;
        } while (batch_processed > 0);

        std::cout << "✓ Processed " << total_processed << " debug events\n";

        // Test event burst
        std::cout << "\nTesting event burst...\n";
        engine.emit_test_event_burst(5);

        // Simple main loop simulation
        std::cout << "Simulating main loop...\n";
        for (int i = 0; i < 10; ++i)
        {
            engine.process_events(&batch_processed);
            if (batch_processed > 0)
            {
                std::cout << "  Loop " << i << ": processed " << batch_processed << " events\n";
            }
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        }
        #endif

        // Print final statistics
        g_stats.print_summary();

        std::cout << "\n✅ SUCCESS: Type-safe callbacks executed without unused fields!\n";
        std::cout << "📝 Key benefits:\n";
        std::cout << "   - DocumentEventCallback only receives doc-related fields\n";
        std::cout << "   - SyncEventCallback only receives document count\n";
        std::cout << "   - ErrorEventCallback only receives error message\n";
        std::cout << "   - ConnectionEventCallback only receives connection state\n";
        std::cout << "   - ConflictEventCallback only receives conflict data\n";

    }
    catch (const std::exception& e)
    {
        std::cout << "❌ Error: " << e.what() << "\n";
        return 1;
    }

    std::cout << "\n=== Demo completed successfully! ===\n";
    return 0;
}

/*
 * Compilation instructions:
 *
 * 1. Build the Rust library first:
 *    cd sync-workspace
 *    cargo build --release
 *
 * 2. Compile this C++ example:
 *    g++ -std=c++14 \
 *        -I./sync-client/target/include \
 *        -L./target/release \
 *        -lsync_client \
 *        -framework CoreFoundation -framework Security \
 *        -ldl -lpthread -lm \
 *        examples/cpp/callback_example.cpp \
 *        -o callback_example
 *
 * 3. Run the example:
 *    ./callback_example
 *
 * Key benefits of type-specific callbacks:
 *
 * ✅ COMPILE-TIME TYPE SAFETY - each callback receives only relevant parameters
 * ✅ NO WASTED FIELDS - old EventData had 5-6 unused fields for most events
 * ✅ CLEANER API - document callbacks get doc info, sync callbacks get counts
 * ✅ EASIER MAINTENANCE - can't accidentally access wrong fields
 * ✅ BETTER DOCUMENTATION - function signature shows exactly what you receive
 */
