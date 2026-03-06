"""Common argument validation helpers for MCP tool handlers.

All validators return a safe default on type mismatch (graceful degradation).
"""


def validate_str(arguments: dict, key: str, default: str = "") -> str:
    """Extract a string value, returning *default* if missing or wrong type."""
    val = arguments.get(key, default)
    if not isinstance(val, str):
        return default
    return val


def validate_int(arguments: dict, key: str, default: int = 0) -> int:
    """Extract an integer value, returning *default* if missing or wrong type.

    ``bool`` is explicitly excluded (``isinstance(True, int)`` is ``True``).
    """
    val = arguments.get(key, default)
    if isinstance(val, bool) or not isinstance(val, int):
        return default
    return val


def validate_dict(arguments: dict, key: str) -> dict:
    """Extract a dict value, returning ``{}`` if missing or wrong type."""
    val = arguments.get(key, {})
    if not isinstance(val, dict):
        return {}
    return val


def validate_list(arguments: dict, key: str) -> list:
    """Extract a list value, returning ``[]`` if missing or wrong type."""
    val = arguments.get(key, [])
    if not isinstance(val, list):
        return []
    return val
