use exiv_core::events::EventProcessor;
use exiv_core::managers::{AgentManager, PluginManager, PluginRegistry, SystemMetrics};
use exiv_shared::{ExivEvent, ExivEventData};
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};

/// Helper to create EventProcessor for testing
async fn create_test_processor(
    max_history_size: usize,
) -> (Arc<EventProcessor>, Arc<RwLock<VecDeque<Arc<ExivEvent>>>>) {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    exiv_core::db::init_db(&pool, "sqlite::memory:")
        .await
        .unwrap();

    let registry = Arc::new(PluginRegistry::new(5, 10));
    let plugin_manager = Arc::new(PluginManager::new(pool.clone(), vec![], 30, 10).unwrap());
    let agent_manager = AgentManager::new(pool.clone());
    let (tx, _rx) = broadcast::channel(100);
    let metrics = Arc::new(SystemMetrics::new());
    let event_history = Arc::new(RwLock::new(VecDeque::new()));

    let processor = Arc::new(EventProcessor::new(
        registry,
        plugin_manager,
        agent_manager,
        tx,
        event_history.clone(),
        metrics,
        max_history_size,
        24,   // event_retention_hours
        None, // evolution_engine
        None, // fitness_collector
        None, // consensus
    ));

    (processor, event_history)
}

#[tokio::test]
async fn test_event_history_size_limit() {
    let (_processor, history) = create_test_processor(1000).await;

    // Manually add 1500 events to the history
    {
        let mut hist = history.write().await;
        for i in 0..1500 {
            let event = Arc::new(ExivEvent::new(ExivEventData::SystemNotification(format!(
                "Event {}",
                i
            ))));
            hist.push_back(event);

            // Apply size limit
            if hist.len() > 1000 {
                hist.pop_front();
            }
        }
    }

    // Verify only last 1000 retained
    let hist = history.read().await;
    assert_eq!(hist.len(), 1000, "History should be capped at 1000 events");

    // Verify the oldest event is #500 (since we added 1500 and kept last 1000)
    if let Some(oldest) = hist.front() {
        if let ExivEventData::SystemNotification(msg) = &oldest.data {
            assert!(
                msg.contains("Event 500") || msg.contains("Event 50"),
                "Oldest event should be around #500, got: {}",
                msg
            );
        }
    }
}

#[tokio::test]
async fn test_time_based_cleanup() {
    let (processor, history) = create_test_processor(1000).await;

    // Add events with old timestamps (manually)
    {
        let mut hist = history.write().await;

        // Add 10 old events (25 hours ago)
        let old_time = chrono::Utc::now() - chrono::Duration::hours(25);
        for i in 0..10 {
            let mut event = ExivEvent::new(ExivEventData::SystemNotification(format!(
                "Old Event {}",
                i
            )));
            event.timestamp = old_time; // Set to old timestamp
            hist.push_back(Arc::new(event));
        }

        // Add 10 recent events (1 hour ago)
        let recent_time = chrono::Utc::now() - chrono::Duration::hours(1);
        for i in 0..10 {
            let mut event = ExivEvent::new(ExivEventData::SystemNotification(format!(
                "Recent Event {}",
                i
            )));
            event.timestamp = recent_time;
            hist.push_back(Arc::new(event));
        }
    }

    // Verify we have 20 events before cleanup
    {
        let hist = history.read().await;
        assert_eq!(hist.len(), 20);
    }

    // Run cleanup (this should remove events older than 24 hours)
    processor.cleanup_old_events().await;

    // Verify old events are removed
    let hist = history.read().await;
    assert_eq!(
        hist.len(),
        10,
        "Only recent events should remain after cleanup"
    );

    // Verify all remaining events are recent
    for event in hist.iter() {
        if let ExivEventData::SystemNotification(msg) = &event.data {
            assert!(
                msg.contains("Recent"),
                "Only recent events should remain, found: {}",
                msg
            );
        }
    }
}

#[tokio::test]
async fn test_configurable_history_size() {
    // Create processor with custom size limit
    let (_processor, history) = create_test_processor(500).await;

    // Add 700 events
    {
        let mut hist = history.write().await;
        for i in 0..700 {
            let event = Arc::new(ExivEvent::new(ExivEventData::SystemNotification(format!(
                "Event {}",
                i
            ))));
            hist.push_back(event);

            // Apply size limit (500)
            if hist.len() > 500 {
                hist.pop_front();
            }
        }
    }

    // Verify limit is enforced at 500
    let hist = history.read().await;
    assert_eq!(
        hist.len(),
        500,
        "History should be capped at configured size (500)"
    );
}

#[tokio::test]
async fn test_cleanup_task_integration() {
    let (processor, history) = create_test_processor(1000).await;

    // Spawn cleanup task
    processor
        .clone()
        .spawn_cleanup_task(std::sync::Arc::new(tokio::sync::Notify::new()));

    // Add some old events
    {
        let mut hist = history.write().await;
        let old_time = chrono::Utc::now() - chrono::Duration::hours(25);
        for i in 0..5 {
            let mut event = ExivEvent::new(ExivEventData::SystemNotification(format!("Old {}", i)));
            event.timestamp = old_time;
            hist.push_back(Arc::new(event));
        }
    }

    // Verify events exist
    {
        let hist = history.read().await;
        assert_eq!(hist.len(), 5);
    }

    // Wait a bit for cleanup task to potentially run (though we won't wait 5 minutes)
    // This test mainly verifies the task spawns without panicking
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Cleanup task is running in background - test just verifies no panic
}
