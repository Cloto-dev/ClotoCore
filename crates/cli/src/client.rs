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

    /// DELETE request returning deserialized JSON.
    pub async fn delete_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let req = self.client.delete(self.url(path));
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

    /// PUT request with JSON body, returning deserialized JSON.
    pub async fn put<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let req = self.client.put(self.url(path)).json(body);
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

    // ── MCP Servers ─────────────────────────────────────────

    pub async fn get_mcp_servers(&self) -> Result<serde_json::Value> {
        self.get("/api/mcp/servers").await
    }

    pub async fn create_mcp_server(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        self.post("/api/mcp/servers", body).await
    }

    pub async fn delete_mcp_server(&self, name: &str) -> Result<serde_json::Value> {
        self.delete_json(&format!("/api/mcp/servers/{name}")).await
    }

    pub async fn start_mcp_server(&self, name: &str) -> Result<serde_json::Value> {
        self.post(
            &format!("/api/mcp/servers/{name}/start"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn stop_mcp_server(&self, name: &str) -> Result<serde_json::Value> {
        self.post(
            &format!("/api/mcp/servers/{name}/stop"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn restart_mcp_server(&self, name: &str) -> Result<serde_json::Value> {
        self.post(
            &format!("/api/mcp/servers/{name}/restart"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn get_mcp_server_settings(&self, name: &str) -> Result<serde_json::Value> {
        self.get(&format!("/api/mcp/servers/{name}/settings")).await
    }

    #[allow(dead_code)]
    pub async fn update_mcp_server_settings(
        &self,
        name: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.put(&format!("/api/mcp/servers/{name}/settings"), body)
            .await
    }

    pub async fn get_mcp_server_access(&self, name: &str) -> Result<serde_json::Value> {
        self.get(&format!("/api/mcp/servers/{name}/access")).await
    }

    #[allow(dead_code)]
    pub async fn put_mcp_server_access(
        &self,
        name: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.put(&format!("/api/mcp/servers/{name}/access"), body)
            .await
    }

    // ── Cron Jobs ───────────────────────────────────────────

    pub async fn list_cron_jobs(&self, agent_id: Option<&str>) -> Result<serde_json::Value> {
        let path = match agent_id {
            Some(id) => format!("/api/cron/jobs?agent_id={id}"),
            None => "/api/cron/jobs".to_string(),
        };
        self.get(&path).await
    }

    pub async fn create_cron_job(&self, body: &serde_json::Value) -> Result<serde_json::Value> {
        self.post("/api/cron/jobs", body).await
    }

    pub async fn delete_cron_job(&self, id: &str) -> Result<serde_json::Value> {
        self.delete_json(&format!("/api/cron/jobs/{id}")).await
    }

    pub async fn toggle_cron_job(&self, id: &str, enabled: bool) -> Result<serde_json::Value> {
        self.post(
            &format!("/api/cron/jobs/{id}/toggle"),
            &serde_json::json!({ "enabled": enabled }),
        )
        .await
    }

    pub async fn run_cron_job(&self, id: &str) -> Result<serde_json::Value> {
        self.post(&format!("/api/cron/jobs/{id}/run"), &serde_json::json!({}))
            .await
    }

    // ── LLM Providers ───────────────────────────────────────

    pub async fn list_llm_providers(&self) -> Result<serde_json::Value> {
        self.get("/api/llm/providers").await
    }

    pub async fn set_llm_provider_key(
        &self,
        provider_id: &str,
        api_key: &str,
    ) -> Result<serde_json::Value> {
        self.post(
            &format!("/api/llm/providers/{provider_id}/key"),
            &serde_json::json!({ "api_key": api_key }),
        )
        .await
    }

    pub async fn delete_llm_provider_key(&self, provider_id: &str) -> Result<serde_json::Value> {
        self.delete_json(&format!("/api/llm/providers/{provider_id}/key"))
            .await
    }

    // ── System ──────────────────────────────────────────────

    pub async fn get_system_version(&self) -> Result<serde_json::Value> {
        self.get("/api/system/version").await
    }

    pub async fn get_system_health(&self) -> Result<serde_json::Value> {
        self.get("/api/system/health").await
    }

    pub async fn shutdown_system(&self) -> Result<serde_json::Value> {
        self.post("/api/system/shutdown", &serde_json::json!({}))
            .await
    }

    pub async fn invalidate_api_key(&self) -> Result<serde_json::Value> {
        self.post("/api/system/invalidate-key", &serde_json::json!({}))
            .await
    }

    pub async fn get_yolo_mode(&self) -> Result<serde_json::Value> {
        self.get("/api/settings/yolo").await
    }

    pub async fn set_yolo_mode(&self, enabled: bool) -> Result<serde_json::Value> {
        self.put(
            "/api/settings/yolo",
            &serde_json::json!({ "enabled": enabled }),
        )
        .await
    }

    // ── Data ────────────────────────────────────────────────

    pub async fn get_memories(&self) -> Result<serde_json::Value> {
        self.get("/api/memories").await
    }

    pub async fn get_episodes(&self) -> Result<serde_json::Value> {
        self.get("/api/episodes").await
    }

    // ── Plugin Config ───────────────────────────────────────

    pub async fn get_plugin_config(&self, plugin_id: &str) -> Result<serde_json::Value> {
        self.get(&format!("/api/plugins/{plugin_id}/config")).await
    }

    pub async fn update_plugin_config(
        &self,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> Result<serde_json::Value> {
        self.post(
            &format!("/api/plugins/{plugin_id}/config"),
            &serde_json::json!({ "key": key, "value": value }),
        )
        .await
    }

    /// GET SSE stream (raw response for line-by-line parsing).
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
