# Desktop Framework Integration

## JUCE Integration

The `replicant` JUCE module provides the simplest integration path. It handles timer-based event processing automatically and delivers callbacks on the message thread.

### Prerequisites

1. JUCE 6.0+ project
2. Replicant SDK (`dist/` folder from build)
3. C++14 or later

### Project Setup (Projucer)

1. **Add the module path**
   - Open your `.jucer` file in Projucer
   - Go to *File > Global Paths*
   - Add the SDK path to *User Modules*: `/path/to/replicant-sdk/dist/juce`

2. **Enable the module**
   - In your project, go to *Modules*
   - Click *+* and select `replicant`

3. **Add library search path**
   - Go to your exporter settings (e.g., *Xcode (macOS)*)
   - Add to *Extra Library Search Paths*: `/path/to/replicant-sdk/dist/lib`

4. **Save and export**

### Project Setup (CMake)

```cmake
cmake_minimum_required(VERSION 3.15)
project(MyApp VERSION 1.0.0)

# Add JUCE
add_subdirectory(JUCE)

# Add Replicant module path
list(APPEND JUCE_MODULE_PATH "/path/to/replicant-sdk/dist/juce")

juce_add_gui_app(MyApp
    PRODUCT_NAME "My App")

target_sources(MyApp PRIVATE
    Source/Main.cpp
    Source/MainComponent.cpp)

target_compile_features(MyApp PRIVATE cxx_std_14)

target_link_libraries(MyApp PRIVATE
    juce::juce_core
    juce::juce_events
    juce::juce_gui_basics
    replicant)

# Link the replicant_client library
target_link_directories(MyApp PRIVATE /path/to/replicant-sdk/dist/lib)
```

### Basic Usage

```cpp
#include <replicant/replicant.h>

class MainComponent : public juce::Component
{
public:
    MainComponent()
        : sync("sqlite:local.db?mode=rwc",
               "wss://your-server.com/ws",
               "user@example.com",
               "rpa_your_api_key",
               "rps_your_api_secret")
    {
        // Callbacks run on the message thread - safe to update UI directly
        sync.onDocumentCreated = [this](const std::string& id,
                                        const std::string& title,
                                        const std::string& content)
        {
            addItemToList(id, title);
            repaint();
        };

        sync.onDocumentUpdated = [this](const std::string& id,
                                        const std::string& title,
                                        const std::string& content)
        {
            updateItemInList(id, title);
            repaint();
        };

        sync.onDocumentDeleted = [this](const std::string& id)
        {
            removeItemFromList(id);
            repaint();
        };

        sync.onConnectionChanged = [this](bool connected)
        {
            statusLabel.setText(connected ? "Online" : "Offline",
                                juce::dontSendNotification);
        };

        sync.onSyncError = [this](const std::string& message)
        {
            juce::AlertWindow::showMessageBoxAsync(
                juce::MessageBoxIconType::WarningIcon,
                "Sync Error", message);
        };
    }

    void createNewDocument()
    {
        try
        {
            auto id = sync.createDocument(R"({"title":"New Note","content":""})");
            DBG("Created document: " + juce::String(id));
        }
        catch (const replicant::SyncException& e)
        {
            DBG("Error: " + juce::String(e.what()));
        }
    }

private:
    replicant::Replicant sync;
    juce::Label statusLabel;

    // ... UI implementation
};
```

### API Reference

**Constructor**
```cpp
Replicant(const std::string& databaseUrl,    // e.g., "sqlite:data.db?mode=rwc"
          const std::string& serverUrl,       // e.g., "wss://server.com/ws"
          const std::string& email,           // User identifier
          const std::string& apiKey,          // rpa_ prefixed key
          const std::string& apiSecret);      // rps_ prefixed secret
```

**Document Operations** (all throw `replicant::SyncException` on failure)
```cpp
std::string createDocument(const std::string& contentJson);
void updateDocument(const std::string& documentId, const std::string& contentJson);
void deleteDocument(const std::string& documentId);
std::string getDocument(const std::string& documentId);
std::string getAllDocuments();  // Returns JSON array
```

**Status**
```cpp
bool isConnected();
uint64_t countDocuments();
uint64_t countPendingSync();
```

**Callbacks** (all execute on message thread)
```cpp
std::function<void(const std::string& id, const std::string& title, const std::string& content)> onDocumentCreated;
std::function<void(const std::string& id, const std::string& title, const std::string& content)> onDocumentUpdated;
std::function<void(const std::string& id)> onDocumentDeleted;
std::function<void(bool connected)> onConnectionChanged;
std::function<void(const std::string& message)> onSyncError;
```

---

## Qt Integration

Use `QTimer` with Qt's signal/slot system:

```cpp
#include <QTimer>
#include "replicant.hpp"

class SyncWindow : public QMainWindow
{
    Q_OBJECT

public:
    SyncWindow()
        : client("sqlite:client.db?mode=rwc",
                 "ws://localhost:8080/ws",
                 "user@example.com",
                 "rpa_key", "rps_secret")
    {
        client.register_document_callback(documentCallback, this);
        client.register_connection_callback(connectionCallback, this);

        timer = new QTimer(this);
        connect(timer, &QTimer::timeout, this, &SyncWindow::processEvents);
        timer->start(100);  // 10Hz
    }

private slots:
    void processEvents()
    {
        client.process_events();
    }

private:
    replicant::Client client;
    QTimer* timer;

    static void documentCallback(EventType type, const char* id,
                                 const char* title, const char* content, void* ctx)
    {
        auto* self = static_cast<SyncWindow*>(ctx);
        // Handle on main thread
    }

    static void connectionCallback(EventType type, bool connected,
                                   uint32_t attempt, void* ctx)
    {
        auto* self = static_cast<SyncWindow*>(ctx);
        // Update connection status
    }
};
```

---

## GTK Integration

Use `g_timeout_add()`:

```c
#include <gtk/gtk.h>
#include "replicant.h"

typedef struct {
    Replicant* client;
    GtkLabel* status;
} AppData;

gboolean process_events(gpointer data)
{
    AppData* app = (AppData*)data;
    uint32_t count;
    replicant_process_events(app->client, &count);
    return G_SOURCE_CONTINUE;
}

void on_document_created(EventType type, const char* id,
                         const char* title, const char* content, void* ctx)
{
    AppData* app = (AppData*)ctx;
    gtk_label_set_text(app->status, "Document created");
}

int main(int argc, char* argv[])
{
    gtk_init(&argc, &argv);

    AppData app = {0};
    app.client = replicant_create(
        "sqlite:client.db?mode=rwc",
        "ws://localhost:8080/ws",
        "user@example.com",
        "rpa_key", "rps_secret");

    replicant_register_document_callback(app.client, on_document_created, &app, -1);

    g_timeout_add(100, process_events, &app);  // 10Hz

    // Create UI...
    gtk_main();

    replicant_destroy(app.client);
    return 0;
}
```

---

## Summary

| Framework | Timer Mechanism | Module/Wrapper |
|-----------|-----------------|----------------|
| JUCE | Built into `replicant::Replicant` | `replicant` JUCE module |
| Qt | `QTimer` | `replicant::Client` (C++ wrapper) |
| GTK | `g_timeout_add()` | C API directly |

All integrations follow the same principle:
- Call `process_events()` periodically (10Hz is sufficient)
- Callbacks execute on the main/UI thread
- No additional synchronization needed
