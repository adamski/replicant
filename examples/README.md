# Sync Client SDK Examples

This directory contains example projects demonstrating how to use the Sync Client SDK in C and C++ applications.

## Directory Structure

```
examples/
├── c/                  # C examples
│   ├── simple_example.c
│   ├── callback_example.c
│   └── CMakeLists.txt
├── cpp/                # C++ examples
│   ├── simple_example.cpp
│   ├── callback_example.cpp
│   └── CMakeLists.txt
└── README.md           # This file
```

## Prerequisites

1. **SDK Distribution**: Ensure the SDK has been built by running:
   ```bash
   cd ../
   ./scripts/build_dist.sh
   ```

2. **CMake**: Version 3.15 or higher
3. **C/C++ Compiler**: GCC, Clang, or MSVC

## Building Examples

### C Examples

```bash
cd examples/c
mkdir build && cd build
cmake ..
cmake --build .

# Run basic example
./simple_example

# Run callback example
./callback_example
```

### C++ Examples

```bash
cd examples/cpp
mkdir build && cd build
cmake ..
cmake --build .

# Run basic example
./simple_example

# Run callback example
./callback_example
```

## What the Examples Demonstrate

### simple_example.c
- Creating a sync engine instance
- Basic document operations (create, update, delete)
- Library version retrieval
- Proper cleanup and error handling

### callback_example.c
- Event callback registration and handling
- Real-time document change notifications
- Event filtering (all events vs specific types)
- Callback context management
- Event processing on the main thread

### simple_example.cpp
- C++ wrapper usage with RAII
- Exception handling for sync operations
- Modern C++ features (C++14)
- Safe memory management

### callback_example.cpp
- Thread-safe event handling without mutexes
- C++ callback registration and management
- Modern event processing patterns
- Cross-thread event queuing with single-thread execution
- Event statistics and monitoring

## SDK Integration

These examples show how external applications can integrate the Sync Client SDK:

1. **Headers**: Include `sync_client.h` for C or `sync_client.hpp` for C++
2. **Linking**: Link against `libsync_client` library
3. **Platform Libraries**: Additional system libraries required per platform
4. **CMake**: Use the provided CMakeLists.txt as templates

## Error Handling

Both examples demonstrate proper error handling:
- C: Check return codes (`SyncResult` enum)
- C++: Catch `sync_exception` for error conditions

## Running Without Server

These examples can run in offline mode without a running sync server. They will:
- Create local documents successfully
- Store data in local SQLite database
- Log connection errors (expected when no server is available)

## Event System Usage

The `callback_example.c` demonstrates the event-driven architecture:

1. **Register Callbacks**: Use `sync_engine_register_event_callback()` to listen for events
2. **Event Types**: Available events include:
   - `DocumentCreated` (0) - New document added
   - `DocumentUpdated` (1) - Document content changed  
   - `DocumentDeleted` (2) - Document removed
   - `SyncStarted` (3) - Synchronization began
   - `SyncCompleted` (4) - Synchronization finished
   - `SyncError` (5) - Synchronization failed
   - `ConflictDetected` (6) - Document conflict found
   - `ConnectionLost` (7) - Server connection lost
   - `ConnectionAttempted` (8) - Attempting server connection
   - `ConnectionSucceeded` (9) - Successfully connected to server

3. **Process Events**: Call `sync_engine_process_events()` regularly (e.g., in a timer)
4. **Thread Safety**: Events are queued from any thread but processed on the callback thread

## Extending the Examples

To add more advanced features:
- Custom document schemas and validation
- Network connectivity handling and retry logic
- Multi-client synchronization scenarios
- Performance monitoring and metrics
- Framework integration (JUCE, Qt, GTK)

See the SDK documentation and `FRAMEWORK_INTEGRATION.md` for detailed API reference.