#include "replicant.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main()
{
    // Create Replicant client with HMAC authentication
    struct Replicant* client = replicant_create(
        "sqlite:client.db?mode=rwc",
        "ws://localhost:8080/ws",
        "test-user@example.com",
        "rpa_test_api_key_example_12345",
        "rps_test_api_secret_example_67890"
    );

    if (!client)
    {
        printf("Failed to create Replicant client\n");
        return 1;
    }

    // Get version
    char* version = replicant_get_version();
    if (version)
    {
        printf("Replicant client version: %s\n", version);
        replicant_string_free(version);
    }

    // Create a document
    char doc_id[37] = {0};
    enum SyncResult result = replicant_create_document(
        client,
        "{\"title\":\"My Document\",\"content\":\"Hello World\",\"type\":\"note\",\"priority\":\"medium\"}",
        doc_id
    );

    if (result == Success)
    {
        printf("Created document: %s\n", doc_id);

        // Update the document
        result = replicant_update_document(
            client,
            doc_id,
            "{\"content\":\"Hello Updated World\",\"type\":\"note\",\"priority\":\"high\"}"
        );

        if (result == Success)
        {
            printf("Updated document successfully\n");

            // Delete the document
            result = replicant_delete_document(client, doc_id);

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
    replicant_destroy(client);
    return 0;
}
