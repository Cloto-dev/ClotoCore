use std::sync::Arc;
use sqlx::SqlitePool;
use exiv_shared::{
    MemoryProvider, ExivMessage, MessageSource, Plugin, PluginRuntimeContext, PluginCast
};
use plugin_ks22::Ks22Plugin;
use exiv_core::db::SqliteDataStore;

#[tokio::test]
#[ignore = "Known bug: UUID-based ordering is not chronological"]
async fn test_memory_chronology_issue() {
    // 1. Setup DB
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE plugin_data (plugin_id TEXT, key TEXT, value TEXT, PRIMARY KEY(plugin_id, key))").execute(&pool).await.unwrap();
    
    let store = Arc::new(SqliteDataStore::new(pool));
    
    // 2. Initialize Plugin
    let plugin = Ks22Plugin::new_plugin(exiv_shared::PluginConfig {
        id: "core.ks22".to_string(),
        config_values: std::collections::HashMap::new(),
    }).await.unwrap();
    
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    plugin.on_plugin_init(PluginRuntimeContext {
        effective_permissions: vec![],
        store: store.clone(),
        event_tx: tx,
    }, None).await.unwrap();
    
    let memory: &dyn MemoryProvider = plugin.as_memory().expect("Should implement MemoryProvider");
    let agent_id = "agent.test".to_string();

    // 3. Store messages in a specific order with slight delays
    // UUID-based keys in current implementation will NOT be chronological.
    let mut messages = Vec::new();
    for i in 0..5 {
        let msg = ExivMessage::new(
            MessageSource::User { id: "u".into(), name: "N".into() },
            format!("Message {}", i)
        );
        messages.push(msg.clone());
        memory.store(agent_id.clone(), msg).await.unwrap();
        // No delay needed because UUIDs are random, but we want to ensure timestamps differ in real scenarios
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    // 4. Recall
    let recalled = memory.recall(agent_id.clone(), "", 5).await.unwrap();
    
    println!("Recalled order:");
    for m in &recalled {
        println!(" - {}", m.content);
    }

    // CURRENT FAIL: The messages will be in arbitrary order (whatever the UUID sorting gives).
    // EXPECTED: Message 0, 1, 2, 3, 4 (chronological order)
    
    assert_eq!(recalled.len(), 5);
    
    // If the bug is present, this assertion will likely fail because UUIDs are not chronological.
    // We expect the result to be chronological (Message 0 first, Message 4 last in the returned list).
    for (i, item) in recalled.iter().enumerate().take(5) {
        assert_eq!(item.content, format!("Message {}", i), "Order mismatch at index {}", i);
    }
}
