use anyhow::{Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;

use crate::config::CliConfig;

pub struct ClotoClient {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl ClotoClient {
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
        let resp = self
            .add_auth(req)
            .send()
            .await
            .context("Failed to connect to Cloto kernel")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("{status}: {body}");
        }

        resp.json::<T>().await.context("Failed to parse response")
    }

    /// POST request with JSON body, returning deserialized JSON.
    pub async fn post<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let req = self.client.post(self.url(path)).json(body);
        let resp = self
            .add_auth(req)
            .send()
            .await
            .context("Failed to connect to Cloto kernel")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("{status}: {body}");
        }

        resp.json::<T>().await.context("Failed to parse response")
    }

    /// GET agents list.
    pub async fn get_agents(&self) -> Result<Vec<cloto_shared::AgentMetadata>> {
        self.get("/api/agents").await
    }

    /// GET plugins list.
    pub async fn get_plugins(&self) -> Result<Vec<cloto_shared::PluginManifest>> {
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

    /// DELETE agent by ID.
    pub async fn delete_agent(&self, agent_id: &str) -> Result<serde_json::Value> {
        let req = self
            .client
            .delete(self.url(&format!("/api/agents/{agent_id}")));
        let resp = self
            .add_auth(req)
            .send()
            .await
            .context("Failed to connect to Cloto kernel")?;

        let status = resp.status();
        if !status.is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("{status}: {msg}");
        }
        resp.json::<serde_json::Value>()
            .await
            .context("Failed to parse response")
    }

    /// POST power toggle.
    pub async fn power_toggle(
        &self,
        agent_id: &str,
        enabled: bool,
        password: Option<&str>,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "enabled": enabled,
            "password": password,
        });
        self.post(&format!("/api/agents/{agent_id}/power"), &body)
            .await
    }

    /// POST chat message.
    #[allow(dead_code)]
    pub async fn send_chat(&self, msg: &cloto_shared::ClotoMessage) -> Result<serde_json::Value> {
        self.post("/api/chat", msg).await
    }

    /// GET pending permission requests.
    pub async fn get_pending_permissions(&self) -> Result<Vec<serde_json::Value>> {
        self.get("/api/permissions/pending").await
    }

    /// POST approve a permission request.
    pub async fn approve_permission(&self, request_id: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "approved_by": "cli-admin" });
        self.post(&format!("/api/permissions/{request_id}/approve"), &body)
            .await
    }

    /// POST deny a permission request.
    pub async fn deny_permission(&self, request_id: &str) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "approved_by": "cli-admin" });
        self.post(&format!("/api/permissions/{request_id}/deny"), &body)
            .await
    }

    /// POST grant a permission to a plugin.
    pub async fn grant_plugin_permission(
        &self,
        plugin_id: &str,
        permission: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "permission": permission });
        self.post(
            &format!("/api/plugins/{plugin_id}/permissions/grant"),
            &body,
        )
        .await
    }

    pub async fn get_plugin_permissions(&self, plugin_id: &str) -> Result<serde_json::Value> {
        self.get(&format!("/api/plugins/{plugin_id}/permissions"))
            .await
    }

    pub async fn revoke_plugin_permission(
        &self,
        plugin_id: &str,
        permission: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({ "permission": permission });
        let req = self
            .client
            .delete(self.url(&format!("/api/plugins/{plugin_id}/permissions")))
            .json(&body);
        let resp = self
            .add_auth(req)
            .send()
            .await
            .context("Failed to connect to Cloto kernel")?;
        let status = resp.status();
        if !status.is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("{status}: {msg}");
        }
        resp.json::<serde_json::Value>()
            .await
            .context("Failed to parse response")
    }

    /// GET SSE stream (raw response for line-by-line parsing).
    #[allow(dead_code)]
    pub async fn sse_stream(&self) -> Result<reqwest::Response> {
        let req = self.client.get(self.url("/api/events"));
        let resp = self
            .add_auth(req)
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
