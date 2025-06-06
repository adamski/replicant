/**
 * @file sync_client_events.h
 * @brief Event callback system for the JSON Database Sync Client
 * 
 * This header provides event listener functionality for the sync client,
 * allowing C/C++ applications to register callbacks for document and sync events.
 */

#ifndef SYNC_CLIENT_EVENTS_H
#define SYNC_CLIENT_EVENTS_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>
#include <stdbool.h>

/**
 * @brief Event types that can be emitted by the sync client
 */
typedef enum {
    SYNC_EVENT_DOCUMENT_CREATED = 0,        /**< A new document was created */
    SYNC_EVENT_DOCUMENT_UPDATED = 1,        /**< An existing document was updated */
    SYNC_EVENT_DOCUMENT_DELETED = 2,        /**< A document was deleted */
    SYNC_EVENT_SYNC_STARTED = 3,            /**< Synchronization process started */
    SYNC_EVENT_SYNC_COMPLETED = 4,          /**< Synchronization process completed */
    SYNC_EVENT_SYNC_ERROR = 5,              /**< An error occurred during sync */
    SYNC_EVENT_CONFLICT_DETECTED = 6,       /**< A conflict was detected between versions */
    SYNC_EVENT_CONNECTION_STATE_CHANGED = 7 /**< Connection state changed (connected/disconnected) */
} SyncEventType;

/**
 * @brief Event data structure passed to callbacks
 * 
 * Contains all relevant information about the event. Not all fields are populated
 * for all event types - check the event_type to determine which fields are valid.
 */
typedef struct {
    SyncEventType event_type;       /**< Type of event that occurred */
    const char* document_id;        /**< Document UUID (may be null for non-document events) */
    const char* title;              /**< Document title (may be null) */
    const char* content;            /**< Document content as JSON string (may be null) */
    const char* error;              /**< Error message (only for error events, may be null) */
    uint64_t numeric_data;          /**< Numeric data (e.g., sync count for SYNC_COMPLETED) */
    bool boolean_data;              /**< Boolean data (e.g., connection state for CONNECTION_STATE_CHANGED) */
} SyncEventData;

/**
 * @brief Event callback function type
 * 
 * @param event Pointer to event data structure (valid only during callback execution)
 * @param context User-defined context pointer passed during registration
 * 
 * @note The event pointer is only valid during the callback execution.
 *       Do not store or access it after the callback returns.
 * @note String pointers in the event data are temporary and will be freed
 *       after the callback returns. Copy them if you need to retain the data.
 * @note The callback should be fast and non-blocking. Avoid long-running
 *       operations or blocking I/O in the callback.
 */
typedef void (*SyncEventCallback)(const SyncEventData* event, void* context);

/**
 * @brief Result codes for sync client operations
 */
typedef enum {
    SYNC_RESULT_SUCCESS = 0,              /**< Operation completed successfully */
    SYNC_RESULT_ERROR_INVALID_INPUT = -1, /**< Invalid input parameters */
    SYNC_RESULT_ERROR_CONNECTION = -2,    /**< Connection or network error */
    SYNC_RESULT_ERROR_DATABASE = -3,      /**< Database operation error */
    SYNC_RESULT_ERROR_SERIALIZATION = -4, /**< JSON serialization/deserialization error */
    SYNC_RESULT_ERROR_UNKNOWN = -99       /**< Unknown error occurred */
} SyncResult;

/* Forward declarations - actual sync engine types are opaque */
struct CSyncEngine;

/**
 * @brief Register an event callback with the sync engine
 * 
 * @param engine Sync engine instance
 * @param callback Function to call when events occur
 * @param context User-defined context pointer (passed to callback)
 * @param event_filter Event type to filter for, or -1 to receive all events
 * 
 * @return SYNC_RESULT_SUCCESS on success, error code on failure
 * 
 * @note Multiple callbacks can be registered and will all be invoked
 * @note Callbacks are permanent - there is no unregister function in this version
 * @note The context pointer must remain valid for the lifetime of the sync engine
 * 
 * @example
 * ```c
 * void my_callback(const SyncEventData* event, void* context) {
 *     printf("Event: %d, Document: %s\n", event->event_type, 
 *            event->document_id ? event->document_id : "none");
 * }
 * 
 * // Register for all events
 * sync_engine_register_event_callback(engine, my_callback, NULL, -1);
 * 
 * // Register only for document creation events
 * sync_engine_register_event_callback(engine, my_callback, NULL, SYNC_EVENT_DOCUMENT_CREATED);
 * ```
 */
SyncResult sync_engine_register_event_callback(
    struct CSyncEngine* engine,
    SyncEventCallback callback,
    void* context,
    int event_filter
);

/**
 * @brief Process all queued events on the current thread
 * 
 * @param engine Sync engine instance
 * @param out_processed_count Optional output for number of events processed (can be NULL)
 * 
 * @return SYNC_RESULT_SUCCESS on success, error code on failure
 * 
 * @note This function MUST be called on the same thread where callbacks were registered.
 *       Events can be generated on any thread, but callbacks are only invoked on the
 *       thread that registered them. This ensures thread safety without requiring
 *       mutexes or locks in user callback code.
 * 
 * @note Call this function regularly (e.g., in your main loop) to process pending events.
 * 
 * @example
 * ```c
 * // In your main loop:
 * while (running) {
 *     uint32_t processed_count;
 *     SyncResult result = sync_engine_process_events(engine, &processed_count);
 *     if (result == SYNC_RESULT_SUCCESS && processed_count > 0) {
 *         printf("Processed %u events\n", processed_count);
 *     }
 *     
 *     // Do other work...
 *     usleep(16000); // ~60 FPS
 * }
 * ```
 */
SyncResult sync_engine_process_events(
    struct CSyncEngine* engine,
    uint32_t* out_processed_count
);

/* Debug/Testing Functions (only available in debug builds) */
#ifdef DEBUG

/**
 * @brief Emit a test event (debug builds only)
 * 
 * @param engine Sync engine instance
 * @param event_type Event type to emit (0-7)
 * 
 * @return SYNC_RESULT_SUCCESS on success, error code on failure
 * 
 * @note This function is only available in debug builds for testing callbacks
 */
SyncResult sync_engine_emit_test_event(
    struct CSyncEngine* engine,
    int event_type
);

/**
 * @brief Emit multiple test events in sequence (debug builds only)
 * 
 * @param engine Sync engine instance
 * @param count Number of events to emit (1-100)
 * 
 * @return SYNC_RESULT_SUCCESS on success, error code on failure
 * 
 * @note This function is only available in debug builds for stress testing callbacks
 */
SyncResult sync_engine_emit_test_event_burst(
    struct CSyncEngine* engine,
    int count
);

#endif /* DEBUG */

#ifdef __cplusplus
}
#endif

#endif /* SYNC_CLIENT_EVENTS_H */