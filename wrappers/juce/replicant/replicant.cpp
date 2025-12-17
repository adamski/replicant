/*******************************************************************************

    replicant - Implementation

*******************************************************************************/

#include "replicant.h"

namespace replicant
{

//==============================================================================
SyncClient::SyncClient(const std::string& databaseUrl,
                       const std::string& serverUrl,
                       const std::string& email,
                       const std::string& apiKey,
                       const std::string& apiSecret)
    : client_(databaseUrl, serverUrl, email, apiKey, apiSecret)
{
    client_.register_document_callback(documentCallback, this);
    client_.register_connection_callback(connectionCallback, this);
    client_.register_error_callback(errorCallback, this);
    startTimerHz(60);
}

SyncClient::~SyncClient()
{
    stopTimer();
}

//==============================================================================
void SyncClient::timerCallback()
{
    client_.process_events();
}

//==============================================================================
void SyncClient::documentCallback(EventType eventType, const char* docId,
                                   const char* /*title*/, const char* content, void* context)
{
    auto* self = static_cast<SyncClient*>(context);
    const std::string documentId = docId ? docId : "";
    const std::string documentContent = content ? content : "";

    switch (eventType)
    {
        case DocumentCreated:
            if (self->onDocumentCreated)
                self->onDocumentCreated(documentId, documentContent);
            break;

        case DocumentUpdated:
            if (self->onDocumentUpdated)
                self->onDocumentUpdated(documentId, documentContent);
            break;

        case DocumentDeleted:
            if (self->onDocumentDeleted)
                self->onDocumentDeleted(documentId);
            break;

        default:
            break;
    }
}

void SyncClient::connectionCallback(EventType eventType, bool /*isConnected*/,
                                     uint32_t /*attemptNumber*/, void* context)
{
    auto* self = static_cast<SyncClient*>(context);
    if (self->onConnectionChanged)
        self->onConnectionChanged(eventType == ConnectionSucceeded);
}

void SyncClient::errorCallback(EventType /*eventType*/, const char* errorMessage, void* context)
{
    auto* self = static_cast<SyncClient*>(context);
    if (self->onSyncError && errorMessage)
        self->onSyncError(errorMessage);
}

//==============================================================================
std::string SyncClient::createDocument(const std::string& contentJson)
{
    return client_.create_document(contentJson);
}

void SyncClient::updateDocument(const std::string& documentId, const std::string& contentJson)
{
    client_.update_document(documentId, contentJson);
}

void SyncClient::deleteDocument(const std::string& documentId)
{
    client_.delete_document(documentId);
}

std::string SyncClient::getDocument(const std::string& documentId)
{
    try
    {
        return client_.get_document(documentId);
    }
    catch (const SyncException&)
    {
        return {};
    }
}

std::string SyncClient::getAllDocuments()
{
    return client_.get_all_documents();
}

//==============================================================================
bool SyncClient::isConnected() const
{
    return const_cast<Client&>(client_).is_connected();
}

uint64_t SyncClient::countDocuments() const
{
    return const_cast<Client&>(client_).count_documents();
}

uint64_t SyncClient::countPendingSync() const
{
    return const_cast<Client&>(client_).count_pending_sync();
}

} // namespace replicant
