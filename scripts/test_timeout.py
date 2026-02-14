"""
Test script for Python Bridge timeout testing
"""
import time

EXIV_MANIFEST = {
    "id": "test.timeout",
    "name": "Timeout Test Script",
    "description": "Script for testing timeout functionality",
    "version": "0.1.0",
    "capabilities": ["Reasoning"]
}

def think(params):
    """Simulates a long-running blocking operation"""
    # Sleep for 15 seconds (longer than Rust's 10s timeout)
    time.sleep(15)
    return "This should timeout before completing"

def quick_think(params):
    """Returns immediately for testing successful execution"""
    return {"thought": "Quick response", "success": True}
