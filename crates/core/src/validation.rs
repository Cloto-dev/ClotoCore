use serde::Deserialize;
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateAgentRequest {
    #[validate(length(min = 1, max = 100, message = "Agent ID must be 1-100 characters"))]
    pub id: Option<String>,

    #[validate(length(min = 1, max = 200, message = "Name must be 1-200 characters"))]
    pub name: String,

    #[validate(length(max = 1000, message = "Description must be at most 1000 characters"))]
    pub description: Option<String>,

    pub default_engine: Option<String>,

    pub capabilities: Option<Vec<String>>,
}

// Bug #13: Custom validation function for config key characters
fn validate_config_key(key: &str) -> Result<(), validator::ValidationError> {
    // Allow alphanumeric, underscore, hyphen, dot (common config key patterns)
    // Reject control characters, whitespace, special chars that could cause issues
    if key
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        Ok(())
    } else {
        Err(validator::ValidationError::new("invalid_characters"))
    }
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdatePluginConfigRequest {
    #[validate(
        length(min = 1, max = 100, message = "Config key must be 1-100 characters"),
        custom(function = validate_config_key)
    )]
    pub key: String,

    #[validate(length(min = 1, message = "Config value cannot be empty"))]
    pub value: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct SendMessageRequest {
    #[validate(length(min = 1, message = "Content cannot be empty"))]
    pub content: String,

    pub target_agent: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ApprovePermissionRequest {
    #[validate(length(min = 1, message = "Approver ID cannot be empty"))]
    pub approved_by: String,
}

/// Validation helper function
pub fn validate_request<T: Validate>(req: &T) -> Result<(), String> {
    req.validate()
        .map_err(|e| format!("Validation error: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_agent_validation_success() {
        let req = CreateAgentRequest {
            id: Some("agent.test".to_string()),
            name: "Test Agent".to_string(),
            description: Some("A test agent".to_string()),
            default_engine: None,
            capabilities: None,
        };

        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn test_create_agent_validation_name_too_long() {
        let req = CreateAgentRequest {
            id: Some("agent.test".to_string()),
            name: "a".repeat(201), // Exceeds 200 char limit
            description: None,
            default_engine: None,
            capabilities: None,
        };

        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_create_agent_validation_empty_name() {
        let req = CreateAgentRequest {
            id: Some("agent.test".to_string()),
            name: String::new(),
            description: None,
            default_engine: None,
            capabilities: None,
        };

        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_update_config_validation_success() {
        let req = UpdatePluginConfigRequest {
            key: "api_key".to_string(),
            value: "secret_value".to_string(),
        };

        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn test_update_config_validation_empty_key() {
        let req = UpdatePluginConfigRequest {
            key: String::new(),
            value: "value".to_string(),
        };

        assert!(validate_request(&req).is_err());
    }
}
