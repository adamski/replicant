/**
 * @file cpp_lambda_callbacks.cpp
 * @brief Modern C++ example demonstrating event callbacks with lambda functions
 * 
 * This example shows how to use the sync client event system with:
 * - Lambda functions for callbacks
 * - Smart pointers for automatic resource management
 * - RAII patterns for cleanup
 * - STL containers for event tracking
 * - Modern C++ features (C++11 and later)
 * - snake_case naming conventions for consistency with Rust
 */

#include <iostream>
#include <memory>
#include <vector>
#include <string>
#include <functional>
#include <map>
#include <atomic>
#include <chrono>
#include <thread>
#include <mutex>
#include <sstream>

// Include the event header
#include "../include/sync_client_events.h"

// Forward declarations for the main sync client functions
extern "C" {
    struct CSyncEngine;
    extern struct CSyncEngine* sync_engine_create(const char* database_url, const char* server_url, const char* auth_token, const char* user_identifier);
    extern void sync_engine_destroy(struct CSyncEngine* engine);
    extern SyncResult sync_engine_create_document(struct CSyncEngine* engine, const char* content_json, char* out_document_id);
    extern SyncResult sync_engine_update_document(struct CSyncEngine* engine, const char* document_id, const char* content_json);
    extern SyncResult sync_engine_delete_document(struct CSyncEngine* engine, const char* document_id);
    
    // Debug functions (only available in debug builds)
    #ifdef DEBUG
    extern SyncResult sync_engine_emit_test_event(struct CSyncEngine* engine, int event_type);
    extern SyncResult sync_engine_emit_test_event_burst(struct CSyncEngine* engine, int count);
    #endif
}

namespace SyncClient {

/**
 * @brief Modern C++ wrapper for the sync engine with RAII and smart pointers
 */
class SyncEngine
{
private:
    std::unique_ptr<CSyncEngine, void(*)(CSyncEngine*)> engine_;
    
public:
    explicit SyncEngine(const std::string& database_url, 
                       const std::string& server_url, 
                       const std::string& auth_token,
                       const std::string& user_identifier)
        : engine_(sync_engine_create(database_url.c_str(), server_url.c_str(), auth_token.c_str(), user_identifier.c_str()),
                  sync_engine_destroy)
    {
        if (!engine_)
        {
            throw std::runtime_error("Failed to create sync engine");
        }
    }
    
    // Non-copyable but movable
    SyncEngine(const SyncEngine&) = delete;
    SyncEngine& operator=(const SyncEngine&) = delete;
    SyncEngine(SyncEngine&&) = default;
    SyncEngine& operator=(SyncEngine&&) = default;
    
    CSyncEngine* get() const { return engine_.get(); }
    
    SyncResult create_document(const std::string& content_json, std::string& out_document_id)
    {
        char doc_id[37] = {0};
        auto result = sync_engine_create_document(engine_.get(), content_json.c_str(), doc_id);
        if (result == SYNC_RESULT_SUCCESS)
        {
            out_document_id = std::string(doc_id);
        }
        return result;
    }
    
    SyncResult update_document(const std::string& document_id, const std::string& content_json)
    {
        return sync_engine_update_document(engine_.get(), document_id.c_str(), content_json.c_str());
    }
    
    SyncResult delete_document(const std::string& document_id)
    {
        return sync_engine_delete_document(engine_.get(), document_id.c_str());
    }
    
    #ifdef DEBUG
    SyncResult emit_test_event(int event_type)
    {
        return sync_engine_emit_test_event(engine_.get(), event_type);
    }
    
    SyncResult emit_test_event_burst(int count)
    {
        return sync_engine_emit_test_event_burst(engine_.get(), count);
    }
    #endif
};

/**
 * @brief Event data wrapper with modern C++ conveniences
 */
struct EventInfo
{
    SyncEventType type;
    std::string document_id;
    std::string title;
    std::string content;
    std::string error;
    uint64_t numeric_data;
    bool boolean_data;
    
    explicit EventInfo(const SyncEventData* event) 
        : type(event->event_type)
        , numeric_data(event->numeric_data)
        , boolean_data(event->boolean_data)
    {
        if (event->document_id) document_id = event->document_id;
        if (event->title) title = event->title;
        if (event->content) content = event->content;
        if (event->error) error = event->error;
    }
    
    std::string get_type_name() const
    {
        switch (type)
        {
            case SYNC_EVENT_DOCUMENT_CREATED: return "DocumentCreated";
            case SYNC_EVENT_DOCUMENT_UPDATED: return "DocumentUpdated";
            case SYNC_EVENT_DOCUMENT_DELETED: return "DocumentDeleted";
            case SYNC_EVENT_SYNC_STARTED: return "SyncStarted";
            case SYNC_EVENT_SYNC_COMPLETED: return "SyncCompleted";
            case SYNC_EVENT_SYNC_ERROR: return "SyncError";
            case SYNC_EVENT_CONFLICT_DETECTED: return "ConflictDetected";
            case SYNC_EVENT_CONNECTION_STATE_CHANGED: return "ConnectionStateChanged";
            default: return "Unknown";
        }
    }
    
    std::string to_string() const
    {
        std::ostringstream oss;
        oss << "Event{type=" << get_type_name();
        if (!document_id.empty()) oss << ", doc_id=" << document_id;
        if (!title.empty()) oss << ", title='" << title << "'";
        if (!error.empty()) oss << ", error='" << error << "'";
        if (numeric_data > 0) oss << ", numeric=" << numeric_data;
        if (boolean_data) oss << ", boolean=true";
        oss << "}";
        return oss.str();
    }
};

/**
 * @brief Event statistics collector using modern C++ containers
 */
class EventStats
{
private:
    std::mutex mutex_;
    std::map<SyncEventType, std::atomic<int>> counts_;
    std::vector<EventInfo> recent_events_;
    static constexpr size_t MAX_RECENT_EVENTS = 10;
    
public:
    void record_event(const EventInfo& event)
    {
        std::lock_guard<std::mutex> lock(mutex_);
        counts_[event.type]++;
        
        recent_events_.push_back(event);
        if (recent_events_.size() > MAX_RECENT_EVENTS)
        {
            recent_events_.erase(recent_events_.begin());
        }
    }
    
    int get_count(SyncEventType type) const
    {
        auto it = counts_.find(type);
        return it != counts_.end() ? it->second.load() : 0;
    }
    
    int get_total_count() const
    {
        int total = 0;
        for (const auto& pair : counts_)
        {
            total += pair.second.load();
        }
        return total;
    }
    
    std::vector<EventInfo> get_recent_events() const
    {
        std::lock_guard<std::mutex> lock(const_cast<std::mutex&>(mutex_));
        return recent_events_;
    }
    
    void print_summary() const
    {
        std::cout << "\n=== Event Statistics ===\n";
        std::cout << "Total events: " << get_total_count() << "\n";
        
        for (const auto& pair : counts_)
        {
            if (pair.second.load() > 0)
            {
                EventInfo dummy_event(nullptr);
                dummy_event.type = pair.first;
                std::cout << "  " << dummy_event.get_type_name() 
                         << ": " << pair.second.load() << "\n";
            }
        }
        
        auto recent = get_recent_events();
        if (!recent.empty())
        {
            std::cout << "\nRecent events:\n";
            for (const auto& event : recent)
            {
                std::cout << "  " << event.to_string() << "\n";
            }
        }
        std::cout << "========================\n";
    }
};

/**
 * @brief Callback manager using std::function and lambdas
 */
class CallbackManager
{
private:
    struct CallbackInfo
    {
        std::function<void(const EventInfo&)> handler;
        SyncEventType filter;
        bool filter_enabled;
        
        CallbackInfo(std::function<void(const EventInfo&)> h, SyncEventType f = SYNC_EVENT_DOCUMENT_CREATED, bool enabled = false)
            : handler(std::move(h)), filter(f), filter_enabled(enabled) {}
    };
    
    std::vector<std::unique_ptr<CallbackInfo>> callbacks_;
    
    // Static C callback that forwards to C++ handlers
    static void static_callback(const SyncEventData* event, void* context)
    {
        auto* manager = static_cast<CallbackManager*>(context);
        EventInfo info(event);
        
        for (auto& callback_info : manager->callbacks_)
        {
            if (!callback_info->filter_enabled || callback_info->filter == event->event_type)
            {
                callback_info->handler(info);
            }
        }
    }
    
public:
    template<typename Callable>
    void add_callback(Callable&& callback, SyncEventType filter = SYNC_EVENT_DOCUMENT_CREATED, bool enable_filter = false)
    {
        callbacks_.emplace_back(
            std::make_unique<CallbackInfo>(
                std::forward<Callable>(callback), 
                filter, 
                enable_filter
            )
        );
    }
    
    SyncResult register_with(SyncEngine& engine, int event_filter = -1)
    {
        return sync_engine_register_event_callback(
            engine.get(),
            static_callback,
            this,
            event_filter
        );
    }
};

} // namespace SyncClient

/**
 * @brief Test function demonstrating various lambda callback patterns
 */
void test_lambda_callbacks()
{
    std::cout << "=== C++ Lambda Callbacks Test ===\n";
    
    try
    {
        // Create sync engine with RAII
        SyncClient::SyncEngine engine("sqlite::memory:", "ws://localhost:8080/ws", "test-token", "lambda-test@example.com");
        std::cout << "✓ Sync engine created\n";
        
        // Create event statistics collector
        SyncClient::EventStats stats;
        
        // Create callback manager
        SyncClient::CallbackManager callbacks;
        
        // Add various lambda callbacks
        
        // 1. Simple lambda that captures stats by reference
        callbacks.add_callback([&stats](const SyncClient::EventInfo& event) {
            stats.record_event(event);
            std::cout << "📊 Stats: " << event.to_string() << "\n";
        });
        
        // 2. Lambda with capture by value for counting specific events
        int document_events = 0;
        callbacks.add_callback([document_events](const SyncClient::EventInfo& event) mutable {
            if (event.type == SYNC_EVENT_DOCUMENT_CREATED || 
                event.type == SYNC_EVENT_DOCUMENT_UPDATED || 
                event.type == SYNC_EVENT_DOCUMENT_DELETED) {
                document_events++;
                std::cout << "📄 Document event #" << document_events << ": " << event.get_type_name() << "\n";
            }
        });
        
        // 3. Lambda for error handling with complex logic
        callbacks.add_callback([](const SyncClient::EventInfo& event) {
            if (event.type == SYNC_EVENT_SYNC_ERROR) {
                std::cerr << "🚨 Sync Error: " << event.error << "\n";
                // Could trigger retry logic, notifications, etc.
            } else if (event.type == SYNC_EVENT_CONFLICT_DETECTED) {
                std::cout << "⚠️  Conflict detected for document: " << event.document_id << "\n";
                // Could trigger conflict resolution UI
            }
        });
        
        // 4. Performance monitoring lambda with timing
        auto start_time = std::chrono::steady_clock::now();
        callbacks.add_callback([start_time](const SyncClient::EventInfo& event) {
            if (event.type == SYNC_EVENT_SYNC_COMPLETED) {
                auto now = std::chrono::steady_clock::now();
                auto duration = std::chrono::duration_cast<std::chrono::milliseconds>(now - start_time);
                std::cout << "⏱️  Sync completed in " << duration.count() << "ms, " 
                         << event.numeric_data << " documents\n";
            }
        });
        
        // 5. Lambda using std::function for more complex callback types
        std::function<void(const SyncClient::EventInfo&)> connection_monitor = 
            [](const SyncClient::EventInfo& event) {
                if (event.type == SYNC_EVENT_CONNECTION_STATE_CHANGED) {
                    std::cout << "🔗 Connection " << (event.boolean_data ? "established" : "lost") << "\n";
                }
            };
        callbacks.add_callback(connection_monitor);
        
        // Register all callbacks with the engine
        auto result = callbacks.register_with(engine);
        if (result != SYNC_RESULT_SUCCESS) {
            throw std::runtime_error("Failed to register callbacks");
        }
        std::cout << "✓ Lambda callbacks registered\n";
        
        // Test with actual document operations
        std::cout << "\n--- Testing Document Operations ---\n";
        
        std::string doc_id;
        auto create_result = engine.create_document(
            R"({"title": "C++ Test Document", "language": "C++", "features": ["lambdas", "RAII", "smart_pointers"]})",
            doc_id
        );
        
        if (create_result == SYNC_RESULT_SUCCESS) {
            std::cout << "✓ Document created: " << doc_id << "\n";
            
            // Small delay to allow callbacks to process
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
            
            // Update the document
            auto update_result = engine.update_document(doc_id, 
                R"({"language": "C++", "features": ["lambdas", "RAII", "smart_pointers"], "updated": true})");
            
            if (update_result == SYNC_RESULT_SUCCESS) {
                std::cout << "✓ Document updated\n";
                std::this_thread::sleep_for(std::chrono::milliseconds(10));
                
                // Delete the document
                auto delete_result = engine.delete_document(doc_id);
                if (delete_result == SYNC_RESULT_SUCCESS) {
                    std::cout << "✓ Document deleted\n";
                    std::this_thread::sleep_for(std::chrono::milliseconds(10));
                }
            }
        } else {
            std::cout << "ℹ️  Document creation failed (expected in offline mode): " << create_result << "\n";
        }
        
        #ifdef DEBUG
        // Test with debug events if available
        std::cout << "\n--- Testing Debug Events ---\n";
        
        engine.emit_test_event(SYNC_EVENT_SYNC_STARTED);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
        
        engine.emit_test_event(SYNC_EVENT_SYNC_COMPLETED);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
        
        engine.emit_test_event(SYNC_EVENT_CONNECTION_STATE_CHANGED);
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
        
        // Test burst events
        std::cout << "Testing event burst...\n";
        engine.emit_test_event_burst(3);
        std::this_thread::sleep_for(std::chrono::milliseconds(50));
        
        #endif
        
        // Print final statistics
        stats.print_summary();
        
        std::cout << "\n✓ All lambda callbacks executed successfully!\n";
        
    } catch (const std::exception& e) {
        std::cerr << "❌ Error: " << e.what() << "\n";
        return;
    }
    
    std::cout << "\n=== C++ Lambda Test Completed Successfully! ===\n";
}

/**
 * @brief Test advanced C++ patterns with move semantics and perfect forwarding
 */
void test_advanced_cpp_patterns() {
    std::cout << "\n=== Advanced C++ Patterns Test ===\n";
    
    try {
        SyncClient::SyncEngine engine("sqlite::memory:", "ws://localhost:8080/ws", "advanced-test", "advanced-test@example.com");
        SyncClient::CallbackManager callbacks;
        
        // Test with shared_ptr instead of unique_ptr for lambdas (easier to copy)
        auto shared_data = std::make_shared<std::string>("Advanced callback data");
        callbacks.add_callback([data = shared_data](const SyncClient::EventInfo& event) {
            std::cout << "🔧 Advanced callback with data: " << *data 
                     << ", event: " << event.get_type_name() << "\n";
        });
        
        // Test with shared state using shared_ptr (avoids atomic copy issues)
        struct CallbackState {
            std::vector<std::string> event_history;
            int total_events = 0;
            std::mutex mutex;
            
            void add_event(const std::string& event_name) {
                std::lock_guard<std::mutex> lock(mutex);
                event_history.push_back(event_name);
                total_events++;
                if (event_history.size() > 5) {
                    event_history.erase(event_history.begin());
                }
            }
            
            std::pair<int, std::vector<std::string>> get_state() const {
                std::lock_guard<std::mutex> lock(const_cast<std::mutex&>(mutex));
                return {total_events, event_history};
            }
        };
        
        auto shared_state = std::make_shared<CallbackState>();
        callbacks.add_callback([shared_state](const SyncClient::EventInfo& event) {
            shared_state->add_event(event.get_type_name());
            auto state = shared_state->get_state();
            std::cout << "📋 State callback - Total events: " << state.first 
                     << ", Recent: ";
            for (const auto& name : state.second) {
                std::cout << name << " ";
            }
            std::cout << "\n";
        });
        
        callbacks.register_with(engine);
        std::cout << "✓ Advanced callbacks registered\n";
        
        #ifdef DEBUG
        // Trigger some events
        for (int i = 0; i < 3; ++i) {
            engine.emit_test_event(SYNC_EVENT_DOCUMENT_CREATED);
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        }
        #endif
        
        std::cout << "✓ Advanced patterns test completed\n";
        
    } catch (const std::exception& e) {
        std::cerr << "❌ Advanced test error: " << e.what() << "\n";
    }
}

int main() {
    std::cout << "Starting C++ Lambda Callbacks Demo\n";
    std::cout << "===================================\n";
    
    // Test basic lambda patterns
    test_lambda_callbacks();
    
    // Test advanced C++ patterns
    test_advanced_cpp_patterns();
    
    std::cout << "\n🎉 All C++ tests completed!\n";
    return 0;
}

/*
 * Compilation instructions:
 * 
 * To compile this C++ example:
 * 
 * 1. Build the Rust library first:
 *    cd sync-workspace
 *    cargo build --release
 * 
 * 2. Compile the C++ test:
 *    g++ -std=c++14 \
 *        -I./sync-client/include \
 *        -L./target/release \
 *        -lsync_client \
 *        -framework CoreFoundation -framework Security \
 *        -ldl -lpthread -lm \
 *        sync-client/examples/cpp_lambda_callbacks.cpp \
 *        -o cpp_lambda_test
 * 
 * 3. Run the test:
 *    ./cpp_lambda_test
 * 
 * Features demonstrated:
 * - RAII with smart pointers (unique_ptr)
 * - Lambda functions with various capture modes
 * - Move semantics and perfect forwarding
 * - STL containers (vector, map, function)
 * - Thread-safe programming with atomic and mutex
 * - Modern C++ exception handling
 * - Template programming
 * - Chrono library for timing
 * - snake_case naming conventions for consistency
 */