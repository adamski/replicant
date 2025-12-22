/*******************************************************************************

    replicant - Implementation

*******************************************************************************/

#include "replicant.h"

namespace replicant
{

//==============================================================================
Replicant::Replicant(const std::string& databaseUrl,
                     const std::string& serverUrl,
                     const std::string& email,
                     const std::string& apiKey,
                     const std::string& apiSecret)
    : client(databaseUrl, serverUrl, email, apiKey, apiSecret)
{
    client.register_document_callback(documentCallback, this);
    client.register_connection_callback(connectionCallback, this);
    client.register_error_callback(errorCallback, this);
    startTimerHz(10);
}

Replicant::~Replicant()
{
    stopTimer();

    // Clear callbacks to prevent any queued events from calling into dead lambdas
    onDocumentCreated = nullptr;
    onDocumentUpdated = nullptr;
    onDocumentDeleted = nullptr;
    onConnectionChanged = nullptr;
    onSyncError = nullptr;
}

//==============================================================================
void Replicant::timerCallback()
{
    client.process_events();
}

//==============================================================================
void Replicant::documentCallback(EventType eventType, const char* docId,
                                 const char* title, const char* content, void* context)
{
    auto* self = static_cast<Replicant*>(context);
    const std::string documentId = docId ? docId : "";
    const std::string documentTitle = title ? title : "";
    const std::string documentContent = content ? content : "";

    switch (eventType)
    {
        case DocumentCreated:
            if (self->onDocumentCreated)
                self->onDocumentCreated(documentId, documentTitle, documentContent);
            break;

        case DocumentUpdated:
            if (self->onDocumentUpdated)
                self->onDocumentUpdated(documentId, documentTitle, documentContent);
            break;

        case DocumentDeleted:
            if (self->onDocumentDeleted)
                self->onDocumentDeleted(documentId);
            break;

        default:
            break;
    }
}

void Replicant::connectionCallback(EventType eventType, bool /*isConnected*/,
                                   uint32_t /*attemptNumber*/, void* context)
{
    auto* self = static_cast<Replicant*>(context);
    if (self->onConnectionChanged)
        self->onConnectionChanged(eventType == ConnectionSucceeded);
}

void Replicant::errorCallback(EventType /*eventType*/, const char* errorMessage, void* context)
{
    auto* self = static_cast<Replicant*>(context);
    if (self->onSyncError && errorMessage)
        self->onSyncError(errorMessage);
}

//==============================================================================
std::string Replicant::createDocument(const std::string& contentJson)
{
    return client.create_document(contentJson);
}

void Replicant::updateDocument(const std::string& documentId, const std::string& contentJson)
{
    client.update_document(documentId, contentJson);
}

void Replicant::deleteDocument(const std::string& documentId)
{
    client.delete_document(documentId);
}

std::string Replicant::getDocument(const std::string& documentId)
{
    return client.get_document(documentId);
}

std::string Replicant::getAllDocuments()
{
    return client.get_all_documents();
}

//==============================================================================
bool Replicant::isConnected()
{
    return client.is_connected();
}

uint64_t Replicant::countDocuments()
{
    return client.count_documents();
}

uint64_t Replicant::countPendingSync()
{
    return client.count_pending_sync();
}

//==============================================================================
void Replicant::configureSearch(const std::string& pathsJson)
{
    client.configure_search(pathsJson);
}

std::string Replicant::searchDocuments(const std::string& query, uint32_t limit)
{
    return client.search_documents(query, limit);
}

void Replicant::rebuildSearchIndex()
{
    client.rebuild_search_index();
}

} // namespace replicant
