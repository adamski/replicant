use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Wrap,
    },
    Frame, Terminal,
};
use serde_json::{json, Value};
use sqlx::Row;
use std::{
    error::Error,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::time::sleep;
use sync_client::{ClientDatabase, SyncEngine};
use sync_core::models::Document;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "task-list")]
#[command(about = "Task list manager with real-time sync", long_about = None)]
struct Cli {
    /// Database file name (will auto-create in databases/ directory)
    #[arg(short, long, default_value = "tasks")]
    database: String,

    /// Auto-generate unique database name for concurrent testing
    #[arg(short, long)]
    auto: bool,

    /// User identifier (email/username) for shared identity across clients
    #[arg(short, long)]
    user: Option<String>,

    /// Server WebSocket URL
    #[arg(short, long, default_value = "ws://localhost:8080/ws")]
    server: String,

    /// Authentication token
    #[arg(short, long, default_value = "demo-token")]
    token: String,
}

#[derive(Clone)]
struct Task {
    id: Uuid,
    title: String,
    description: String,
    status: String,
    priority: String,
    tags: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    version: i64,
    sync_status: Option<String>,
}

struct ActivityEntry {
    timestamp: chrono::DateTime<chrono::Utc>,
    message: String,
    event_type: ActivityType,
}

#[derive(Clone, Copy)]
enum ActivityType {
    Created,
    Updated,
    Deleted,
    SyncStarted,
    SyncCompleted,
    Connected,
    Disconnected,
    Error,
}

struct AppState {
    tasks: Vec<Task>,
    selected_task: usize,
    activity_log: Vec<ActivityEntry>,
    sync_status: SyncStatus,
    last_sync: Option<Instant>,
    should_quit: bool,
    needs_refresh: bool,
    last_refresh: Option<Instant>,
    database_name: String,
}

#[derive(Clone)]
struct SyncStatus {
    connected: bool,
    pending_count: usize,
    conflict_count: usize,
    connection_state: String,
    last_attempt: Option<Instant>,
    next_retry: Option<Instant>,
}

impl AppState {
    fn new(database_name: String) -> Self {
        Self {
            tasks: Vec::new(),
            selected_task: 0,
            activity_log: Vec::new(),
            sync_status: SyncStatus {
                connected: false,
                pending_count: 0,
                conflict_count: 0,
                connection_state: "Starting...".to_string(),
                last_attempt: None,
                next_retry: None,
            },
            last_sync: None,
            should_quit: false,
            needs_refresh: true,
            last_refresh: None,
            database_name,
        }
    }

    fn add_activity(&mut self, message: String, event_type: ActivityType) {
        self.activity_log.push(ActivityEntry {
            timestamp: chrono::Utc::now(),
            message,
            event_type,
        });
        // Keep only last 20 entries
        if self.activity_log.len() > 20 {
            self.activity_log.remove(0);
        }
    }

    fn get_selected_task(&self) -> Option<&Task> {
        self.tasks.get(self.selected_task)
    }

    fn move_selection_up(&mut self) {
        if self.selected_task > 0 {
            self.selected_task -= 1;
        }
    }

    fn move_selection_down(&mut self) {
        if self.selected_task < self.tasks.len().saturating_sub(1) {
            self.selected_task += 1;
        }
    }
}

// Thread-safe wrapper for app state
type SharedState = Arc<Mutex<AppState>>;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging with minimal output to avoid interfering with TUI
    // Use RUST_LOG=off to completely disable logging for clean TUI
    let log_level = std::env::var("RUST_LOG").unwrap_or_else(|_| "off".to_string());
    if log_level != "off" {
        tracing_subscriber::fmt()
            .with_env_filter(&log_level)
            .with_writer(std::io::stderr)
            .init();
    }

    let cli = Cli::parse();

    // Setup database
    std::fs::create_dir_all("databases")?;
    
    // Generate database name
    let db_name = if cli.auto {
        // Auto-generate unique name with timestamp and random suffix
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let random_suffix = Uuid::new_v4().to_string()[..8].to_string();
        format!("client_{}_{}", timestamp, random_suffix)
    } else {
        cli.database.clone()
    };
    
    let db_file = format!("databases/{}.sqlite3", db_name);
    let db_url = format!("sqlite:{}?mode=rwc", db_file);
    
    let db = Arc::new(ClientDatabase::new(&db_url).await?);
    db.run_migrations().await?;

    // Get or create user
    let user_id = match db.get_user_id().await {
        Ok(id) => id,
        Err(_) => {
            // Generate deterministic user ID based on user identifier or create random
            let id = if let Some(user_identifier) = &cli.user {
                // Use UUID v5 with namespace to create deterministic UUID from username/email
                // Use a custom namespace for our application
                let namespace = Uuid::parse_str("6ba7b810-9dad-11d1-80b4-00c04fd430c8").unwrap(); // Using DNS namespace
                Uuid::new_v5(&namespace, user_identifier.as_bytes())
            } else {
                Uuid::new_v4()
            };
            
            // Client ID should always be unique per client instance
            let client_id = Uuid::new_v4();
            setup_user(&db, id, client_id, &cli.server, &cli.token).await?;
            id
        }
    };

    // Create shared state
    let state = Arc::new(Mutex::new(AppState::new(db_name.clone())));

    // Load initial tasks
    load_tasks(&db, user_id, state.clone()).await?;

    // Setup terminal first - we want to show the UI immediately
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initialize UI with startup message
    {
        let mut app_state = state.lock().unwrap();
        app_state.add_activity("Task Manager starting...".to_string(), ActivityType::SyncStarted);
        if cli.auto {
            app_state.add_activity(format!("Created new database: {}", db_name), ActivityType::SyncStarted);
        } else {
            app_state.add_activity(format!("Using database: {}", db_name), ActivityType::SyncStarted);
        }
        if let Some(user_identifier) = &cli.user {
            app_state.add_activity(format!("Using shared identity: {}", user_identifier), ActivityType::SyncStarted);
        }
        app_state.sync_status.connection_state = "Starting...".to_string();
    }

    // Start connection attempt in background
    let state_clone = state.clone();
    let server_url = cli.server.clone();
    let token = cli.token.clone();
    let db_url_clone = db_url.clone();
    
    let sync_engine = Arc::new(Mutex::new(None::<Arc<SyncEngine>>));
    let sync_engine_clone = sync_engine.clone();
    
    // Spawn background task for connection with periodic retry
    tokio::spawn(async move {
        // Show connection attempt
        {
            let mut app_state = state_clone.lock().unwrap();
            app_state.add_activity(format!("Connecting to {}", server_url), ActivityType::SyncStarted);
            app_state.sync_status.connection_state = "Connecting...".to_string();
        }
        
        let mut retry_interval = Duration::from_secs(5); // Start with 5 seconds
        let max_retry_interval = Duration::from_secs(30); // Cap at 30 seconds
        
        loop {
            // Try to connect to server
            match SyncEngine::new(&db_url_clone, &server_url, &token).await {
                Ok(mut engine) => {
                    // Don't register callbacks here - they will be registered in the main thread

                    // Try to start the sync engine
                    match engine.start().await {
                        Ok(_) => {
                            {
                                let mut app_state = state_clone.lock().unwrap();
                                app_state.sync_status.connected = true;
                                app_state.sync_status.connection_state = "Connected".to_string();
                                app_state.add_activity("Successfully connected to server".to_string(), ActivityType::Connected);
                            }
                            
                            // Store the engine for use
                            *sync_engine_clone.lock().unwrap() = Some(Arc::new(engine));
                            
                            // Give initial sync time to complete
                            sleep(Duration::from_millis(500)).await;
                            
                            // Trigger a refresh to show synced documents
                            {
                                let mut app_state = state_clone.lock().unwrap();
                                app_state.needs_refresh = true;
                            }
                            
                            // Reset retry interval on successful connection
                            retry_interval = Duration::from_secs(5);
                            
                            // Connection successful, break out of retry loop
                            break;
                        }
                        Err(e) => {
                            let mut app_state = state_clone.lock().unwrap();
                            app_state.sync_status.connected = false;
                            app_state.sync_status.connection_state = "Offline (connection failed)".to_string();
                            app_state.add_activity(format!("Connection failed: {}", e), ActivityType::Error);
                            
                            // Still store the engine for offline use
                            *sync_engine_clone.lock().unwrap() = Some(Arc::new(engine));
                            
                            // Don't retry immediately on start failure - this typically means protocol issues
                            break;
                        }
                    }
                }
                Err(e) => {
                    let now = Instant::now();
                    {
                        let mut app_state = state_clone.lock().unwrap();
                        app_state.sync_status.connected = false;
                        app_state.sync_status.connection_state = format!("Offline (retry in {}s)", retry_interval.as_secs());
                        app_state.sync_status.last_attempt = Some(now);
                        app_state.sync_status.next_retry = Some(now + retry_interval);
                        app_state.add_activity(format!("Connection attempt failed: {}", e), ActivityType::Error);
                    }
                    
                    // Wait before retrying
                    sleep(retry_interval).await;
                    
                    // Exponential backoff with cap
                    retry_interval = std::cmp::min(retry_interval * 2, max_retry_interval);
                    
                    // Show retry attempt
                    {
                        let mut app_state = state_clone.lock().unwrap();
                        app_state.add_activity(format!("Retrying connection to {}", server_url), ActivityType::SyncStarted);
                        app_state.sync_status.connection_state = "Connecting...".to_string();
                    }
                    
                    // Continue the loop to retry
                    continue;
                }
            }
        }
    });

    // Run app (UI will start immediately while connection happens in background)
    let res = run_app(&mut terminal, state.clone(), db.clone(), sync_engine, user_id).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{err:?}");
    }

    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    state: SharedState,
    db: Arc<ClientDatabase>,
    sync_engine: Arc<Mutex<Option<Arc<SyncEngine>>>>,
    user_id: Uuid,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    let tick_rate = Duration::from_millis(250);
    
    // Flag to track if callbacks are registered
    let mut callbacks_registered = false;
    
    loop {
        // Register callbacks on main thread if we have an engine but haven't registered yet
        if !callbacks_registered {
            if let Some(engine) = sync_engine.lock().unwrap().as_ref() {
                let events = engine.event_dispatcher();
                let state_cb = state.clone();
                
                let _ = events.register_rust_callback(
                    Box::new(move |event_type, document_id, title, _content, error, numeric_data, boolean_data, _context| {
                        // Use panic protection in the callback
                        let result = std::panic::catch_unwind(|| {
                            let mut app_state = state_cb.lock().unwrap();
                        
                            match event_type {
                                sync_client::events::EventType::DocumentCreated => {
                                    app_state.add_activity(
                                        format!("Task created: {}", title.unwrap_or("Untitled")),
                                        ActivityType::Created,
                                    );
                                    app_state.needs_refresh = true;
                                }
                                sync_client::events::EventType::DocumentUpdated => {
                                    app_state.add_activity(
                                        format!("Task updated: {}", title.unwrap_or("Untitled")),
                                        ActivityType::Updated,
                                    );
                                    app_state.needs_refresh = true;
                                }
                                sync_client::events::EventType::DocumentDeleted => {
                                    app_state.add_activity(
                                        format!("Task deleted: {}", document_id.map(|id| {
                                            if id.len() >= 8 { &id[..8] } else { id }
                                        }).unwrap_or("unknown")),
                                        ActivityType::Deleted,
                                    );
                                    app_state.needs_refresh = true;
                                }
                                sync_client::events::EventType::SyncStarted => {
                                    app_state.add_activity("Sync started".to_string(), ActivityType::SyncStarted);
                                }
                                sync_client::events::EventType::SyncCompleted => {
                                    app_state.add_activity(
                                        format!("Sync completed ({} docs)", numeric_data),
                                        ActivityType::SyncCompleted,
                                    );
                                    app_state.last_sync = Some(Instant::now());
                                    app_state.needs_refresh = true;
                                }
                                sync_client::events::EventType::ConnectionStateChanged => {
                                    if boolean_data {
                                        app_state.add_activity("Connected to server".to_string(), ActivityType::Connected);
                                        app_state.sync_status.connected = true;
                                        app_state.sync_status.connection_state = "Connected".to_string();
                                    } else {
                                        app_state.add_activity("Disconnected from server".to_string(), ActivityType::Disconnected);
                                        app_state.sync_status.connected = false;
                                        app_state.sync_status.connection_state = "Disconnected".to_string();
                                    }
                                }
                                sync_client::events::EventType::SyncError => {
                                    app_state.add_activity(
                                        format!("Sync error: {}", error.unwrap_or("unknown")),
                                        ActivityType::Error,
                                    );
                                }
                                sync_client::events::EventType::ConnectionAttempted => {
                                    app_state.add_activity(
                                        format!("Connecting to {}", title.unwrap_or("server")),
                                        ActivityType::SyncStarted,
                                    );
                                    app_state.sync_status.connection_state = "Connecting...".to_string();
                                }
                                sync_client::events::EventType::ConnectionSucceeded => {
                                    app_state.add_activity(
                                        format!("Connected to {}", title.unwrap_or("server")),
                                        ActivityType::Connected,
                                    );
                                    app_state.sync_status.connected = true;
                                    app_state.sync_status.connection_state = "Connected".to_string();
                                }
                                _ => {}
                            }
                        });
                        if result.is_err() {
                            eprintln!("Panic caught in event callback");
                        }
                    }),
                    std::ptr::null_mut(),
                    None,
                );
                callbacks_registered = true;
            }
        }
        
        // Process sync events FIRST
        if let Some(engine) = sync_engine.lock().unwrap().as_ref() {
            let dispatcher = engine.event_dispatcher();
            match dispatcher.process_events() {
                Ok(count) if count > 0 => {
                    // Events were processed - this might set needs_refresh
                }
                Err(e) => eprintln!("Error processing events: {:?}", e),
                _ => {}
            }
        }

        // Check if we should refresh tasks
        {
            let should_refresh = {
                let mut app_state = state.lock().unwrap();
                if app_state.needs_refresh {
                    app_state.needs_refresh = false;
                    true
                } else {
                    false
                }
            };
            
            if should_refresh {
                let _ = load_tasks(&db, user_id, state.clone()).await;
                update_sync_status(&db, state.clone()).await;
                
                // Mark refresh time for visual feedback
                {
                    let mut app_state = state.lock().unwrap();
                    app_state.last_refresh = Some(Instant::now());
                }
            }
        }

        // Draw UI once per loop iteration
        terminal.draw(|f| ui(f, &state))?;

        // Handle input with reasonable timeout 
        let timeout = Duration::from_millis(100);
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key_event(key.code, state.clone(), db.clone(), sync_engine.clone(), user_id).await;
                }
            }
        }

        // Check if we should quit
        if state.lock().unwrap().should_quit {
            return Ok(());
        }

        // Update tick for time tracking
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
}

fn ui(f: &mut Frame, state: &SharedState) {
    let app_state = state.lock().unwrap();
    
    // Main layout - title bar and content
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title bar
            Constraint::Min(0),     // Content
        ])
        .split(f.size());

    // Title bar with refresh indicator
    let title = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::Cyan));
    
    let refresh_indicator = if let Some(last_refresh) = app_state.last_refresh {
        let elapsed = last_refresh.elapsed();
        if elapsed.as_secs() < 2 {
            " ●" // Show indicator for 2 seconds after refresh
        } else {
            ""
        }
    } else {
        ""
    };
    
    let title_text = format!("Task Manager [{}]{}", app_state.database_name, refresh_indicator);
    
    let title_paragraph = Paragraph::new(title_text)
        .block(title)
        .alignment(Alignment::Center);
    f.render_widget(title_paragraph, main_chunks[0]);

    // Content area - 2x2 grid
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_chunks[1]);

    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(67), Constraint::Percentage(33)])
        .split(content_chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(67), Constraint::Percentage(33)])
        .split(content_chunks[1]);

    // Top-left: Task list
    render_task_list(f, left_chunks[0], &app_state);

    // Top-right: Task details
    render_task_details(f, right_chunks[0], &app_state);

    // Bottom-left: Sync status
    render_sync_status(f, left_chunks[1], &app_state);

    // Bottom-right: Activity log
    render_activity_log(f, right_chunks[1], &app_state);
}

fn render_task_list(f: &mut Frame, area: Rect, app_state: &AppState) {
    let pending_count = app_state.tasks.iter().filter(|t| t.status != "completed").count();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Tasks ({} total, {} pending)", app_state.tasks.len(), pending_count));

    let items: Vec<ListItem> = app_state
        .tasks
        .iter()
        .enumerate()
        .map(|(idx, task)| {
            let status_icon = match task.status.as_str() {
                "completed" => "✅",
                "in_progress" => "🔄",
                "pending" => "⏳",
                _ => "❓",
            };
            
            let priority_icon = match task.priority.as_str() {
                "high" => "🔴",
                "medium" => "🟡",
                "low" => "🟢",
                _ => "⚪",
            };
            
            let sync_icon = match task.sync_status.as_deref() {
                Some("pending") => " 📤",
                Some("conflict") => " ⚠️",
                _ => "",
            };
            
            let content = format!("{} {} {}{}", status_icon, priority_icon, task.title, sync_icon);
            let style = if idx == app_state.selected_task {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut list_state = ListState::default();
    list_state.select(Some(app_state.selected_task));

    f.render_stateful_widget(list, area, &mut list_state);

    // Render help text at bottom
    let help_text = vec![
        Line::from(vec![
            Span::raw("[j/k: navigate] [space: toggle]"),
        ]),
        Line::from(vec![
            Span::raw("[n: new] [d: delete] [q: quit]"),
        ]),
        Line::from(vec![
            Span::raw("Use --auto for concurrent testing"),
        ]),
    ];
    
    let help_area = Rect {
        x: area.x + 1,
        y: area.y + area.height - 4,
        width: area.width - 2,
        height: 3,
    };
    
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(help, help_area);
}

fn render_task_details(f: &mut Frame, area: Rect, app_state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Task Details");

    if let Some(task) = app_state.get_selected_task() {
        let status_display = match task.status.as_str() {
            "completed" => "✅ Completed",
            "in_progress" => "🔄 In Progress",
            "pending" => "⏳ Pending",
            _ => &task.status,
        };
        
        let priority_display = match task.priority.as_str() {
            "high" => "🔴 High",
            "medium" => "🟡 Medium",
            "low" => "🟢 Low",
            _ => &task.priority,
        };

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Title: ", Style::default().fg(Color::Gray)),
                Span::styled(&task.title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Gray)),
                Span::raw(status_display),
            ]),
            Line::from(vec![
                Span::styled("Priority: ", Style::default().fg(Color::Gray)),
                Span::raw(priority_display),
            ]),
            Line::from(""),
        ];

        if !task.description.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Description:", Style::default().fg(Color::Gray)),
            ]));
            lines.push(Line::from(task.description.clone()));
            lines.push(Line::from(""));
        }

        if !task.tags.is_empty() {
            let tags_str = task.tags.iter().map(|t| format!("#{}", t)).collect::<Vec<_>>().join(" ");
            lines.push(Line::from(vec![
                Span::styled("Tags: ", Style::default().fg(Color::Gray)),
                Span::styled(tags_str, Style::default().fg(Color::Cyan)),
            ]));
            lines.push(Line::from(""));
        }

        lines.push(Line::from(vec![
            Span::styled("Created: ", Style::default().fg(Color::Gray)),
            Span::raw(task.created_at.format("%Y-%m-%d %H:%M").to_string()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Updated: ", Style::default().fg(Color::Gray)),
            Span::raw(task.updated_at.format("%Y-%m-%d %H:%M").to_string()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Version: ", Style::default().fg(Color::Gray)),
            Span::styled(task.version.to_string(), Style::default().fg(Color::Yellow)),
        ]));

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
    } else {
        let paragraph = Paragraph::new("No task selected")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, area);
    }
}

fn render_sync_status(f: &mut Frame, area: Rect, app_state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Sync Status");

    let connection_status = {
        let mut status_text = app_state.sync_status.connection_state.clone();
        
        // Show live countdown if we're waiting to retry
        if let Some(next_retry) = app_state.sync_status.next_retry {
            if !app_state.sync_status.connected {
                let now = Instant::now();
                if next_retry > now {
                    let seconds_left = (next_retry - now).as_secs();
                    if seconds_left > 0 {
                        status_text = format!("Offline (retry in {}s)", seconds_left);
                    }
                }
            }
        }
        
        Line::from(vec![
            Span::raw(if app_state.sync_status.connected { "🔗 " } else { "📡 " }),
            Span::styled(
                status_text,
                Style::default().fg(if app_state.sync_status.connected { Color::Green } else { Color::Yellow }),
            ),
        ])
    };

    let sync_info = if app_state.sync_status.pending_count > 0 || app_state.sync_status.conflict_count > 0 {
        if app_state.sync_status.conflict_count > 0 {
            Line::from(vec![
                Span::raw("⚠️  "),
                Span::styled(
                    format!("{} conflicts", app_state.sync_status.conflict_count),
                    Style::default().fg(Color::Red),
                ),
            ])
        } else {
            Line::from(vec![
                Span::raw("📤 "),
                Span::styled(
                    format!("{} pending sync", app_state.sync_status.pending_count),
                    Style::default().fg(Color::Yellow),
                ),
            ])
        }
    } else {
        Line::from(vec![
            Span::raw("✅ "),
            Span::raw("All changes synced"),
        ])
    };

    let last_sync_line = if let Some(last_sync) = app_state.last_sync {
        let elapsed = last_sync.elapsed();
        let time_str = if elapsed.as_secs() < 60 {
            format!("{} seconds ago", elapsed.as_secs())
        } else if elapsed.as_secs() < 3600 {
            format!("{} minutes ago", elapsed.as_secs() / 60)
        } else {
            format!("{} hours ago", elapsed.as_secs() / 3600)
        };
        Line::from(vec![
            Span::raw("📊 Last sync: "),
            Span::raw(time_str),
        ])
    } else {
        Line::from(vec![
            Span::raw("📊 Last sync: "),
            Span::styled("Never", Style::default().fg(Color::DarkGray)),
        ])
    };

    let mut lines = vec![
        connection_status,
        sync_info,
        last_sync_line,
    ];
    
    // Add retry info if we're offline and have attempted connections
    if !app_state.sync_status.connected && app_state.sync_status.last_attempt.is_some() {
        lines.push(Line::from(vec![
            Span::styled("Auto-retrying in background", Style::default().fg(Color::DarkGray)),
        ]));
    }
    
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("[n: new task] [q: quit]", Style::default().fg(Color::DarkGray)),
    ]));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

fn render_activity_log(f: &mut Frame, area: Rect, app_state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Activity Log");

    let items: Vec<Line> = app_state
        .activity_log
        .iter()
        .rev()
        .map(|entry| {
            let icon = match entry.event_type {
                ActivityType::Created => "➕",
                ActivityType::Updated => "📝",
                ActivityType::Deleted => "🗑️",
                ActivityType::SyncStarted => "🔄",
                ActivityType::SyncCompleted => "✅",
                ActivityType::Connected => "🔗",
                ActivityType::Disconnected => "❌",
                ActivityType::Error => "🚨",
            };
            
            let time_str = entry.timestamp.format("%H:%M").to_string();
            
            Line::from(vec![
                Span::styled(time_str, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::raw(icon),
                Span::raw(" "),
                Span::raw(&entry.message),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(items).block(block);
    f.render_widget(paragraph, area);
}

async fn handle_key_event(
    key: KeyCode,
    state: SharedState,
    db: Arc<ClientDatabase>,
    sync_engine: Arc<Mutex<Option<Arc<SyncEngine>>>>,
    user_id: Uuid,
) {
    match key {
        KeyCode::Char('q') => {
            state.lock().unwrap().should_quit = true;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            state.lock().unwrap().move_selection_down();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.lock().unwrap().move_selection_up();
        }
        KeyCode::Char(' ') => {
            // Toggle task completion
            let task_id = {
                let app_state = state.lock().unwrap();
                app_state.get_selected_task().map(|t| t.id)
            };
            
            if let Some(id) = task_id {
                toggle_task_completion(&db, &sync_engine, id, state.clone()).await;
            }
        }
        // Removed manual refresh - auto-refresh via callbacks
        KeyCode::Char('n') => {
            // Create new task (simplified for this example)
            create_sample_task(&db, &sync_engine, user_id, state.clone()).await;
        }
        KeyCode::Char('d') => {
            // Delete selected task
            let task_id = {
                let app_state = state.lock().unwrap();
                app_state.get_selected_task().map(|t| t.id)
            };
            
            if let Some(id) = task_id {
                delete_task(&db, &sync_engine, id, state.clone()).await;
            }
        }
        _ => {}
    }
}

async fn load_tasks(
    db: &ClientDatabase,
    user_id: Uuid,
    state: SharedState,
) -> Result<(), Box<dyn Error>> {
    let rows = sqlx::query(
        r#"
        SELECT id, title, content, sync_status, created_at, updated_at, version
        FROM documents 
        WHERE user_id = ?1 AND deleted_at IS NULL
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id.to_string())
    .fetch_all(&db.pool)
    .await?;

    let mut tasks = Vec::new();
    for row in rows {
        let id = Uuid::parse_str(&row.try_get::<String, _>("id")?)?;
        let title = row.try_get::<String, _>("title")?;
        let content_str = row.try_get::<String, _>("content")?;
        let sync_status = row.try_get::<Option<String>, _>("sync_status")?;
        let created_at = row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")?;
        let updated_at = row.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at")?;
        let version = row.try_get::<i64, _>("version")?;
        
        let content: Value = serde_json::from_str(&content_str).unwrap_or_default();
        
        let task = Task {
            id,
            title: content.get("title").and_then(|v| v.as_str()).unwrap_or(&title).to_string(),
            description: content.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            status: content.get("status").and_then(|v| v.as_str()).unwrap_or("pending").to_string(),
            priority: content.get("priority").and_then(|v| v.as_str()).unwrap_or("medium").to_string(),
            tags: content.get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            created_at,
            updated_at,
            version,
            sync_status,
        };
        
        tasks.push(task);
    }

    let mut app_state = state.lock().unwrap();
    app_state.tasks = tasks;
    
    // Update sync status
    drop(app_state);
    update_sync_status(&db, state).await;
    
    Ok(())
}

async fn update_sync_status(db: &ClientDatabase, state: SharedState) {
    let pending = sqlx::query("SELECT COUNT(*) as count FROM documents WHERE sync_status = 'pending'")
        .fetch_one(&db.pool)
        .await
        .ok()
        .and_then(|row| row.try_get::<i64, _>("count").ok())
        .unwrap_or(0) as usize;
        
    let conflicts = sqlx::query("SELECT COUNT(*) as count FROM documents WHERE sync_status = 'conflict'")
        .fetch_one(&db.pool)
        .await
        .ok()
        .and_then(|row| row.try_get::<i64, _>("count").ok())
        .unwrap_or(0) as usize;
    
    let mut app_state = state.lock().unwrap();
    app_state.sync_status.pending_count = pending;
    app_state.sync_status.conflict_count = conflicts;
}

async fn toggle_task_completion(
    db: &ClientDatabase,
    sync_engine: &Arc<Mutex<Option<Arc<SyncEngine>>>>,
    task_id: Uuid,
    state: SharedState,
) {
    let doc = match db.get_document(&task_id).await {
        Ok(doc) => doc,
        Err(_) => return,
    };
    
    let mut content = doc.content.clone();
    if let Some(obj) = content.as_object_mut() {
        let current_status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
        let new_status = if current_status == "completed" { "pending" } else { "completed" };
        obj.insert("status".to_string(), json!(new_status));
        
        if new_status == "completed" {
            obj.insert("completed_at".to_string(), json!(chrono::Utc::now().to_rfc3339()));
        } else {
            obj.remove("completed_at");
        }
    }
    
    if let Some(engine) = sync_engine.lock().unwrap().as_ref() {
        let _ = engine.update_document(task_id, content).await;
    } else {
        // Offline update
        let mut updated_doc = doc;
        updated_doc.revision_id = updated_doc.next_revision(&content);
        updated_doc.content = content;
        updated_doc.version += 1;
        updated_doc.updated_at = chrono::Utc::now();
        
        let _ = db.save_document(&updated_doc).await;
        
        // Mark as pending sync
        let _ = sqlx::query("UPDATE documents SET sync_status = 'pending' WHERE id = ?1")
            .bind(task_id.to_string())
            .execute(&db.pool)
            .await;
    }
    
    // Trigger UI refresh
    {
        let mut app_state = state.lock().unwrap();
        app_state.needs_refresh = true;
    }
    
    // Also reload tasks immediately for responsive UI
    if let Ok(user_id) = db.get_user_id().await {
        let _ = load_tasks(db, user_id, state).await;
    }
}

async fn create_sample_task(
    db: &ClientDatabase,
    sync_engine: &Arc<Mutex<Option<Arc<SyncEngine>>>>,
    user_id: Uuid,
    state: SharedState,
) {
    let title = format!("New Task {}", chrono::Utc::now().format("%H:%M:%S"));
    let content = json!({
        "title": title.clone(),
        "description": "Created from task list UI",
        "status": "pending",
        "priority": "medium",
        "tags": vec!["ui", "demo"],
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    
    if let Some(engine) = sync_engine.lock().unwrap().as_ref() {
        let _ = engine.create_document(title, content).await;
    } else {
        // Offline create
        let doc = Document {
            id: Uuid::new_v4(),
            user_id,
            title,
            revision_id: Document::initial_revision(&content),
            content,
            version: 1,
            vector_clock: sync_core::models::VectorClock::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            deleted_at: None,
        };
        
        let _ = db.save_document(&doc).await;
    }
    
    // Activity will be logged via event callback if online, or we can add it manually if offline
    if sync_engine.lock().unwrap().is_none() {
        let mut app_state = state.lock().unwrap();
        app_state.add_activity("Task created (offline)".to_string(), ActivityType::Created);
    }
    
    // Trigger UI refresh for immediate feedback
    {
        let mut app_state = state.lock().unwrap();
        app_state.needs_refresh = true;
    }
}

async fn delete_task(
    db: &ClientDatabase,
    sync_engine: &Arc<Mutex<Option<Arc<SyncEngine>>>>,
    task_id: Uuid,
    state: SharedState,
) {
    if let Some(engine) = sync_engine.lock().unwrap().as_ref() {
        // Use sync engine if available
        let _ = engine.delete_document(task_id).await;
        
        let mut app_state = state.lock().unwrap();
        app_state.add_activity("Task deleted".to_string(), ActivityType::Deleted);
    } else {
        // Offline delete
        let _ = db.delete_document(&task_id).await;
        
        let mut app_state = state.lock().unwrap();
        app_state.add_activity("Task deleted (offline)".to_string(), ActivityType::Deleted);
    }
    
    // Trigger UI refresh for immediate feedback
    {
        let mut app_state = state.lock().unwrap();
        app_state.needs_refresh = true;
    }
}

async fn setup_user(
    db: &ClientDatabase,
    user_id: Uuid,
    client_id: Uuid,
    server_url: &str,
    token: &str,
) -> Result<(), Box<dyn Error>> {
    sqlx::query(
        "INSERT INTO user_config (user_id, client_id, server_url, auth_token) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(user_id.to_string())
    .bind(client_id.to_string())
    .bind(server_url)
    .bind(token)
    .execute(&db.pool)
    .await?;
    Ok(())
}