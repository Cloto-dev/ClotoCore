use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub use exiv_macros::exiv_plugin;
pub use inventory;

/// SDK version constant for consistent version reporting across all plugins
/// M-14: Plugins should reference this instead of their own CARGO_PKG_VERSION
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exivプラットフォーム内での一意の識別子（Agent, Plugin, Session等）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExivId(Uuid);

impl std::fmt::Display for ExivId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// L-02: Default generates a random UUID v4 (intentional design).
/// Each default ExivId is unique, suitable for trace IDs and ephemeral identifiers.
/// For deterministic IDs, use `ExivId::from_name()` instead.
impl Default for ExivId {
    fn default() -> Self {
        Self::new()
    }
}

impl ExivId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// トレース用のIDを生成
    pub fn new_trace_id() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_name(name: &str) -> Self {
        let namespace = Uuid::NAMESPACE_DNS;
        Self(Uuid::new_v5(&namespace, name.as_bytes()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CapabilityType {
    /// 思考・推論能力 (ReasoningEngine)
    Reasoning,
    /// 記憶・永続化能力 (MemoryProvider)
    Memory,
    /// 外部通信・入出力能力 (CommunicationAdapter)
    Communication,
    /// 特定タスク実行能力 (Tool)
    Tool,
    /// 視覚・画像処理能力
    Vision,
    /// 物理/ハードウェア操作能力
    HAL,
    /// Webサーバー拡張能力 (APIエンドポイント提供)
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    VisionRead,
    InputControl,
    FileRead,
    FileWrite,
    NetworkAccess,
    ProcessExecution,
    MemoryRead,
    MemoryWrite,
    AdminAccess,
}

impl std::fmt::Display for Permission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// M-13: Explicit serde tagging for consistent serialization
#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "type", content = "detail")]
pub enum ExivError {
    #[error("Permission denied: {0}")]
    PermissionDenied(Permission),
    #[error("Plugin error: {id} - {message}")]
    PluginError { id: String, message: String },
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Timeout occurred: {0}")]
    Timeout(String),
    #[error("Configuration error: {0}")]
    ConfigError(String),
    #[error("Plugin not found: {0}")]
    PluginNotFound(String),
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Validation error: {0}")]
    ValidationError(String),
}

pub type ExivResult<T> = std::result::Result<T, ExivError>;

/// Kernelによって認可され、実行時に提供されるプラグインの権限・環境情報
#[derive(Clone)]
pub struct PluginRuntimeContext {
    pub effective_permissions: Vec<Permission>,
    pub store: Arc<dyn PluginDataStore>,
    pub event_tx: tokio::sync::mpsc::Sender<ExivEventData>,
}

/// プラグインがデータを保存するための抽象ストレージインターフェース (Principle #4: Data Sovereignty / Principle #6: SAL)
#[async_trait]
pub trait PluginDataStore: Send + Sync {
    /// JSON形式でデータを保存
    async fn set_json(&self, plugin_id: &str, key: &str, value: serde_json::Value) -> anyhow::Result<()>;
    /// JSON形式でデータを取得
    async fn get_json(&self, plugin_id: &str, key: &str) -> anyhow::Result<Option<serde_json::Value>>;
    /// 指定されたプレフィックスを持つ全てのデータを取得
    async fn get_all_json(&self, plugin_id: &str, key_prefix: &str) -> anyhow::Result<Vec<(String, serde_json::Value)>>;
}

/// SALを型安全に利用するための拡張トレイト
#[async_trait]
pub trait SALExt: PluginDataStore {
    async fn save<T: Serialize + Sync>(&self, plugin_id: &str, key: &str, value: &T) -> anyhow::Result<()> {
        self.set_json(plugin_id, key, serde_json::to_value(value)?).await
    }

    async fn load<T: for<'de> Deserialize<'de>>(&self, plugin_id: &str, key: &str) -> anyhow::Result<Option<T>> {
        if let Some(json) = self.get_json(plugin_id, key).await? {
            Ok(Some(serde_json::from_value(json)?))
        } else {
            Ok(None)
        }
    }

    /// 時系列順にソート可能なメモリキーを生成する (Principle #4 / Guardrail #4)
    /// 形式: mem:{agent_id}:{timestamp_nanos_padded}:{message_id}
    fn generate_mem_key(&self, agent_id: &str, message: &ExivMessage) -> String {
        let ts = message.timestamp.timestamp_nanos_opt().unwrap_or(0);
        format!("mem:{}:{:020}:{}", agent_id, ts, message.id)
    }
}

impl<T: PluginDataStore + ?Sized> SALExt for T {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[async_trait]
pub trait NetworkCapability: Send + Sync {
    async fn send_http_request(&self, request: HttpRequest) -> anyhow::Result<HttpResponse>;
}

/// 実行時に注入される具体的な能力のラッパー
#[derive(Clone)]
pub enum PluginCapability {
    Network(Arc<dyn NetworkCapability>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    Communication,
    Reasoning,
    Skill,
    Vision,
    Action,
    Memory,
    HAL,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginCategory {
    Agent,  // 対話可能な人格 (#MIND)
    Tool,   // 機能・道具 (#TOOL)
    Memory, // 記憶 (#MEMORY)
    System, // システム内部 (#SYSTEM)
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub category: PluginCategory, // 追加
    pub service_type: ServiceType,
    pub tags: Vec<String>,
    pub is_active: bool,
    pub is_configured: bool,
    pub required_config_keys: Vec<String>,
    pub action_icon: Option<String>,
    pub action_target: Option<String>,
    pub icon_data: Option<String>,
    pub magic_seal: u32,
    pub sdk_version: String,
    pub required_permissions: Vec<Permission>,
    pub provided_capabilities: Vec<CapabilityType>,
    pub provided_tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageSource {
    User { id: String, name: String },
    Agent { id: String },
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExivMessage {
    pub id: String,
    pub source: MessageSource,
    pub target_agent: Option<String>,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

impl ExivMessage {
    pub fn new(source: MessageSource, content: String) -> Self {
        Self {
            id: ExivId::new().to_string(),
            source,
            target_agent: None,
            content,
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HandAction {
    MouseMove { x: i32, y: i32 },
    MouseClick { button: String },
    KeyPress { key: String },
    Wait { ms: u32 },
    // New Actions for Vision-HAL Coordination
    CaptureScreen, 
    ClickElement { label: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorVisionData {
    pub captured_at: DateTime<Utc>,
    pub detected_elements: Vec<DetectedElement>,
    pub image_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedElement {
    pub label: String,
    pub bounds: (i32, i32, i32, i32), // x, y, w, h
    pub confidence: f32,
    pub attributes: HashMap<String, String>,
}

/// プラグインのダウンキャストを補助するためのトレイト
pub trait PluginCast {
    fn as_any(&self) -> &dyn Any;
    fn as_tool(&self) -> Option<&dyn Tool> { None }
    fn as_communication(&self) -> Option<&dyn CommunicationAdapter> { None }
    fn as_reasoning(&self) -> Option<&dyn ReasoningEngine> { None }
    fn as_memory(&self) -> Option<&dyn MemoryProvider> { None }
    fn as_web(&self) -> Option<&dyn WebPlugin> { None }
}

/// 全てのプラグインが実装するベースとなるマーカートレイト
#[async_trait]
pub trait Plugin: Any + Send + Sync + PluginCast {
    fn manifest(&self) -> PluginManifest;

    /// プラグイン自体の初期化（権限の割り当てなど）
    async fn on_plugin_init(
        &self,
        _context: PluginRuntimeContext,
        _network: Option<Arc<dyn NetworkCapability>>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// システムイベントの購読（デフォルトは何もしない）
    /// 戻り値としてイベントデータを返すと、Kernelによって再配信される
    async fn on_event(&self, _event: &ExivEvent) -> anyhow::Result<Option<ExivEventData>> {
        Ok(None)
    }

    /// エージェント初期化時のフック（メタデータの注入など）
    async fn on_agent_init(&self, _agent: &mut AgentMetadata) -> anyhow::Result<()> {
        Ok(())
    }

    /// 実行中に権限が承認され、Capabilityが注入された際のフック
    async fn on_capability_injected(
        &self,
        _capability: PluginCapability,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

pub trait WebPlugin: Plugin {
    fn register_routes(&self, router: axum::Router<Arc<dyn Any + Send + Sync>>) -> axum::Router<Arc<dyn Any + Send + Sync>>;
}

#[async_trait]
pub trait Tool: Plugin {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<serde_json::Value>;
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

#[async_trait]
pub trait CommunicationAdapter: Plugin {
    fn name(&self) -> &str;
    async fn start(&self, event_sender: tokio::sync::mpsc::Sender<ExivEvent>)
        -> anyhow::Result<()>;
    async fn send(&self, target_user_id: &str, content: &str) -> anyhow::Result<()>;
}

#[async_trait]
pub trait ReasoningEngine: Plugin {
    fn name(&self) -> &str;
    async fn think(
        &self,
        agent: &AgentMetadata,
        message: &ExivMessage,
        context: Vec<ExivMessage>,
    ) -> anyhow::Result<String>;
}

#[async_trait]
pub trait MemoryProvider: Plugin {
    fn name(&self) -> &str;
    async fn store(&self, agent_id: String, message: ExivMessage) -> anyhow::Result<()>;
    async fn recall(
        &self,
        agent_id: String,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<ExivMessage>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExivEvent {
    pub trace_id: ExivId,
    pub timestamp: DateTime<Utc>,
    #[serde(flatten)]
    pub data: ExivEventData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GazeData {
    pub x: i32,
    pub y: i32,
    pub confidence: f32,
    pub fixated: bool, // 一定時間留まっているか
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ExivEventData {
    MessageReceived(ExivMessage),
    VisionUpdated(ColorVisionData),
    /// 視線データの更新
    GazeUpdated(GazeData),
    /// プラグインからのアクション要求（権限チェック対象）
    ActionRequested {
        requester: ExivId,
        action: HandAction,
    },
    SystemNotification(String),
    /// プラグインに対して思考（推論）を要求する
    ThoughtRequested {
        agent: AgentMetadata,
        engine_id: String,
        message: ExivMessage,
        context: Vec<ExivMessage>,
    },
    /// プラグインからの思考結果
    ThoughtResponse {
        agent_id: String,
        engine_id: String,
        content: String,
        source_message_id: String,
    },
    /// 複数プラグインによる合意形成の開始 (Prototype)
    ConsensusRequested {
        task: String,
        engine_ids: Vec<String>,
    },
    /// 各プラグインからの合意形成用提案 (Prototype)
    ConsensusProposal {
        engine_id: String,
        content: String,
    },
    /// プラグインの設定が更新された通知
    ConfigUpdated {
        plugin_id: String,
        config: std::collections::HashMap<String, String>,
    },
    /// プラグインからの権限要求
    PermissionRequested {
        plugin_id: String,
        permission: Permission,
        reason: String,
    },
    /// 権限が承認された通知
    PermissionGranted {
        plugin_id: String,
        permission: Permission,
    },
    /// マニフェストが更新された通知
    ManifestUpdated {
        plugin_id: String,
        new_manifest: PluginManifest,
    },
}

impl ExivEvent {
    pub fn new(data: ExivEventData) -> Self {
        Self {
            trace_id: ExivId::new_trace_id(),
            timestamp: Utc::now(),
            data,
        }
    }

    pub fn with_trace(trace_id: ExivId, data: ExivEventData) -> Self {
        Self {
            trace_id,
            timestamp: Utc::now(),
            data,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub default_engine_id: Option<String>,
    pub required_capabilities: Vec<CapabilityType>,
    pub plugin_bindings: Vec<ExivId>,
    pub metadata: HashMap<String, String>,
}

pub struct PluginConfig {
    pub id: String,
    pub config_values: std::collections::HashMap<String, String>,
}

#[async_trait]
pub trait PluginFactory: Send + Sync {
    fn name(&self) -> &str;
    fn service_type(&self) -> ServiceType;
    async fn create(&self, config: PluginConfig) -> anyhow::Result<Arc<dyn Plugin>>;
}

pub struct PluginRegistrar {
    pub factory: fn() -> Arc<dyn PluginFactory>,
}

inventory::collect!(PluginRegistrar);
