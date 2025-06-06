# Desktop Framework Integration

The sync client callback system integrates with desktop frameworks through periodic calls to `process_events()`.

## Integration Pattern

1. Events are queued from any thread
2. Framework calls `process_events()` regularly (e.g., via timer)
3. Callbacks execute on the framework's main thread

## JUCE Integration

Use `juce::Timer` to process events:

```cpp
#include "sync_client_events.h"

class SyncComponent : public juce::Component, private juce::Timer
{
public:
    SyncComponent()
    {
        engine_ = sync_engine_create("sqlite:client.db", "ws://localhost:8080/ws", "token");
        sync_engine_register_event_callback(engine_, eventCallback, this, -1);
        startTimer(16); // ~60 FPS
    }
    
    ~SyncComponent() override
    {
        stopTimer();
        sync_engine_destroy(engine_);
    }

private:
    CSyncEngine* engine_;
    
    void timerCallback() override
    {
        uint32_t count;
        sync_engine_process_events(engine_, &count);
    }
    
    static void eventCallback(const SyncEventData* event, void* context)
    {
        auto* comp = static_cast<SyncComponent*>(context);
        comp->handleEvent(event);
    }
    
    void handleEvent(const SyncEventData* event)
    {
        switch (event->event_type)
        {
            case SYNC_EVENT_DOCUMENT_CREATED:
                // Update UI
                break;
            case SYNC_EVENT_SYNC_COMPLETED:
                // Update status
                break;
        }
    }
};
```

## Qt Integration

Use `QTimer` with Qt's signal/slot system:

```cpp
#include <QTimer>
#include "sync_client_events.h"

class SyncWindow : public QMainWindow
{
    Q_OBJECT

public:
    SyncWindow()
    {
        engine_ = sync_engine_create("sqlite:client.db", "ws://localhost:8080/ws", "token");
        sync_engine_register_event_callback(engine_, eventCallback, this, -1);
        
        timer_ = new QTimer(this);
        connect(timer_, &QTimer::timeout, this, &SyncWindow::processEvents);
        timer_->start(16);
    }
    
    ~SyncWindow()
    {
        sync_engine_destroy(engine_);
    }

private slots:
    void processEvents()
    {
        uint32_t count;
        sync_engine_process_events(engine_, &count);
    }
    
    void onDocumentCreated(const QString& title)
    {
        // Update UI
    }

signals:
    void documentCreated(const QString& title);

private:
    CSyncEngine* engine_;
    QTimer* timer_;
    
    static void eventCallback(const SyncEventData* event, void* context)
    {
        auto* window = static_cast<SyncWindow*>(context);
        
        switch (event->event_type)
        {
            case SYNC_EVENT_DOCUMENT_CREATED:
                emit window->documentCreated(QString::fromUtf8(event->title));
                break;
        }
    }
};
```

## GTK Integration

Use `g_timeout_add()`:

```c
#include <gtk/gtk.h>
#include "sync_client_events.h"

typedef struct {
    CSyncEngine* engine;
    GtkLabel* status;
} AppData;

gboolean process_events(gpointer data) 
{
    AppData* app = (AppData*)data;
    uint32_t count;
    sync_engine_process_events(app->engine, &count);
    return G_SOURCE_CONTINUE;
}

void event_callback(const SyncEventData* event, void* context) 
{
    AppData* app = (AppData*)context;
    
    switch (event->event_type) 
    {
        case SYNC_EVENT_DOCUMENT_CREATED:
            gtk_label_set_text(app->status, "Document created");
            break;
    }
}

int main(int argc, char* argv[]) 
{
    gtk_init(&argc, &argv);
    
    AppData app = {0};
    app.engine = sync_engine_create("sqlite:client.db", "ws://localhost:8080/ws", "token");
    sync_engine_register_event_callback(app.engine, event_callback, &app, -1);
    
    g_timeout_add(16, process_events, &app);
    
    // Create UI...
    gtk_main();
    
    sync_engine_destroy(app.engine);
    return 0;
}
```

## Summary

All frameworks follow the same pattern:
- Register callbacks once
- Use framework timer to call `process_events()` regularly
- Handle events on main thread with no additional synchronization needed