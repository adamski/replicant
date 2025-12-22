/*******************************************************************************

    replicant - Offline-first document sync for JUCE applications

*******************************************************************************/

/*******************************************************************************

 BEGIN_JUCE_MODULE_DECLARATION

  ID:               replicant
  vendor:           adamski
  version:          0.1.0
  name:             Replicant Sync Client
  description:      Offline-first document sync library for JUCE applications
  website:          https://github.com/anthropics/json-db-sync
  license:          MIT
  minimumCppStandard: 14

  dependencies:     juce_core juce_events
  OSXLibs:          replicant_client
  OSXFrameworks:    CoreFoundation Security
  linuxLibs:        replicant_client pthread dl m
  windowsLibs:      replicant_client ws2_32 userenv bcrypt

 END_JUCE_MODULE_DECLARATION

*******************************************************************************/

#pragma once
#define REPLICANT_H_INCLUDED

#include <juce_core/juce_core.h>
#include <juce_events/juce_events.h>

// Path is adjusted by build script when copying to dist/juce/replicant/
#include "../../../dist/include/replicant.hpp"

#include <functional>
#include <string>

namespace replicant
{

//==============================================================================
/**
    A sync client that provides offline-first document synchronization.

    This class wraps the replicant library and integrates with JUCE's
    event system via Timer. Events are processed on the message thread
    at 10Hz, making it safe to update UI directly from callbacks.

    Example usage:
    @code
    replicant::Replicant sync(
        "sqlite:data.db?mode=rwc", "wss://server/ws",
        "user@example.com", "api_key", "api_secret");

    sync.onDocumentCreated = [this](const std::string& id,
                                     const std::string& title,
                                     const std::string& content) {
        addToList(id, title);
        refreshUI();
    };
    @endcode
*/
class Replicant : private juce::Timer
{
public:
    //==============================================================================
    /**
        Creates a new sync client and starts processing events.

        @param databaseUrl  SQLite database URL (e.g., "sqlite:data.db?mode=rwc")
        @param serverUrl    WebSocket server URL (e.g., "wss://server.com/ws")
        @param email        User's email address
        @param apiKey       Application API key (rpa_ prefix)
        @param apiSecret    Application API secret (rps_ prefix)
    */
    Replicant(const std::string& databaseUrl,
              const std::string& serverUrl,
              const std::string& email,
              const std::string& apiKey,
              const std::string& apiSecret);

    ~Replicant() override;

    // Non-copyable, non-movable (prevent accidentally invalidating callbacks)
    Replicant(const Replicant&) = delete;
    Replicant& operator=(const Replicant&) = delete;
    Replicant(Replicant&&) = delete;
    Replicant& operator=(Replicant&&) = delete;

    //==============================================================================
    /** Creates a new document. Returns the document UUID.
        @throws replicant::SyncException on failure */
    std::string createDocument(const std::string& contentJson);

    /** Updates an existing document.
        @throws replicant::SyncException on failure */
    void updateDocument(const std::string& documentId, const std::string& contentJson);

    /** Deletes a document.
        @throws replicant::SyncException on failure */
    void deleteDocument(const std::string& documentId);

    /** Gets a document by ID. Returns the full document JSON.
        @throws replicant::SyncException if not found or on failure */
    std::string getDocument(const std::string& documentId);

    /** Gets all documents as a JSON array.
        @throws replicant::SyncException on failure */
    std::string getAllDocuments();

    //==============================================================================
    /** Returns true if connected to the sync server. */
    bool isConnected();

    /** Returns the number of documents in the local database. */
    uint64_t countDocuments();

    /** Returns the number of documents pending sync. */
    uint64_t countPendingSync();

    //==============================================================================
    /** Configure which JSON paths to index for full-text search.
        @param pathsJson JSON array of JSON paths (e.g., R"(["$.body", "$.notes"])")
        @note Replaces existing configuration and rebuilds the search index.
        @throws replicant::SyncException on failure */
    void configureSearch(const std::string& pathsJson);

    /** Search documents using FTS5 full-text search.
        @param query FTS5 query string (e.g., "music", "tun*", "\"exact phrase\"")
        @param limit Maximum number of results (default 100)
        @return JSON array of matching documents
        @throws replicant::SyncException on failure

        FTS5 Query Syntax:
        - Simple terms: "music" matches documents containing "music"
        - Prefix: "tun*" matches "tuning", "tune", etc.
        - Phrase: "\"equal temperament\"" matches exact phrase
        - Boolean: "music AND theory", "piano OR keyboard"
        - Column filter: "title:beethoven" searches only title field */
    std::string searchDocuments(const std::string& query, uint32_t limit = 100);

    /** Rebuild the full-text search index.
        @note Called automatically by configureSearch(), but can be called
              manually if needed (e.g., after bulk document imports).
        @throws replicant::SyncException on failure */
    void rebuildSearchIndex();

    //==============================================================================
    /** Called when a document is created (locally or from sync).
        Parameters: id, title (extracted from content), full content JSON */
    std::function<void(const std::string& id,
                       const std::string& title,
                       const std::string& content)> onDocumentCreated;

    /** Called when a document is updated (locally or from sync).
        Parameters: id, title (extracted from content), full content JSON */
    std::function<void(const std::string& id,
                       const std::string& title,
                       const std::string& content)> onDocumentUpdated;

    /** Called when a document is deleted (locally or from sync). */
    std::function<void(const std::string& id)> onDocumentDeleted;

    /** Called when the connection state changes. */
    std::function<void(bool connected)> onConnectionChanged;

    /** Called when a sync error occurs. */
    std::function<void(const std::string& message)> onSyncError;

private:
    void timerCallback() override;

    static void documentCallback(EventType eventType, const char* docId,
                                  const char* title, const char* content, void* context);
    static void connectionCallback(EventType eventType, bool isConnected,
                                    uint32_t attemptNumber, void* context);
    static void errorCallback(EventType eventType, const char* errorMessage, void* context);

    replicant::Client client;

    JUCE_LEAK_DETECTOR(Replicant)
};

} // namespace replicant
