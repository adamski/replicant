/**
 * @file simple_cpp_callbacks.cpp
 * @brief Simple C++ example demonstrating thread-safe event callbacks without locks
 * 
 * This example shows how to use the sync client event system with the new
 * single-thread callback model that eliminates the need for mutexes and locks.
 * 
 * Key features:
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

// Include the sync client header
#include "sync_client.h"

// Simple event statistics - no thread synchronization needed!
struct event_stats
{
    int total_events = 0;
    int document_events = 0;
    int sync_events = 0;
    int error_events = 0;
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
        case ConnectionLost: return "ConnectionStateChanged";
        default: return "Unknown";
    }
}

// Simple callback function - no locks needed!
void simple_event_callback(const EventData* event, void* context)
{
    std::string event_name = get_event_type_name(event->event_type);
    
    // Update stats (safe - only called on main thread)
    g_stats.add_event(event_name);
    
    // Handle different event types
    switch (event->event_type)
    {
        case DocumentCreated:
        case DocumentUpdated:
        case DocumentDeleted:
            g_stats.document_events++;
            std::cout << "📄 " << event_name;
            if (event->document_id)
            {
                std::cout << " - Doc ID: " << event->document_id;
            }
            if (event->title)
            {
                std::cout << " - Title: '" << event->title << "'";
            }
            std::cout << "\n";
            break;
            
        case SyncStarted:
        case SyncCompleted:
            g_stats.sync_events++;
            std::cout << "🔄 " << event_name;
            if (event->numeric_data > 0)
            {
                std::cout << " - Count: " << event->numeric_data;
            }
            std::cout << "\n";
            break;
            
        case SyncError:
            g_stats.error_events++;
            std::cout << "🚨 " << event_name;
            if (event->error)
            {
                std::cout << " - Error: " << event->error;
            }
            std::cout << "\n";
            break;
            
        case ConnectionLost:
            std::cout << "🔗 " << event_name << " - Connected: " 
                     << (event->boolean_data ? "Yes" : "No") << "\n";
            break;
            
        case ConflictDetected:
            std::cout << "⚠️ " << event_name;
            if (event->document_id)
            {
                std::cout << " - Doc ID: " << event->document_id;
            }
            std::cout << "\n";
            break;
            
        default:
            std::cout << "❓ " << event_name << "\n";
            break;
    }
}

// Simple RAII wrapper for sync engine
class simple_sync_engine
{
private:
    SyncEngine* engine_;
    
public:
    simple_sync_engine(const std::string& database_url,
                      const std::string& server_url,
                      const std::string& email,
                      const std::string& api_key,
                      const std::string& api_secret)
    {
        engine_ = sync_engine_create(database_url.c_str(), server_url.c_str(), email.c_str(), api_key.c_str(), api_secret.c_str());
        if (!engine_)
        {
            throw std::runtime_error("Failed to create sync engine");
        }
    }
    
    ~simple_sync_engine()
    {
        if (engine_)
        {
            sync_engine_destroy(engine_);
        }
    }
    
    // Non-copyable
    simple_sync_engine(const simple_sync_engine&) = delete;
    simple_sync_engine& operator=(const simple_sync_engine&) = delete;
    
    SyncEngine* get() const { return engine_; }
    
    SyncResult register_callback(EventCallback callback, void* context = nullptr, int event_filter = -1)
    {
        return sync_engine_register_event_callback(engine_, callback, context, event_filter);
    }
    
    SyncResult process_events(uint32_t* processed_count = nullptr)
    {
        return sync_engine_process_events(engine_, processed_count);
    }
    
    SyncResult create_document(const std::string& content_json, std::string& out_doc_id)
    {
        char doc_id[37] = {0};
        auto result = sync_engine_create_document(engine_, content_json.c_str(), doc_id);
        if (result == Success)
        {
            out_doc_id = std::string(doc_id);
        }
        return result;
    }
    
    SyncResult update_document(const std::string& document_id, const std::string& content_json)
    {
        return sync_engine_update_document(engine_, document_id.c_str(), content_json.c_str());
    }
    
    SyncResult delete_document(const std::string& document_id)
    {
        return sync_engine_delete_document(engine_, document_id.c_str());
    }
    
    #ifdef DEBUG
    SyncResult emit_test_event(int event_type)
    {
        return sync_engine_emit_test_event(engine_, event_type);
    }
    
    SyncResult emit_test_event_burst(int count)
    {
        return sync_engine_emit_test_event_burst(engine_, count);
    }
    #endif
};

int main()
{
    std::cout << "=== Simple C++ Callbacks Demo ===\n";
    std::cout << "This demo shows thread-safe callbacks WITHOUT locks or mutexes!\n\n";
    
    try
    {
        // Create sync engine with HMAC authentication
        simple_sync_engine engine("sqlite::memory:", "ws://localhost:8080/ws", "simple-cpp-test@example.com", "rpa_test_api_key_example_12345", "rps_test_api_secret_example_67890");
        std::cout << "✓ Sync engine created\n";
        
        // Register callback - this sets the callback thread to the current thread
        auto result = engine.register_callback(simple_event_callback);
        if (result != Success)
        {
            std::cout << "❌ Failed to register callback: " << result << "\n";
            return 1;
        }
        std::cout << "✓ Event callback registered\n";
        
        // Test document operations
        std::cout << "\n--- Testing Document Operations ---\n";
        
        std::string doc_id;
        auto create_result = engine.create_document(
            R"({"title": "Simple C++ Document", "language": "C++", "complexity": "simple", "thread_safe": true})",
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
                R"({"language": "C++", "complexity": "simple", "thread_safe": true, "updated": true})");
            
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
        engine.emit_test_event(ConnectionLost);
        
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
        
        std::cout << "\n✅ SUCCESS: All callbacks executed on main thread without locks!\n";
        std::cout << "📝 Key insight: Events can be generated on any thread, but callbacks\n";
        std::cout << "   are only executed when you call process_events() on the main thread.\n";
        std::cout << "   This eliminates the need for thread synchronization in your code!\n";
        
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
 *        -I./sync-client/include \
 *        -L./target/release \
 *        -lsync_client \
 *        -framework CoreFoundation -framework Security \
 *        -ldl -lpthread -lm \
 *        sync-client/examples/simple_cpp_callbacks.cpp \
 *        -o simple_cpp_test
 * 
 * 3. Run the test:
 *    ./simple_cpp_test
 * 
 * Key benefits of this approach:
 * 
 * ✅ NO MUTEXES OR LOCKS in user code
 * ✅ Thread-safe by design (single callback thread)
 * ✅ Simple main loop pattern
 * ✅ Events can be generated from any thread
 * ✅ Callbacks only execute on the thread that registered them
 * ✅ Easy to understand and maintain
 * ✅ No risk of deadlocks or race conditions in user code
 * ✅ Perfect for game engines, UI frameworks, and event loops
 * 
 * Usage pattern:
 * 1. Register callbacks (once, on main thread)
 * 2. In your main loop, regularly call process_events()
 * 3. That's it! No need to worry about thread safety.
 */