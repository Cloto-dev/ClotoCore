//! End-to-end checks of what the kernel does with an agent's long-term memory
//! around one message: whether it stores the turn (agent metadata
//! `memory_store`), and how recalled text is labelled when it reaches the engine
//! (docs/CONVERSATIONS_DESIGN.md §2c).
//!
//! The memory and the engine are native test plugins that count and record, so a
//! gate that exists only in a helper — and not on the path a real message takes —
//! does not pass here.

use async_trait::async_trait;
use cloto_core::handlers::system::SystemHandler;
use cloto_core::managers::{AgentManager, PluginRegistry};
use cloto_shared::{
    AgentMetadata, ClotoId, ClotoMessage, MemoryProvider, MessageSource, Permission, Plugin,
    PluginCast, PluginManifest, ReasoningEngine, ServiceType,
};
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

const ENGINE: &str = "engine.recorder";
const MEMORY: &str = "memory.counter";

fn manifest(id: &str) -> PluginManifest {
    PluginManifest {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        version: "1.0".to_string(),
        category: cloto_shared::PluginCategory::Tool,
        service_type: ServiceType::Skill,
        tags: vec![],
        is_active: true,
        is_configured: true,
        required_config_keys: vec![],
        action_icon: None,
        action_target: None,
        icon_data: None,
        magic_seal: 0x5645_5253,
        sdk_version: "1.0".to_string(),
        required_permissions: vec![],
        provided_capabilities: vec![],
        provided_tools: vec![],
    }
}

/// Answers every message with "ok" and keeps the context it was handed.
struct RecordingEngine {
    seen: Arc<Mutex<Vec<Vec<ClotoMessage>>>>,
}

impl PluginCast for RecordingEngine {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_reasoning(&self) -> Option<&dyn ReasoningEngine> {
        Some(self)
    }
}

#[async_trait]
impl Plugin for RecordingEngine {
    fn manifest(&self) -> PluginManifest {
        manifest(ENGINE)
    }
}

#[async_trait]
impl ReasoningEngine for RecordingEngine {
    fn name(&self) -> &str {
        ENGINE
    }
    async fn think(
        &self,
        _agent: &AgentMetadata,
        _message: &ClotoMessage,
        context: Vec<ClotoMessage>,
    ) -> anyhow::Result<String> {
        self.seen.lock().unwrap().push(context);
        Ok("ok".into())
    }
}

/// Counts stores and recalls; every recall returns one old message that is not
/// part of any conversation the test sends.
struct CountingMemory {
    stores: Arc<AtomicUsize>,
    recalls: Arc<AtomicUsize>,
}

impl PluginCast for CountingMemory {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_memory(&self) -> Option<&dyn MemoryProvider> {
        Some(self)
    }
}

#[async_trait]
impl Plugin for CountingMemory {
    fn manifest(&self) -> PluginManifest {
        manifest(MEMORY)
    }
}

#[async_trait]
impl MemoryProvider for CountingMemory {
    fn name(&self) -> &str {
        MEMORY
    }
    async fn store(&self, _agent_id: String, _message: ClotoMessage) -> anyhow::Result<()> {
        self.stores.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn recall(
        &self,
        _agent_id: String,
        _query: &str,
        _limit: usize,
    ) -> anyhow::Result<Vec<ClotoMessage>> {
        self.recalls.fetch_add(1, Ordering::SeqCst);
        let mut old = ClotoMessage::new(
            MessageSource::User {
                id: "user1".into(),
                name: "User".into(),
            },
            "an instruction from last week, with last week's figures".into(),
        );
        old.id = "recalled-old".into();
        old.timestamp = chrono::Utc::now() - chrono::Duration::days(7);
        Ok(vec![old])
    }
}

struct Rig {
    handler: SystemHandler,
    stores: Arc<AtomicUsize>,
    recalls: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<Vec<ClotoMessage>>>>,
}

async fn rig(agent_metadata: &str) -> Rig {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    cloto_core::db::init_db(&pool, "sqlite::memory:", None)
        .await
        .unwrap();
    let agent_id = "agent.memory-test";
    sqlx::query("INSERT INTO agents (id, name, description, status, default_engine_id, required_capabilities, metadata, enabled) VALUES (?, 'Memory Test', 'Desc', 'online', ?, '[\"Reasoning\", \"Memory\"]', ?, 1)")
        .bind(agent_id)
        .bind(ENGINE)
        .bind(agent_metadata)
        .execute(&pool)
        .await
        .unwrap();

    let registry = Arc::new(PluginRegistry::new(
        5,
        10,
        50,
        cloto_core::test_utils::test_mcp_manager().await,
    ));
    let stores = Arc::new(AtomicUsize::new(0));
    let recalls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    {
        let mut state = registry.state.write().await;
        state.plugins.insert(
            ENGINE.into(),
            Arc::new(RecordingEngine { seen: seen.clone() }),
        );
        state.plugins.insert(
            MEMORY.into(),
            Arc::new(CountingMemory {
                stores: stores.clone(),
                recalls: recalls.clone(),
            }),
        );
        state.effective_permissions.insert(
            ClotoId::from_name(MEMORY),
            vec![Permission::MemoryRead, Permission::MemoryWrite],
        );
    }

    let agent_manager = AgentManager::new(pool.clone(), 90_000);
    let (event_tx, _event_rx) = mpsc::channel(64);
    let handler = SystemHandler::new(
        registry,
        agent_manager,
        agent_id.to_string(),
        event_tx,
        10, // memory_context_limit
        Arc::new(cloto_core::managers::SystemMetrics::new()),
        vec![],
        "consensus:".to_string(),
        16,
        30,
        Arc::new(dashmap::DashMap::new()),
        Arc::new(dashmap::DashMap::new()),
        pool,
        Arc::new(dashmap::DashMap::new()),
        5,
        false,
    );
    Rig {
        handler,
        stores,
        recalls,
        seen,
    }
}

fn user_message() -> ClotoMessage {
    let mut m = ClotoMessage::new(
        MessageSource::User {
            id: "user1".into(),
            name: "User".into(),
        },
        "what changed today?".into(),
    );
    m.target_agent = Some("agent.memory-test".into());
    m
}

/// Stores are fire-and-forget background tasks, so wait until the count settles
/// instead of reading it the instant the handler returns.
async fn settled(counter: &AtomicUsize) -> usize {
    let mut last = counter.load(Ordering::SeqCst);
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let now = counter.load(Ordering::SeqCst);
        if now == last && now > 0 {
            break;
        }
        last = now;
    }
    counter.load(Ordering::SeqCst)
}

#[tokio::test]
async fn by_default_the_kernel_stores_the_turn() {
    // The control: without it, a zero below could mean the rig never reaches the
    // store path at all.
    let r = rig(&format!(r#"{{"preferred_memory":"{MEMORY}"}}"#)).await;
    r.handler.handle_message(user_message()).await.unwrap();
    assert!(
        settled(&r.stores).await >= 1,
        "the default must keep storing the message and the reply"
    );
    assert!(r.recalls.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn memory_store_off_stores_nothing_and_still_recalls() {
    let r = rig(&format!(
        r#"{{"preferred_memory":"{MEMORY}","memory_store":"off"}}"#
    ))
    .await;
    r.handler.handle_message(user_message()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        r.stores.load(Ordering::SeqCst),
        0,
        "memory_store=off must stop every automatic store, the message and the reply"
    );
    assert!(
        r.recalls.load(Ordering::SeqCst) >= 1,
        "memory_store only governs writing; recall is recall_policy's to decide"
    );
}

#[tokio::test]
async fn recalled_text_reaches_the_engine_labelled_memory() {
    let r = rig(&format!(r#"{{"preferred_memory":"{MEMORY}"}}"#)).await;
    r.handler.handle_message(user_message()).await.unwrap();
    let seen = r.seen.lock().unwrap();
    let context = seen.first().expect("the engine was called");
    let recalled = context
        .iter()
        .find(|m| m.id == "recalled-old")
        .expect("the recalled message is in the context");
    assert_eq!(
        recalled
            .metadata
            .get(cloto_core::conversation_context::CONTEXT_TYPE_KEY)
            .map(String::as_str),
        Some(cloto_core::conversation_context::CONTEXT_MEMORY),
        "recalled text must not reach the engine unlabelled, where it reads as a turn of this conversation"
    );
}
