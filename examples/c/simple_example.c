#include "sync_client.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main()
{
    // Create sync engine with HMAC authentication
    struct SyncEngine* engine = sync_engine_create(
        "sqlite:client.db?mode=rwc",
        "ws://localhost:8080/ws",
        "test-user@example.com",
        "rpa_test_api_key_example_12345",
        "rps_test_api_secret_example_67890"
    );
    
    if (!engine) 
    {
        printf("Failed to create sync engine\n");
        return 1;
    }
    
    // Get version
    char* version = sync_get_version();
    if (version) 
    {
        printf("Sync client version: %s\n", version);
        sync_string_free(version);
    }
    
    // Create a document
    char doc_id[37] = {0};
    enum SyncResult result = sync_engine_create_document(
        engine,
        "{\"title\":\"My Document\",\"content\":\"Hello World\",\"type\":\"note\",\"priority\":\"medium\"}",
        doc_id
    );
    
    if (result == Success) 
    {
        printf("Created document: %s\n", doc_id);
        
        // Update the document
        result = sync_engine_update_document(
            engine,
            doc_id,
            "{\"content\":\"Hello Updated World\",\"type\":\"note\",\"priority\":\"high\"}"
        );
        
        if (result == Success) 
        {
            printf("Updated document successfully\n");
            
            // Delete the document
            result = sync_engine_delete_document(engine, doc_id);
            
            if (result == Success) 
            {
                printf("Deleted document successfully\n");
            } 
            else 
            {
                printf("Failed to delete document: %d\n", result);
            }
        } 
        else 
        {
            printf("Failed to update document: %d\n", result);
        }
    } 
    else 
    {
        printf("Failed to create document: %d\n", result);
    }
    
    // Clean up
    sync_engine_destroy(engine);
    return 0;
}