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
  linuxLibs:        replicant_client
  windowsLibs:      replicant_client

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
    event system via Timer. Events are processed on the message thread,
    making it safe to update UI directly from callbacks.

    Example usage:
    @code
    replicant::SyncClient sync(
        "sqlite:data.db?mode=rwc", "wss://server/ws",
        "user@example.com", "api_key", "api_secret");

    sync.onDocumentCreated = [this](const std::string& id, const std::string& content) {
        auto task = nlohmann::json::parse(content).get<Task>();
        refreshUI();
    };
    @endcode
*/
class SyncClient : private juce::Timer
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
    SyncClient(const std::string& databaseUrl,
               const std::string& serverUrl,
               const std::string& email,
               const std::string& apiKey,
               const std::string& apiSecret);

    ~SyncClient() override;

    SyncClient(const SyncClient&) = delete;
    SyncClient& operator=(const SyncClient&) = delete;

    //==============================================================================
    /** Creates a new document. Returns the document UUID. */
    std::string createDocument(const std::string& contentJson);

    /** Updates an existing document. */
    void updateDocument(const std::string& documentId, const std::string& contentJson);

    /** Deletes a document. */
    void deleteDocument(const std::string& documentId);

    /** Gets a document by ID. Returns empty string if not found. */
    std::string getDocument(const std::string& documentId);

    /** Gets all documents as a JSON array. */
    std::string getAllDocuments();

    //==============================================================================
    /** Returns true if connected to the sync server. */
    bool isConnected() const;

    /** Returns the number of documents in the local database. */
    uint64_t countDocuments() const;

    /** Returns the number of documents pending sync. */
    uint64_t countPendingSync() const;

    //==============================================================================
    /** Called when a document is created (locally or from sync). */
    std::function<void(const std::string& id, const std::string& content)> onDocumentCreated;

    /** Called when a document is updated (locally or from sync). */
    std::function<void(const std::string& id, const std::string& content)> onDocumentUpdated;

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

    Client client_;

    JUCE_LEAK_DETECTOR(SyncClient)
};

} // namespace replicant
