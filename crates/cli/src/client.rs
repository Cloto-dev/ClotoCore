use anyhow::{Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::config::CliConfig;

pub struct ExivClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl ExivClient {
    pub fn new(config: &CliConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: config.url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn add_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(key) => req.header("X-API-Key", key),
            None => req,
        }
    }

    /// GET request returning deserialized JSON.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let req = self.client.get(self.url(path));
        let resp = self.add_auth(req)
            .send()
            .await
            .context("Failed to connect to Exiv kernel")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("{status}: {body}");
        }

        resp.json::<T>().await.context("Failed to parse response")
    }

    /// POST request with JSON body, returning deserialized JSON.
    pub async fn post<B: serde::Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let req = self.client.post(self.url(path)).json(body);
        let resp = self.add_auth(req)
            .send()
            .await
            .context("Failed to connect to Exiv kernel")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("{status}: {body}");
        }

        resp.json::<T>().await.context("Failed to parse response")
    }

    /// GET agents list.
    pub async fn get_agents(&self) -> Result<Vec<exiv_shared::AgentMetadata>> {
        self.get("/api/agents").await
    }

    /// GET plugins list.
    pub async fn get_plugins(&self) -> Result<Vec<exiv_shared::PluginManifest>> {
        self.get("/api/plugins").await
    }

    /// GET system metrics.
    pub async fn get_metrics(&self) -> Result<serde_json::Value> {
        self.get("/api/metrics").await
    }

    /// GET event history.
    #[allow(dead_code)]
    pub async fn get_history(&self) -> Result<Vec<serde_json::Value>> {
        self.get("/api/history").await
    }

    /// POST create agent.
    pub async fn create_agent(&self, req: &serde_json::Value) -> Result<serde_json::Value> {
        self.post("/api/agents", req).await
    }

    /// POST power toggle.
    pub async fn power_toggle(&self, agent_id: &str, enabled: bool, password: Option<&str>) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "enabled": enabled,
            "password": password,
        });
        self.post(&format!("/api/agents/{agent_id}/power"), &body).await
    }

    /// POST chat message.
    #[allow(dead_code)]
    pub async fn send_chat(&self, msg: &exiv_shared::ExivMessage) -> Result<serde_json::Value> {
        self.post("/api/chat", msg).await
    }

    /// GET pending permission requests.
    pub async fn get_pending_permissions(&self) -> Result<Vec<serde_json::Value>> {
        self.get("/api/permissions/pending").await
    }

    /// POST approve a permission request.
    pub async fn approve_permission(&self, request_id: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "approved_by": "cli-admin" });
        self.post(&format!("/api/permissions/{request_id}/approve"), &body).await
    }

    /// POST deny a permission request.
    pub async fn deny_permission(&self, request_id: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "approved_by": "cli-admin" });
        self.post(&format!("/api/permissions/{request_id}/deny"), &body).await
    }

    /// POST grant a permission to a plugin.
    pub async fn grant_plugin_permission(&self, plugin_id: &str, permission: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "permission": permission });
        self.post(&format!("/api/plugins/{plugin_id}/permissions/grant"), &body).await
    }

    /// GET SSE stream (raw response for line-by-line parsing).
    #[allow(dead_code)]
    pub async fn sse_stream(&self) -> Result<reqwest::Response> {
        let req = self.client.get(self.url("/api/events"));
        let resp = self.add_auth(req)
            .send()
            .await
            .context("Failed to connect to SSE stream")?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("SSE connection failed: {body}");
        }

        Ok(resp)
    }
}
