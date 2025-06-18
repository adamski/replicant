cmake_minimum_required(VERSION 3.15)
project(SyncClientCppExample LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 14)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Include the SDK headers
include_directories(${CMAKE_CURRENT_SOURCE_DIR}/../../dist/include)

# Find the sync client library
find_library(SYNC_CLIENT_LIB
    NAMES sync_client
    PATHS ${CMAKE_CURRENT_SOURCE_DIR}/../../dist/lib
    NO_DEFAULT_PATH
    REQUIRED
)

# Platform-specific libraries
if(APPLE)
    set(PLATFORM_LIBS "-framework Security" "-framework Foundation")
elseif(UNIX)
    set(PLATFORM_LIBS pthread dl m)
elseif(WIN32)
    set(PLATFORM_LIBS ws2_32 userenv bcrypt)
endif()

# C++ examples
add_executable(simple_example simple_example.cpp)
target_link_libraries(simple_example ${SYNC_CLIENT_LIB} ${PLATFORM_LIBS})

add_executable(callback_example callback_example.cpp)
target_link_libraries(callback_example ${SYNC_CLIENT_LIB} ${PLATFORM_LIBS})


# Print build information
message(STATUS "Building C++ example against SDK:")
message(STATUS "  Headers: ${CMAKE_CURRENT_SOURCE_DIR}/../../dist/include")
message(STATUS "  Library: ${SYNC_CLIENT_LIB}")
message(STATUS "  C++ Standard: ${CMAKE_CXX_STANDARD}")