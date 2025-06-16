/**
 * @file c_event_callbacks.c
 * @brief Example demonstrating event callbacks in the sync client
 * 
 * This example shows how to:
 * 1. Register event callbacks for different event types
 * 2. Handle various sync events (document operations, sync progress, errors)
 * 3. Use context pointers to maintain state
 * 4. Filter events by type
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

/* Include the main sync client header */
/* Note: In a real build system, you'd link against the compiled library */
/* This is just for demonstration - the actual header paths may differ */
#include "../include/sync_client_events.h"

/* Forward declaration of the main sync client functions */
/* These would normally be in sync_client.h */
struct CSyncEngine;
typedef enum {
    Success = 0,
    ErrorInvalidInput = -1,
    ErrorConnection = -2,
    ErrorDatabase = -3,
    ErrorSerialization = -4,
    ErrorUnknown = -99,
} CSyncResult;

extern struct CSyncEngine* sync_engine_create(const char* database_url, const char* server_url, const char* auth_token, const char* user_identifier);
extern void sync_engine_destroy(struct CSyncEngine* engine);
extern CSyncResult sync_engine_create_document(struct CSyncEngine* engine, const char* title, const char* content_json, char* out_document_id);
extern CSyncResult sync_engine_update_document(struct CSyncEngine* engine, const char* document_id, const char* content_json);
extern CSyncResult sync_engine_delete_document(struct CSyncEngine* engine, const char* document_id);

/* Context structure to track callback statistics */
typedef struct {
    int document_created_count;
    int document_updated_count;
    int document_deleted_count;
    int sync_started_count;
    int sync_completed_count;
    int sync_error_count;
    int conflict_count;
    int connection_state_changes;
    char last_document_id[64];
    char last_error_message[256];
} CallbackStats;

/* Callback function for all events */
void all_events_callback(const SyncEventData* event, void* context) {
    CallbackStats* stats = (CallbackStats*)context;
    
    printf("[ALL_EVENTS] Event type: %d", event->event_type);
    
    switch (event->event_type) {
        case SYNC_EVENT_DOCUMENT_CREATED:
            stats->document_created_count++;
            if (event->document_id) {
                strncpy(stats->last_document_id, event->document_id, sizeof(stats->last_document_id) - 1);
                printf(" - Document created: %s", event->document_id);
                if (event->title) {
                    printf(" (title: %s)", event->title);
                }
            }
            break;
            
        case SYNC_EVENT_DOCUMENT_UPDATED:
            stats->document_updated_count++;
            if (event->document_id) {
                printf(" - Document updated: %s", event->document_id);
                if (event->title) {
                    printf(" (title: %s)", event->title);
                }
            }
            break;
            
        case SYNC_EVENT_DOCUMENT_DELETED:
            stats->document_deleted_count++;
            if (event->document_id) {
                printf(" - Document deleted: %s", event->document_id);
            }
            break;
            
        case SYNC_EVENT_SYNC_STARTED:
            stats->sync_started_count++;
            printf(" - Sync started");
            break;
            
        case SYNC_EVENT_SYNC_COMPLETED:
            stats->sync_completed_count++;
            printf(" - Sync completed (%llu documents)", (unsigned long long)event->numeric_data);
            break;
            
        case SYNC_EVENT_SYNC_ERROR:
            stats->sync_error_count++;
            if (event->error) {
                strncpy(stats->last_error_message, event->error, sizeof(stats->last_error_message) - 1);
                printf(" - Sync error: %s", event->error);
            }
            break;
            
        case SYNC_EVENT_CONFLICT_DETECTED:
            stats->conflict_count++;
            if (event->document_id) {
                printf(" - Conflict detected for document: %s", event->document_id);
            }
            break;
            
        case SYNC_EVENT_CONNECTION_STATE_CHANGED:
            stats->connection_state_changes++;
            printf(" - Connection state changed: %s", event->boolean_data ? "connected" : "disconnected");
            break;
            
        default:
            printf(" - Unknown event type");
            break;
    }
    
    printf("\n");
}

/* Callback function for document events only */
void document_events_callback(const SyncEventData* event, void* context) {
    const char* prefix = (const char*)context;
    
    printf("[%s] Document event: ", prefix);
    
    switch (event->event_type) {
        case SYNC_EVENT_DOCUMENT_CREATED:
            printf("CREATED");
            break;
        case SYNC_EVENT_DOCUMENT_UPDATED:
            printf("UPDATED");
            break;
        case SYNC_EVENT_DOCUMENT_DELETED:
            printf("DELETED");
            break;
        default:
            printf("UNKNOWN");
            break;
    }
    
    if (event->document_id) {
        printf(" - ID: %s", event->document_id);
    }
    if (event->title) {
        printf(" - Title: %s", event->title);
    }
    
    printf("\n");
}

/* Print callback statistics */
void print_stats(const CallbackStats* stats) {
    printf("\n=== Callback Statistics ===\n");
    printf("Documents created: %d\n", stats->document_created_count);
    printf("Documents updated: %d\n", stats->document_updated_count);
    printf("Documents deleted: %d\n", stats->document_deleted_count);
    printf("Sync operations started: %d\n", stats->sync_started_count);
    printf("Sync operations completed: %d\n", stats->sync_completed_count);
    printf("Sync errors: %d\n", stats->sync_error_count);
    printf("Conflicts detected: %d\n", stats->conflict_count);
    printf("Connection state changes: %d\n", stats->connection_state_changes);
    if (strlen(stats->last_document_id) > 0) {
        printf("Last document ID: %s\n", stats->last_document_id);
    }
    if (strlen(stats->last_error_message) > 0) {
        printf("Last error: %s\n", stats->last_error_message);
    }
    printf("===========================\n\n");
}

int main() {
    printf("=== Sync Client Event Callbacks Example ===\n\n");
    
    /* Initialize callback statistics */
    CallbackStats stats = {0};
    
    /* Create sync engine (works offline if server is not available) */
    struct CSyncEngine* engine = sync_engine_create(
        "sqlite:client_events_example.db?mode=rwc",
        "ws://localhost:8080/ws",
        "demo-token",
        "callback-test@example.com"
    );
    
    if (!engine) {
        printf("Failed to create sync engine\n");
        return 1;
    }
    
    printf("Sync engine created successfully\n");
    
    /* Register callback for all events */
    SyncResult result = sync_engine_register_event_callback(
        engine,
        all_events_callback,
        &stats,
        -1  /* -1 means all event types */
    );
    
    if (result != SYNC_RESULT_SUCCESS) {
        printf("Failed to register all-events callback: %d\n", result);
        sync_engine_destroy(engine);
        return 1;
    }
    
    printf("Registered callback for all events\n");
    
    /* Register additional callback for document events only */
    const char* doc_prefix = "DOC_ONLY";
    result = sync_engine_register_event_callback(
        engine,
        document_events_callback,
        (void*)doc_prefix,
        SYNC_EVENT_DOCUMENT_CREATED  /* Only document created events */
    );
    
    if (result != SYNC_RESULT_SUCCESS) {
        printf("Failed to register document-only callback: %d\n", result);
        sync_engine_destroy(engine);
        return 1;
    }
    
    printf("Registered callback for document creation events\n\n");
    
    /* Test document operations to trigger events */
    printf("=== Testing Document Operations ===\n");
    
    /* Create a document */
    printf("Creating document...\n");
    char doc_id[37] = {0};
    result = sync_engine_create_document(
        engine,
        "Test Document",
        "{\"content\":\"Hello from event callbacks!\",\"type\":\"note\",\"priority\":\"high\"}",
        doc_id
    );
    
    if (result == Success) {
        printf("Document created with ID: %s\n", doc_id);
        
        /* Small delay to see events */
        usleep(100000);  /* 100ms */
        
        /* Update the document */
        printf("Updating document...\n");
        result = sync_engine_update_document(
            engine,
            doc_id,
            "{\"content\":\"Updated content!\",\"type\":\"note\",\"priority\":\"medium\",\"updated\":true}"
        );
        
        if (result == Success) {
            printf("Document updated successfully\n");
            usleep(100000);  /* 100ms */
            
            /* Delete the document */
            printf("Deleting document...\n");
            result = sync_engine_delete_document(engine, doc_id);
            
            if (result == Success) {
                printf("Document deleted successfully\n");
                usleep(100000);  /* 100ms */
            } else {
                printf("Failed to delete document: %d\n", result);
            }
        } else {
            printf("Failed to update document: %d\n", result);
        }
    } else {
        printf("Failed to create document: %d\n", result);
    }
    
    /* Test events with debug functions (if available in debug builds) */
    #ifdef DEBUG
    printf("\n=== Testing Debug Events ===\n");
    
    printf("Triggering test sync started event...\n");
    sync_engine_emit_test_event(engine, SYNC_EVENT_SYNC_STARTED);
    usleep(50000);
    
    printf("Triggering test sync completed event...\n");
    sync_engine_emit_test_event(engine, SYNC_EVENT_SYNC_COMPLETED);
    usleep(50000);
    
    printf("Triggering test error event...\n");
    sync_engine_emit_test_event(engine, SYNC_EVENT_SYNC_ERROR);
    usleep(50000);
    
    printf("Triggering burst of events...\n");
    sync_engine_emit_test_event_burst(engine, 3);
    usleep(100000);
    
    #else
    printf("\n=== Debug Events Not Available ===\n");
    printf("Compile with DEBUG flag to enable test event functions\n");
    #endif
    
    /* Print final statistics */
    print_stats(&stats);
    
    /* Clean up */
    printf("Cleaning up...\n");
    sync_engine_destroy(engine);
    
    printf("Example completed successfully!\n");
    return 0;
}

/*
 * Compilation instructions:
 * 
 * This example demonstrates the API but won't compile without the actual
 * sync client library. To compile and run:
 * 
 * 1. Build the sync client library:
 *    cd sync-workspace
 *    cargo build --release
 * 
 * 2. Compile this example (adjust paths as needed):
 *    gcc -I./sync-client/include \
 *        -L./target/release \
 *        -lsync_client \
 *        sync-client/examples/c_event_callbacks.c \
 *        -o event_callbacks_example
 * 
 * 3. Run the example:
 *    ./event_callbacks_example
 * 
 * Expected output:
 * - Callback registrations
 * - Document creation, update, delete events
 * - Statistics showing event counts
 * - All events logged with context information
 */