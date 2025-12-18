#pragma once

#include "replicant.h"
#include <memory>
#include <string>
#include <stdexcept>
#include <functional>

namespace replicant
{

/**
 * Exception thrown by replicant operations
 */
class SyncException : public std::runtime_error
{
public:
    explicit SyncException(const std::string& message) : std::runtime_error(message) {}
    explicit SyncException(SyncResult result) : std::runtime_error(get_error_message(result)) {}

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
 * RAII wrapper for the Replicant client with modern C++ interface
 *
 * This class provides a clean, exception-safe interface to the Replicant library.
 * It automatically handles resource management and provides type-safe operations.
 *
 * Example usage:
 * ```cpp
 * replicant::Client client("sqlite:client.db?mode=rwc", "ws://localhost:8080/ws", "user@example.com", "rpa_key", "rps_secret");
 * auto doc_id = client.create_document(R"({"title":"My Document","content":"Hello World"})");
 * client.update_document(doc_id, R"({"content":"Updated content"})");
 * ```
 */
class Client
{
private:
    std::unique_ptr<::Replicant, std::function<void(::Replicant*)>> handle_;

    void check_result(SyncResult result) const
    {
        if (result != Success)
        {
            throw SyncException(result);
        }
    }

public:
    /**
     * Create a new Replicant client instance with HMAC authentication
     *
     * @param database_url SQLite database URL (e.g., "sqlite:client.db?mode=rwc")
     * @param server_url WebSocket server URL (e.g., "ws://localhost:8080/ws")
     * @param email User email address for identification
     * @param api_key Application API key (rpa_ prefix)
     * @param api_secret Application API secret (rps_ prefix)
     * @throws SyncException if creation fails
     */
    Client(const std::string& database_url,
           const std::string& server_url,
           const std::string& email,
           const std::string& api_key,
           const std::string& api_secret)
    {
        ::Replicant* raw_handle = replicant_create(
            database_url.c_str(),
            server_url.c_str(),
            email.c_str(),
            api_key.c_str(),
            api_secret.c_str()
        );

        if (!raw_handle)
        {
            throw SyncException("Failed to create Replicant client");
        }

        handle_ = std::unique_ptr<::Replicant, std::function<void(::Replicant*)>>(
            raw_handle,
            [](::Replicant* r)
            {
                if (r)
                {
                    replicant_destroy(r);
                }
            }
        );
    }

    /**
     * Create a new document
     *
     * @param content_json Document content as JSON string (should include any title as part of the JSON)
     * @return Document ID
     * @throws SyncException if creation fails
     */
    std::string create_document(const std::string& content_json)
    {
        char doc_id[37] = {0}; // UUID string + null terminator
        SyncResult result = replicant_create_document(
            handle_.get(),
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
     * @throws SyncException if update fails
     */
    void update_document(const std::string& document_id, const std::string& content_json)
    {
        SyncResult result = replicant_update_document(
            handle_.get(),
            document_id.c_str(),
            content_json.c_str()
        );

        check_result(result);
    }

    /**
     * Delete a document
     *
     * @param document_id Document ID to delete
     * @throws SyncException if deletion fails
     */
    void delete_document(const std::string& document_id)
    {
        SyncResult result = replicant_delete_document(
            handle_.get(),
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
        char* version_str = replicant_get_version();
        if (!version_str)
        {
            return "unknown";
        }

        std::string version(version_str);
        replicant_string_free(version_str);
        return version;
    }

    /**
     * Get a document by ID
     *
     * @param document_id Document ID (UUID string)
     * @return Document as JSON string (includes id, title, content, sync_revision, etc.)
     * @throws SyncException if document not found or retrieval fails
     */
    std::string get_document(const std::string& document_id)
    {
        char* content = nullptr;
        SyncResult result = replicant_get_document(
            handle_.get(),
            document_id.c_str(),
            &content
        );

        check_result(result);

        std::string doc(content);
        replicant_string_free(content);
        return doc;
    }

    /**
     * Get all documents as a JSON array
     *
     * @return JSON array of all documents (empty array [] if no documents)
     * @throws SyncException if retrieval fails
     */
    std::string get_all_documents()
    {
        char* docs = nullptr;
        SyncResult result = replicant_get_all_documents(
            handle_.get(),
            &docs
        );

        check_result(result);

        std::string all_docs(docs);
        replicant_string_free(docs);
        return all_docs;
    }

    /**
     * Get the count of local documents
     *
     * @return Number of documents in local database
     * @throws SyncException if count fails
     */
    uint64_t count_documents()
    {
        uint64_t count = 0;
        SyncResult result = replicant_count_documents(handle_.get(), &count);
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
        return replicant_is_connected(handle_.get());
    }

    /**
     * Get the count of documents pending sync to server
     *
     * @return Number of documents waiting to be synced
     * @throws SyncException if count fails
     */
    uint64_t count_pending_sync()
    {
        uint64_t count = 0;
        SyncResult result = replicant_count_pending_sync(handle_.get(), &count);
        check_result(result);
        return count;
    }

    /**
     * Register a callback for document events (Created, Updated, Deleted)
     *
     * @param callback Function to call for document events
     * @param context User-defined context pointer passed to callback
     * @param event_filter Optional filter: 0=Created, 1=Updated, 2=Deleted, -1=all
     * @throws SyncException if registration fails
     */
    void register_document_callback(DocumentEventCallback callback, void* context, int32_t event_filter = -1)
    {
        SyncResult result = replicant_register_document_callback(handle_.get(), callback, context, event_filter);
        check_result(result);
    }

    /**
     * Register a callback for sync events (Started, Completed)
     *
     * @param callback Function to call for sync events
     * @param context User-defined context pointer passed to callback
     * @throws SyncException if registration fails
     */
    void register_sync_callback(SyncEventCallback callback, void* context)
    {
        SyncResult result = replicant_register_sync_callback(handle_.get(), callback, context);
        check_result(result);
    }

    /**
     * Register a callback for error events (SyncError)
     *
     * @param callback Function to call for error events
     * @param context User-defined context pointer passed to callback
     * @throws SyncException if registration fails
     */
    void register_error_callback(ErrorEventCallback callback, void* context)
    {
        SyncResult result = replicant_register_error_callback(handle_.get(), callback, context);
        check_result(result);
    }

    /**
     * Register a callback for connection events (Lost, Attempted, Succeeded)
     *
     * @param callback Function to call for connection events
     * @param context User-defined context pointer passed to callback
     * @throws SyncException if registration fails
     */
    void register_connection_callback(ConnectionEventCallback callback, void* context)
    {
        SyncResult result = replicant_register_connection_callback(handle_.get(), callback, context);
        check_result(result);
    }

    /**
     * Register a callback for conflict events (ConflictDetected)
     *
     * @param callback Function to call for conflict events
     * @param context User-defined context pointer passed to callback
     * @throws SyncException if registration fails
     */
    void register_conflict_callback(ConflictEventCallback callback, void* context)
    {
        SyncResult result = replicant_register_conflict_callback(handle_.get(), callback, context);
        check_result(result);
    }

    /**
     * Process all queued events on the current thread
     *
     * @return Number of events processed
     * @throws SyncException if processing fails
     */
    uint32_t process_events()
    {
        uint32_t count = 0;
        SyncResult result = replicant_process_events(handle_.get(), &count);
        check_result(result);
        return count;
    }

    // Disable copy operations (move-only type)
    Client(const Client&) = delete;
    Client& operator=(const Client&) = delete;

    // Enable move operations
    Client(Client&&) = default;
    Client& operator=(Client&&) = default;
};

} // namespace replicant
