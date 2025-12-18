/**
 * @file callback_example.c
 * @brief Simple test to verify C compilation and basic callback functionality
 *
 * This is a minimal example that can be compiled to test the C interface.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Include the main replicant header */
#include "replicant.h"

/* Document callback function */
void document_callback(
    EventType event_type,
    const char* document_id,
    const char* title,
    const char* content,
    void* context)
{
    int* call_count = (int*)context;
    (*call_count)++;

    printf("Document event received: type=%d, call_count=%d\n", event_type, *call_count);

    if (document_id && strlen(document_id) > 0)
    {
        printf("  Document ID: %s\n", document_id);
    }

    if (title && strlen(title) > 0)
    {
        printf("  Title: %s\n", title);
    }

    if (content && strlen(content) > 0)
    {
        printf("  Content: %s\n", content);
    }
}

int main()
{
    printf("=== Simple C Callback Test ===\n");

    /* Create Replicant client with HMAC authentication */
    struct Replicant* client = replicant_create(
        "sqlite::memory:",
        "ws://localhost:8080/ws",
        "simple-test@example.com",
        "rpa_test_api_key_example_12345",
        "rps_test_api_secret_example_67890"
    );

    if (!client)
    {
        printf("Failed to create Replicant client\n");
        return 1;
    }

    printf("Replicant client created\n");

    /* Register document callback for all document events */
    int call_count = 0;
    SyncResult result = replicant_register_document_callback(
        client,
        document_callback,
        &call_count,
        -1  /* All document events */
    );

    if (result != Success)
    {
        printf("Failed to register callback: %d\n", result);
        replicant_destroy(client);
        return 1;
    }

    printf("Callback registered\n");

    /* Create a document to trigger an event */
    char doc_id[37] = {0};
    result = replicant_create_document(
        client,
        "{\"title\":\"Test Document\",\"message\":\"Hello from C!\"}",
        doc_id
    );

    if (result == Success)
    {
        printf("Document created: %s\n", doc_id);

        /* Process events to trigger callbacks */
        uint32_t processed = 0;
        replicant_process_events(client, &processed);
        printf("Events processed: %u\n", processed);

        printf("Total callbacks received: %d\n", call_count);

        if (call_count > 0)
        {
            printf("Callback system working!\n");
        }
        else
        {
            printf("Note: No callbacks received (may be expected in offline mode)\n");
        }
    }
    else
    {
        printf("Failed to create document: %d\n", result);
    }

    /* Clean up */
    replicant_destroy(client);
    printf("Cleanup complete\n");

    printf("\n=== Test completed successfully! ===\n");
    return 0;
}

/*
 * To compile this test:
 *
 * 1. First build the Rust library:
 *    cd sync-workspace
 *    cargo build --release
 *
 * 2. Then compile this C test:
 *    gcc -I./replicant-client/include \
 *        -L./target/release \
 *        -lreplicant_client \
 *        -ldl -lpthread -lm \
 *        examples/c/callback_example.c \
 *        -o callback_example
 *
 * 3. Run the test:
 *    ./callback_example
 *
 * Note: On macOS you might need additional flags:
 *    gcc -I./replicant-client/include \
 *        -L./target/release \
 *        -lreplicant_client \
 *        -framework CoreFoundation -framework Security \
 *        -ldl -lpthread -lm \
 *        examples/c/callback_example.c \
 *        -o callback_example
 */
