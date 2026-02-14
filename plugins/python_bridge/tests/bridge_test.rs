use plugin_python_bridge::PythonBridgePlugin;
use exiv_shared::{Plugin, PluginConfig};
use std::collections::HashMap;

#[tokio::test]
async fn test_python_bridge_initialization() {
    // Test basic plugin initialization
    let config = PluginConfig {
        id: "bridge.python".to_string(),
        config_values: [
            ("script_path".to_string(), "scripts/bridge_main.py".to_string()),
        ].into_iter().collect(),
    };

    let plugin = PythonBridgePlugin::new_plugin(config).await;
    assert!(plugin.is_ok(), "Plugin initialization should succeed");

    let plugin = plugin.unwrap();
    let manifest = plugin.manifest();

    assert_eq!(manifest.id, "bridge.python");
    assert_eq!(manifest.name, "bridge.python");
    assert_eq!(manifest.magic_seal, 0x56455253); // VERS - official SDK magic seal
}

#[tokio::test]
async fn test_python_bridge_path_validation_prevents_traversal() {
    // Test that path traversal is prevented
    let config = PluginConfig {
        id: "bridge.python".to_string(),
        config_values: [
            ("script_path".to_string(), "../../../etc/passwd".to_string()),
        ].into_iter().collect(),
    };

    let result = PythonBridgePlugin::new_plugin(config).await;
    assert!(result.is_err(), "Should reject path with '..'");

    if let Err(e) = result {
        assert!(e.to_string().contains(".."), "Error should mention '..'");
    }
}

#[tokio::test]
async fn test_python_bridge_path_validation_requires_scripts_dir() {
    // Test that absolute paths are rejected
    let config = PluginConfig {
        id: "bridge.python".to_string(),
        config_values: [
            ("script_path".to_string(), "/absolute/path/script.py".to_string()),
        ].into_iter().collect(),
    };

    let result = PythonBridgePlugin::new_plugin(config).await;
    assert!(result.is_err(), "Should reject absolute paths");

    // Test that paths outside scripts/ are rejected
    let config2 = PluginConfig {
        id: "bridge.python".to_string(),
        config_values: [
            ("script_path".to_string(), "other_dir/script.py".to_string()),
        ].into_iter().collect(),
    };

    let result2 = PythonBridgePlugin::new_plugin(config2).await;
    assert!(result2.is_err(), "Should reject paths outside scripts/");
}

#[tokio::test]
async fn test_python_bridge_default_script_path() {
    // Test that default script path is used when not specified
    let config = PluginConfig {
        id: "bridge.python".to_string(),
        config_values: HashMap::new(),
    };

    let plugin = PythonBridgePlugin::new_plugin(config).await;
    assert!(plugin.is_ok(), "Should use default script path");
}

// Note: Testing actual Python process startup, method calls, timeouts, and restarts
// requires a Python environment and test scripts.
//
// Python-side timeout tests are implemented in:
// - scripts/tests/test_bridge_timeout.py (unit tests for timeout mechanism)
//
// These tests verify:
// - Blocking methods timeout correctly (after configured seconds)
// - Quick methods complete successfully
// - Slow methods complete if within timeout
// - Errors are properly caught and returned
// - Timeout is configurable via environment variable
//
// Run Python tests: cd scripts && python3 tests/test_bridge_timeout.py
//
// Integration tests with actual Python process would require:
//
// #[tokio::test]
// #[ignore] // Requires Python environment
// async fn test_python_process_startup() { ... }
//
// #[tokio::test]
// #[ignore] // Requires Python environment
// async fn test_method_call_timeout() {
//     // Test script: scripts/test_timeout.py
//     // Verifies that methods exceeding timeout return error
// }
//
// #[tokio::test]
// #[ignore] // Requires Python environment
// async fn test_restart_after_crash() { ... }
//
// #[tokio::test]
// #[ignore] // Requires Python environment
// async fn test_max_restart_attempts() { ... }
