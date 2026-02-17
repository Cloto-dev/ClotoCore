use exiv_shared::{AgentMetadata, PluginManifest};

/// Active pane in the TUI layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Agents,
    Events,
}

impl Pane {
    pub fn next(self) -> Self {
        match self {
            Pane::Agents => Pane::Events,
            Pane::Events => Pane::Agents,
        }
    }
}

/// Actions that can be dispatched into the App state.
pub enum AppAction {
    AgentsUpdated(Vec<AgentMetadata>),
    PluginsUpdated(Vec<PluginManifest>),
    MetricsUpdated(serde_json::Value),
    NewEvent(serde_json::Value),
    #[allow(dead_code)]
    Tick,
}

/// TUI application state.
pub struct App {
    pub agents: Vec<AgentMetadata>,
    pub plugins: Vec<PluginManifest>,
    pub metrics: Option<serde_json::Value>,
    pub events: Vec<serde_json::Value>,
    pub active_pane: Pane,
    pub agent_scroll: usize,
    pub event_scroll: usize,
    pub show_help: bool,
    pub should_quit: bool,
    pub endpoint: String,
    pub connected: bool,
    pub last_refresh: std::time::Instant,
}

impl App {
    pub fn new(endpoint: String) -> Self {
        Self {
            agents: Vec::new(),
            plugins: Vec::new(),
            metrics: None,
            events: Vec::new(),
            active_pane: Pane::Agents,
            agent_scroll: 0,
            event_scroll: 0,
            show_help: false,
            should_quit: false,
            endpoint,
            connected: false,
            last_refresh: std::time::Instant::now(),
        }
    }

    pub fn apply(&mut self, action: AppAction) {
        match action {
            AppAction::AgentsUpdated(agents) => {
                self.agents = agents;
                self.connected = true;
                self.last_refresh = std::time::Instant::now();
            }
            AppAction::PluginsUpdated(plugins) => {
                self.plugins = plugins;
            }
            AppAction::MetricsUpdated(metrics) => {
                self.metrics = Some(metrics);
            }
            AppAction::NewEvent(event) => {
                self.events.push(event);
                // Keep a rolling window
                if self.events.len() > 200 {
                    self.events.drain(..self.events.len() - 200);
                }
            }
            AppAction::Tick => {}
        }
    }

    pub fn scroll_up(&mut self) {
        match self.active_pane {
            Pane::Agents => {
                self.agent_scroll = self.agent_scroll.saturating_sub(1);
            }
            Pane::Events => {
                self.event_scroll = self.event_scroll.saturating_sub(1);
            }
        }
    }

    pub fn scroll_down(&mut self) {
        match self.active_pane {
            Pane::Agents => {
                if !self.agents.is_empty() {
                    self.agent_scroll = (self.agent_scroll + 1).min(self.agents.len() - 1);
                }
            }
            Pane::Events => {
                if !self.events.is_empty() {
                    self.event_scroll = (self.event_scroll + 1).min(self.events.len() - 1);
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn selected_agent(&self) -> Option<&AgentMetadata> {
        self.agents.get(self.agent_scroll)
    }
}
