use crate::integration::helpers::TestContext;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

crate::integration_test!(
    test_simple_event_delivery,
    |ctx: TestContext| async move {
        if std::env::var("RUN_INTEGRATION_TESTS").is_err() {
            eprintln!("Skipping integration test. Set RUN_INTEGRATION_TESTS=1 to run.");
            return;
        }

        // Create a test user
        let email = "simple-event-test@example.com";

        // Generate proper HMAC credentials
        let (api_key, api_secret) = ctx
            .generate_test_credentials("test-simple-event")
            .await
            .expect("Failed to generate credentials");

        // Create user
        let user_id = ctx
            .create_test_user(email)
            .await
            .expect("Failed to create user");

        // Create first client
        tracing::info!("Creating client 1...");
        let client1 = ctx
            .create_test_client(email, user_id, &api_key, &api_secret)
            .await
            .expect("Failed to create client 1");

        // Give it time to fully connect
        sleep(Duration::from_millis(1000)).await;

        // Create second client
        tracing::info!("Creating client 2...");
        let client2 = ctx
            .create_test_client(email, user_id, &api_key, &api_secret)
            .await
            .expect("Failed to create client 2");

        // Give it time to fully connect
        sleep(Duration::from_millis(1000)).await;

        // Track events for client 2
        let events_received = Arc::new(Mutex::new(Vec::<String>::new()));
        let events_clone = events_received.clone();

        client2
            .event_dispatcher()
            .register_rust_callback(
                Box::new(
                    move |event_type,
                          document_id,
                          title,
                          _content,
                          _error,
                          _numeric_data,
                          _boolean_data,
                          _context| {
                        use sync_client::events::EventType;
                        let mut events = events_clone.lock().unwrap();

                        match event_type {
                            EventType::DocumentCreated => {
                                if let (Some(doc_id), Some(title_str)) = (document_id, title) {
                                    events.push(format!("created:{}:{}", doc_id, title_str));
                                    tracing::info!(
                                        "Client 2 received DocumentCreated event for {}",
                                        title_str
                                    );
                                }
                            }
                            _ => {}
                        }
                    },
                ),
                std::ptr::null_mut(),
                None,
            )
            .expect("Failed to register callback");

        // Process events a few times to ensure callback is ready
        for _ in 0..5 {
            let _ = client2.event_dispatcher().process_events();
            sleep(Duration::from_millis(100)).await;
        }

        // Create a document on client 1
        tracing::info!("Creating document on client 1...");
        let doc = client1
            .create_document(json!({ "title": "Test Document", "test": true }))
            .await
            .expect("Failed to create document");

        tracing::info!("Document created with ID: {}", doc.id);

        // Process events multiple times with delays
        for i in 0..20 {
            let _ = client1.event_dispatcher().process_events();
            let _ = client2.event_dispatcher().process_events();
            sleep(Duration::from_millis(200)).await;

            // Check if we received the event
            let events = events_received.lock().unwrap();
            if !events.is_empty() {
                tracing::info!("Events received after {} iterations: {:?}", i + 1, *events);
                assert_eq!(events.len(), 1);
                assert!(events[0].contains(&doc.id.to_string()));
                assert!(events[0].contains("Test Document"));
                return;
            }
        }

        // If we get here, the event was never received
        let events = events_received.lock().unwrap();
        panic!(
            "Client 2 never received the create event. Events: {:?}",
            *events
        );
    },
    true
);
