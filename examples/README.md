# Sync Client SDK Examples

This directory contains example projects demonstrating how to use the Sync Client SDK in C and C++ applications.

## Directory Structure

```
examples/
├── c/                  # C examples
│   ├── simple_example.c
│   └── CMakeLists.txt
├── cpp/                # C++ examples
│   ├── simple_example.cpp
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

### C Example

```bash
cd examples/c
mkdir build && cd build
cmake ..
cmake --build .
./simple_example
```

### C++ Example

```bash
cd examples/cpp
mkdir build && cd build
cmake ..
cmake --build .
./simple_example
```

## What the Examples Demonstrate

### simple_example.c
- Creating a sync engine instance
- Basic document operations (create, update, delete)
- Library version retrieval
- Proper cleanup and error handling

### simple_example.cpp
- C++ wrapper usage with RAII
- Exception handling for sync operations
- Modern C++ features (C++14)
- Safe memory management

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

## Extending the Examples

To add more advanced features:
- Event callbacks for real-time updates
- Custom document schemas
- Network connectivity handling
- Multi-client synchronization

See the SDK documentation for detailed API reference.