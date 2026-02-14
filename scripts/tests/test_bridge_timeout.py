"""
Test timeout functionality for Python Bridge
"""
import sys
import time
import os

# Add parent directory to path to import bridge_runtime
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from bridge_runtime import call_method_with_timeout

def blocking_method(params):
    """Simulates infinite loop / blocking operation"""
    while True:
        time.sleep(0.1)
    return "This should never be reached"

def quick_method(params):
    """Returns immediately"""
    return "success"

def slow_method(params):
    """Takes 2 seconds"""
    time.sleep(2)
    return "completed after 2 seconds"

def error_method(params):
    """Raises an exception"""
    raise ValueError("Intentional test error")

def test_timeout_on_blocking_method():
    """Test that blocking methods timeout correctly"""
    print("Testing timeout on blocking method...")
    result = call_method_with_timeout(blocking_method, {}, timeout_secs=1)

    assert result["success"] is False, f"Expected failure, got: {result}"
    assert "timeout" in result["error"].lower(), f"Expected timeout error, got: {result['error']}"
    print("✓ Blocking method timed out correctly")

def test_no_timeout_on_quick_method():
    """Test that quick methods complete successfully"""
    print("Testing quick method...")
    result = call_method_with_timeout(quick_method, {}, timeout_secs=1)

    assert result["success"] is True, f"Expected success, got: {result}"
    assert result["result"] == "success", f"Expected 'success', got: {result['result']}"
    print("✓ Quick method completed successfully")

def test_slow_method_within_timeout():
    """Test that slow methods complete if within timeout"""
    print("Testing slow method within timeout...")
    result = call_method_with_timeout(slow_method, {}, timeout_secs=3)

    assert result["success"] is True, f"Expected success, got: {result}"
    assert result["result"] == "completed after 2 seconds", f"Unexpected result: {result}"
    print("✓ Slow method completed within timeout")

def test_slow_method_exceeds_timeout():
    """Test that slow methods timeout if exceeding limit"""
    print("Testing slow method exceeding timeout...")
    result = call_method_with_timeout(slow_method, {}, timeout_secs=1)

    assert result["success"] is False, f"Expected failure, got: {result}"
    assert "timeout" in result["error"].lower(), f"Expected timeout error, got: {result['error']}"
    print("✓ Slow method timed out correctly")

def test_error_handling():
    """Test that exceptions are properly caught and returned"""
    print("Testing error handling...")
    result = call_method_with_timeout(error_method, {}, timeout_secs=1)

    assert result["success"] is False, f"Expected failure, got: {result}"
    assert "Intentional test error" in result["error"], f"Expected error message, got: {result['error']}"
    assert "traceback" in result, "Expected traceback in result"
    print("✓ Error handled correctly")

def test_configurable_timeout():
    """Test that timeout value can be configured"""
    print("Testing configurable timeout...")

    # Should timeout with 1 second
    result1 = call_method_with_timeout(slow_method, {}, timeout_secs=1)
    assert result1["success"] is False, "Expected timeout with 1 second"

    # Should succeed with 3 seconds
    result2 = call_method_with_timeout(slow_method, {}, timeout_secs=3)
    assert result2["success"] is True, "Expected success with 3 seconds"

    print("✓ Timeout is configurable")

def run_all_tests():
    """Run all test functions"""
    tests = [
        test_no_timeout_on_quick_method,
        test_slow_method_within_timeout,
        test_slow_method_exceeds_timeout,
        test_error_handling,
        test_configurable_timeout,
        test_timeout_on_blocking_method,  # Run last as it takes the full timeout
    ]

    passed = 0
    failed = 0

    for test in tests:
        try:
            test()
            passed += 1
        except AssertionError as e:
            print(f"✗ {test.__name__} failed: {e}")
            failed += 1
        except Exception as e:
            print(f"✗ {test.__name__} error: {e}")
            failed += 1

    print(f"\n{'='*60}")
    print(f"Test Results: {passed} passed, {failed} failed")
    print(f"{'='*60}")

    return failed == 0

if __name__ == "__main__":
    success = run_all_tests()
    sys.exit(0 if success else 1)
