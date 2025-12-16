#pragma once

#include "sync_client.h"
#include <memory>
#include <string>
#include <stdexcept>
#include <functional>

namespace SyncClient 
{

/**
 * Exception thrown by SyncClient operations
 */
class sync_exception : public std::runtime_error 
{
public:
    explicit sync_exception(const std::string& message) : std::runtime_error(message) {}
    explicit sync_exception(SyncResult result) : std::runtime_error(get_error_message(result)) {}
    
private:
    static std::string get_error_message(SyncResult result) 
    {
        switch (result) 
        {
            case Success: return "Success";
            case ErrorInvalidInput: return "Invalid input";
            case ErrorConnection: return "Connection error";
            case ErrorDatabase: return "Database error";
            case ErrorSerialization: return "Serialization error";
            case ErrorUnknown: return "Unknown error";
            default: return "Unrecognized error code";
        }
    }
};

/**
 * RAII wrapper for the sync client with modern C++ interface
 * 
 * This class provides a clean, exception-safe interface to the sync client library.
 * It automatically handles resource management and provides type-safe operations.
 * 
 * Example usage:
 * ```cpp
 * sync_engine engine("sqlite:client.db?mode=rwc", "ws://localhost:8080/ws", "user@example.com", "rpa_key", "rps_secret");
 * auto doc_id = engine.create_document(R"({"title":"My Document","content":"Hello World"})");
 * engine.update_document(doc_id, R"({"content":"Updated content"})");
 * ```
 */
class sync_engine 
{
private:
    std::unique_ptr<SyncEngine, std::function<void(SyncEngine*)>> engine_;
    
    void check_result(SyncResult result) const 
    {
        if (result != Success) 
        {
            throw sync_exception(result);
        }
    }
    
public:
    /**
     * Create a new sync engine instance with HMAC authentication
     *
     * @param database_url SQLite database URL (e.g., "sqlite:client.db?mode=rwc")
     * @param server_url WebSocket server URL (e.g., "ws://localhost:8080/ws")
     * @param email User email address for identification
     * @param api_key Application API key (rpa_ prefix)
     * @param api_secret Application API secret (rps_ prefix)
     * @throws sync_exception if creation fails
     */
    sync_engine(const std::string& database_url,
                const std::string& server_url,
                const std::string& email,
                const std::string& api_key,
                const std::string& api_secret)
    {
        SyncEngine* raw_engine = sync_engine_create(
            database_url.c_str(),
            server_url.c_str(),
            email.c_str(),
            api_key.c_str(),
            api_secret.c_str()
        );
        
        if (!raw_engine) 
        {
            throw sync_exception("Failed to create sync engine");
        }
        
        engine_ = std::unique_ptr<SyncEngine, std::function<void(SyncEngine*)>>(
            raw_engine,
            [](SyncEngine* engine) 
            {
                if (engine) 
                {
                    sync_engine_destroy(engine);
                }
            }
        );
    }
    
    /**
     * Create a new document
     *
     * @param content_json Document content as JSON string (should include any title as part of the JSON)
     * @return Document ID
     * @throws sync_exception if creation fails
     */
    std::string create_document(const std::string& content_json)
    {
        char doc_id[37] = {0}; // UUID string + null terminator
        SyncResult result = sync_engine_create_document(
            engine_.get(),
            content_json.c_str(),
            doc_id
        );

        check_result(result);
        return std::string(doc_id);
    }
    
    /**
     * Update an existing document
     * 
     * @param document_id Document ID to update
     * @param content_json New document content as JSON string
     * @throws sync_exception if update fails
     */
    void update_document(const std::string& document_id, const std::string& content_json) 
    {
        SyncResult result = sync_engine_update_document(
            engine_.get(),
            document_id.c_str(),
            content_json.c_str()
        );
        
        check_result(result);
    }
    
    /**
     * Delete a document
     * 
     * @param document_id Document ID to delete
     * @throws sync_exception if deletion fails
     */
    void delete_document(const std::string& document_id) 
    {
        SyncResult result = sync_engine_delete_document(
            engine_.get(),
            document_id.c_str()
        );
        
        check_result(result);
    }
    
    /**
     * Get the library version
     *
     * @return Version string
     */
    static std::string get_version()
    {
        char* version_str = sync_get_version();
        if (!version_str)
        {
            return "unknown";
        }

        std::string version(version_str);
        sync_string_free(version_str);
        return version;
    }

    /**
     * Get a document by ID
     *
     * @param document_id Document ID (UUID string)
     * @return Document as JSON string (includes id, title, content, sync_revision, etc.)
     * @throws sync_exception if document not found or retrieval fails
     */
    std::string get_document(const std::string& document_id)
    {
        char* content = nullptr;
        SyncResult result = sync_engine_get_document(
            engine_.get(),
            document_id.c_str(),
            &content
        );

        check_result(result);

        std::string doc(content);
        sync_string_free(content);
        return doc;
    }

    /**
     * Get all documents as a JSON array
     *
     * @return JSON array of all documents (empty array [] if no documents)
     * @throws sync_exception if retrieval fails
     */
    std::string get_all_documents()
    {
        char* docs = nullptr;
        SyncResult result = sync_engine_get_all_documents(
            engine_.get(),
            &docs
        );

        check_result(result);

        std::string all_docs(docs);
        sync_string_free(docs);
        return all_docs;
    }

    /**
     * Get the count of local documents
     *
     * @return Number of documents in local database
     * @throws sync_exception if count fails
     */
    uint64_t count_documents()
    {
        uint64_t count = 0;
        SyncResult result = sync_engine_count_documents(engine_.get(), &count);
        check_result(result);
        return count;
    }

    /**
     * Check if connected to the sync server
     *
     * @return true if connected, false otherwise
     */
    bool is_connected()
    {
        return sync_engine_is_connected(engine_.get());
    }

    /**
     * Get the count of documents pending sync to server
     *
     * @return Number of documents waiting to be synced
     * @throws sync_exception if count fails
     */
    uint64_t count_pending_sync()
    {
        uint64_t count = 0;
        SyncResult result = sync_engine_count_pending_sync(engine_.get(), &count);
        check_result(result);
        return count;
    }

    /**
     * Register a callback for document events (Created, Updated, Deleted)
     *
     * @param callback Function to call for document events
     * @param context User-defined context pointer passed to callback
     * @param event_filter Optional filter: 0=Created, 1=Updated, 2=Deleted, -1=all
     * @throws sync_exception if registration fails
     */
    void register_document_callback(DocumentEventCallback callback, void* context, int32_t event_filter = -1)
    {
        SyncResult result = sync_engine_register_document_callback(engine_.get(), callback, context, event_filter);
        check_result(result);
    }

    /**
     * Register a callback for sync events (Started, Completed)
     *
     * @param callback Function to call for sync events
     * @param context User-defined context pointer passed to callback
     * @throws sync_exception if registration fails
     */
    void register_sync_callback(SyncEventCallback callback, void* context)
    {
        SyncResult result = sync_engine_register_sync_callback(engine_.get(), callback, context);
        check_result(result);
    }

    /**
     * Register a callback for error events (SyncError)
     *
     * @param callback Function to call for error events
     * @param context User-defined context pointer passed to callback
     * @throws sync_exception if registration fails
     */
    void register_error_callback(ErrorEventCallback callback, void* context)
    {
        SyncResult result = sync_engine_register_error_callback(engine_.get(), callback, context);
        check_result(result);
    }

    /**
     * Register a callback for connection events (Lost, Attempted, Succeeded)
     *
     * @param callback Function to call for connection events
     * @param context User-defined context pointer passed to callback
     * @throws sync_exception if registration fails
     */
    void register_connection_callback(ConnectionEventCallback callback, void* context)
    {
        SyncResult result = sync_engine_register_connection_callback(engine_.get(), callback, context);
        check_result(result);
    }

    /**
     * Register a callback for conflict events (ConflictDetected)
     *
     * @param callback Function to call for conflict events
     * @param context User-defined context pointer passed to callback
     * @throws sync_exception if registration fails
     */
    void register_conflict_callback(ConflictEventCallback callback, void* context)
    {
        SyncResult result = sync_engine_register_conflict_callback(engine_.get(), callback, context);
        check_result(result);
    }

    /**
     * Process all queued events on the current thread
     *
     * @return Number of events processed
     * @throws sync_exception if processing fails
     */
    uint32_t process_events()
    {
        uint32_t count = 0;
        SyncResult result = sync_engine_process_events(engine_.get(), &count);
        check_result(result);
        return count;
    }

    // Disable copy operations (move-only type)
    sync_engine(const sync_engine&) = delete;
    sync_engine& operator=(const sync_engine&) = delete;

    // Enable move operations
    sync_engine(sync_engine&&) = default;
    sync_engine& operator=(sync_engine&&) = default;
};

} // namespace SyncClient
