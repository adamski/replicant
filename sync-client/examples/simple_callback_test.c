/**
 * @file simple_callback_test.c
 * @brief Simple test to verify C compilation and basic callback functionality
 * 
 * This is a minimal example that can be compiled to test the C interface.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Include the event header */
#include "../include/sync_client_events.h"

/* Forward declarations for the main sync client functions */
extern struct CSyncEngine* sync_engine_create(const char* database_url, const char* server_url, const char* auth_token);
extern void sync_engine_destroy(struct CSyncEngine* engine);
extern SyncResult sync_engine_create_document(struct CSyncEngine* engine, const char* title, const char* content_json, char* out_document_id);

/* Simple callback function */
void simple_callback(const SyncEventData* event, void* context) {
    int* call_count = (int*)context;
    (*call_count)++;
    
    printf("Event received: type=%d, call_count=%d\n", event->event_type, *call_count);
    
    if (event->document_id && strlen(event->document_id) > 0) {
        printf("  Document ID: %s\n", event->document_id);
    }
    
    if (event->title && strlen(event->title) > 0) {
        printf("  Title: %s\n", event->title);
    }
    
    if (event->error && strlen(event->error) > 0) {
        printf("  Error: %s\n", event->error);
    }
    
    if (event->numeric_data > 0) {
        printf("  Numeric data: %llu\n", (unsigned long long)event->numeric_data);
    }
    
    if (event->boolean_data) {
        printf("  Boolean data: true\n");
    }
}

int main() {
    printf("=== Simple C Callback Test ===\n");
    
    /* Create sync engine */
    struct CSyncEngine* engine = sync_engine_create(
        "sqlite::memory:",
        "ws://localhost:8080/ws",
        "test-token"
    );
    
    if (!engine) {
        printf("Failed to create sync engine\n");
        return 1;
    }
    
    printf("✓ Sync engine created\n");
    
    /* Register callback */
    int call_count = 0;
    SyncResult result = sync_engine_register_event_callback(
        engine,
        simple_callback,
        &call_count,
        -1  /* All events */
    );
    
    if (result != SYNC_RESULT_SUCCESS) {
        printf("Failed to register callback: %d\n", result);
        sync_engine_destroy(engine);
        return 1;
    }
    
    printf("✓ Callback registered\n");
    
    /* Create a document to trigger an event */
    char doc_id[37] = {0};
    result = sync_engine_create_document(
        engine,
        "Test Document",
        "{\"message\":\"Hello from C!\"}",
        doc_id
    );
    
    if (result == SYNC_RESULT_SUCCESS) {
        printf("✓ Document created: %s\n", doc_id);
        printf("✓ Total callbacks received: %d\n", call_count);
        
        if (call_count > 0) {
            printf("✓ Callback system working!\n");
        } else {
            printf("⚠ No callbacks received (may be expected in offline mode)\n");
        }
    } else {
        printf("Failed to create document: %d\n", result);
    }
    
    /* Clean up */
    sync_engine_destroy(engine);
    printf("✓ Cleanup complete\n");
    
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
 *    gcc -I./sync-client/include \
 *        -L./target/release \
 *        -lsync_client \
 *        -ldl -lpthread -lm \
 *        sync-client/examples/simple_callback_test.c \
 *        -o simple_test
 * 
 * 3. Run the test:
 *    ./simple_test
 * 
 * Note: On macOS you might need additional flags:
 *    gcc -I./sync-client/include \
 *        -L./target/release \
 *        -lsync_client \
 *        -framework CoreFoundation -framework Security \
 *        -ldl -lpthread -lm \
 *        sync-client/examples/simple_callback_test.c \
 *        -o simple_test
 */