"""
Cloto MCP Server: Agent Utilities
Deterministic tools that compensate for LLM weaknesses:
time awareness, arithmetic, date math, randomness, UUIDs,
unit conversion, encoding/decoding, and hashing.

All tools use Python stdlib only — no external dependencies.
"""

import asyncio
import ast
import base64
import hashlib
import html
import math
import operator
import os
import random
import secrets
import sys
import uuid
from datetime import datetime, timedelta, timezone
from urllib.parse import quote, unquote

sys.path.insert(0, os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")))

from common.mcp_utils import ToolRegistry, run_mcp_server

# ============================================================
# Safe math evaluator (no eval/exec)
# ============================================================

_SAFE_OPERATORS = {
    ast.Add: operator.add,
    ast.Sub: operator.sub,
    ast.Mult: operator.mul,
    ast.Div: operator.truediv,
    ast.FloorDiv: operator.floordiv,
    ast.Mod: operator.mod,
    ast.Pow: operator.pow,
    ast.USub: operator.neg,
    ast.UAdd: operator.pos,
}

_SAFE_FUNCTIONS = {
    "abs": abs,
    "round": round,
    "min": min,
    "max": max,
    "sqrt": math.sqrt,
    "sin": math.sin,
    "cos": math.cos,
    "tan": math.tan,
    "log": math.log,
    "log10": math.log10,
    "log2": math.log2,
    "floor": math.floor,
    "ceil": math.ceil,
    "pi": math.pi,
    "e": math.e,
}


def _safe_eval_node(node):
    """Recursively evaluate an AST node with only safe operations."""
    if isinstance(node, ast.Expression):
        return _safe_eval_node(node.body)
    elif isinstance(node, ast.Constant):
        if isinstance(node.value, (int, float, complex)):
            return node.value
        raise ValueError(f"Unsupported constant: {node.value!r}")
    elif isinstance(node, ast.BinOp):
        op = _SAFE_OPERATORS.get(type(node.op))
        if op is None:
            raise ValueError(f"Unsupported operator: {type(node.op).__name__}")
        left = _safe_eval_node(node.left)
        right = _safe_eval_node(node.right)
        return op(left, right)
    elif isinstance(node, ast.UnaryOp):
        op = _SAFE_OPERATORS.get(type(node.op))
        if op is None:
            raise ValueError(f"Unsupported unary operator: {type(node.op).__name__}")
        return op(_safe_eval_node(node.operand))
    elif isinstance(node, ast.Call):
        if isinstance(node.func, ast.Name) and node.func.id in _SAFE_FUNCTIONS:
            func = _SAFE_FUNCTIONS[node.func.id]
            if callable(func):
                args = [_safe_eval_node(a) for a in node.args]
                return func(*args)
            return func  # constant like pi, e
        raise ValueError(f"Unsupported function: {ast.dump(node.func)}")
    elif isinstance(node, ast.Name):
        if node.id in _SAFE_FUNCTIONS:
            val = _SAFE_FUNCTIONS[node.id]
            if not callable(val):
                return val  # pi, e
        raise ValueError(f"Unsupported name: {node.id}")
    raise ValueError(f"Unsupported expression: {ast.dump(node)}")


def safe_calculate(expression: str):
    """Evaluate a math expression safely without eval()."""
    tree = ast.parse(expression.strip(), mode="eval")
    return _safe_eval_node(tree)


# ============================================================
# Unit conversion tables
# ============================================================

# All units normalized to a base unit per category
_UNIT_TABLE = {
    "length": {
        "base": "m",
        "mm": 0.001, "cm": 0.01, "m": 1.0, "km": 1000.0,
        "in": 0.0254, "ft": 0.3048, "yd": 0.9144, "mi": 1609.344,
    },
    "weight": {
        "base": "kg",
        "mg": 1e-6, "g": 0.001, "kg": 1.0, "t": 1000.0,
        "oz": 0.028349523125, "lb": 0.45359237,
    },
    "temperature": {"base": "special"},
    "time": {
        "base": "s",
        "ms": 0.001, "s": 1.0, "min": 60.0, "h": 3600.0,
        "d": 86400.0, "week": 604800.0,
    },
    "data": {
        "base": "B",
        "B": 1.0, "KB": 1024.0, "MB": 1024**2, "GB": 1024**3, "TB": 1024**4,
    },
}


def _convert_temperature(value: float, from_u: str, to_u: str) -> float:
    # Normalize to Celsius first
    if from_u == "C":
        c = value
    elif from_u == "F":
        c = (value - 32) * 5 / 9
    elif from_u == "K":
        c = value - 273.15
    else:
        raise ValueError(f"Unknown temperature unit: {from_u}")
    # Convert from Celsius to target
    if to_u == "C":
        return c
    elif to_u == "F":
        return c * 9 / 5 + 32
    elif to_u == "K":
        return c + 273.15
    raise ValueError(f"Unknown temperature unit: {to_u}")


def convert_units(value: float, from_unit: str, to_unit: str) -> dict:
    """Convert between units in the same category."""
    for category, units in _UNIT_TABLE.items():
        if category == "temperature":
            temp_units = {"C", "F", "K"}
            if from_unit in temp_units and to_unit in temp_units:
                result = _convert_temperature(value, from_unit, to_unit)
                return {"value": result, "from": from_unit, "to": to_unit, "category": "temperature"}
            continue
        if from_unit in units and to_unit in units:
            base_value = value * units[from_unit]
            result = base_value / units[to_unit]
            return {"value": result, "from": from_unit, "to": to_unit, "category": category}
    raise ValueError(
        f"Cannot convert '{from_unit}' to '{to_unit}'. "
        f"Units must be in the same category. "
        f"Available: {', '.join(cat + ': ' + '/'.join(u for u in units if u != 'base') for cat, units in _UNIT_TABLE.items())}"
    )


# ============================================================
# MCP Server
# ============================================================

registry = ToolRegistry("cloto-mcp-agent-utils")


@registry.tool(
    "get_current_time",
    "Get the current date, time, weekday, and unix timestamp.",
    {
        "type": "object",
        "properties": {
            "timezone": {
                "type": "string",
                "description": "Timezone: 'UTC', 'local', or offset like '+09:00' (default: UTC)",
                "default": "UTC",
            },
        },
        "required": [],
    },
)
async def do_get_current_time(args: dict) -> dict:
    tz_name = args.get("timezone", "UTC")
    if tz_name == "local":
        now = datetime.now().astimezone()
    elif tz_name == "UTC":
        now = datetime.now(timezone.utc)
    else:
        # Offset-based: "+09:00", "-05:00"
        try:
            h, m = 0, 0
            parts = tz_name.replace("UTC", "").replace(":", "")
            if parts:
                sign = 1 if parts[0] != "-" else -1
                digits = parts.lstrip("+-")
                h = int(digits[:2]) if len(digits) >= 2 else int(digits)
                m = int(digits[2:4]) if len(digits) >= 4 else 0
                tz = timezone(timedelta(hours=sign * h, minutes=sign * m))
                now = datetime.now(tz)
            else:
                now = datetime.now(timezone.utc)
        except (ValueError, IndexError):
            now = datetime.now(timezone.utc)

    return {
        "iso8601": now.isoformat(),
        "date": now.strftime("%Y-%m-%d"),
        "time": now.strftime("%H:%M:%S"),
        "weekday": now.strftime("%A"),
        "unix_timestamp": int(now.timestamp()),
        "timezone": str(now.tzinfo),
    }


@registry.tool(
    "calculate",
    "Evaluate a mathematical expression safely. "
    "Supports +, -, *, /, //, %, ** and functions: "
    "sqrt, sin, cos, tan, log, log10, log2, abs, round, min, max, floor, ceil, pi, e.",
    {
        "type": "object",
        "properties": {
            "expression": {
                "type": "string",
                "description": "Math expression (e.g. '2**10', 'sqrt(144)', '355/113')",
            },
        },
        "required": ["expression"],
    },
)
async def do_calculate(args: dict) -> dict:
    expression = args.get("expression", "")
    if not expression:
        return {"error": "expression is required"}
    result = safe_calculate(expression)
    # Format nicely
    if isinstance(result, float) and result == int(result) and abs(result) < 1e15:
        display = str(int(result))
    else:
        display = str(result)
    return {"expression": expression, "result": result, "display": display}


@registry.tool(
    "date_math",
    "Add/subtract time from a date, or calculate the difference between two dates.",
    {
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["add", "subtract", "diff"],
                "description": "'add'/'subtract' time from date, or 'diff' between two dates",
            },
            "date": {
                "type": "string",
                "description": "Base date in ISO 8601 format, or 'now' (default: now)",
            },
            "date2": {
                "type": "string",
                "description": "Second date for 'diff' action (default: now)",
            },
            "days": {"type": "integer", "description": "Days to add/subtract"},
            "hours": {"type": "integer", "description": "Hours to add/subtract"},
            "minutes": {"type": "integer", "description": "Minutes to add/subtract"},
            "weeks": {"type": "integer", "description": "Weeks to add/subtract"},
        },
        "required": ["action"],
    },
)
async def do_date_math(args: dict) -> dict:
    action = args.get("action", "add")
    date_str = args.get("date", "")
    if not date_str or date_str == "now":
        base = datetime.now(timezone.utc)
    else:
        base = datetime.fromisoformat(date_str.replace("Z", "+00:00"))

    if action == "diff":
        date2_str = args.get("date2", "")
        if not date2_str or date2_str == "now":
            date2 = datetime.now(timezone.utc)
        else:
            date2 = datetime.fromisoformat(date2_str.replace("Z", "+00:00"))
        delta = date2 - base
        return {
            "date1": base.isoformat(),
            "date2": date2.isoformat(),
            "days": delta.days,
            "total_seconds": int(delta.total_seconds()),
            "human": f"{abs(delta.days)} days {'later' if delta.days >= 0 else 'earlier'}",
        }

    days = args.get("days", 0)
    hours = args.get("hours", 0)
    minutes = args.get("minutes", 0)
    weeks = args.get("weeks", 0)
    delta = timedelta(days=days, hours=hours, minutes=minutes, weeks=weeks)

    if action == "subtract":
        delta = -delta

    result = base + delta
    return {
        "original": base.isoformat(),
        "result": result.isoformat(),
        "weekday": result.strftime("%A"),
        "delta": str(delta),
    }


@registry.tool(
    "random_number",
    "Generate random integer(s) within a range.",
    {
        "type": "object",
        "properties": {
            "min": {"type": "integer", "description": "Minimum value (default: 1)"},
            "max": {"type": "integer", "description": "Maximum value (default: 100)"},
            "count": {"type": "integer", "description": "How many numbers (max 100, default: 1)"},
            "secure": {
                "type": "boolean",
                "description": "Use cryptographic RNG (default: false)",
            },
        },
        "required": [],
    },
)
async def do_random_number(args: dict) -> dict:
    min_val = args.get("min", 1)
    max_val = args.get("max", 100)
    count = min(args.get("count", 1), 100)
    use_crypto = args.get("secure", False)

    if use_crypto:
        numbers = [secrets.randbelow(max_val - min_val + 1) + min_val for _ in range(count)]
    else:
        numbers = [random.randint(min_val, max_val) for _ in range(count)]

    return {
        "numbers": numbers if count > 1 else numbers[0],
        "min": min_val,
        "max": max_val,
        "count": count,
        "secure": use_crypto,
    }


@registry.tool(
    "generate_uuid",
    "Generate a random UUID v4.",
    {"type": "object", "properties": {}, "required": []},
)
async def do_generate_uuid(args: dict) -> dict:
    return {"uuid": str(uuid.uuid4())}


@registry.tool(
    "convert_units",
    "Convert between units. Categories: "
    "length (mm/cm/m/km/in/ft/yd/mi), "
    "weight (mg/g/kg/t/oz/lb), "
    "temperature (C/F/K), "
    "time (ms/s/min/h/d/week), "
    "data (B/KB/MB/GB/TB).",
    {
        "type": "object",
        "properties": {
            "value": {"type": "number", "description": "The value to convert"},
            "from_unit": {"type": "string", "description": "Source unit (e.g. 'km', 'lb', 'F')"},
            "to_unit": {"type": "string", "description": "Target unit (e.g. 'mi', 'kg', 'C')"},
        },
        "required": ["value", "from_unit", "to_unit"],
    },
)
async def do_convert_units(args: dict) -> dict:
    value = args.get("value")
    from_unit = args.get("from_unit", "")
    to_unit = args.get("to_unit", "")
    if value is None or not from_unit or not to_unit:
        return {"error": "value, from_unit, and to_unit are required"}
    return convert_units(float(value), from_unit, to_unit)


@registry.tool(
    "encode_decode",
    "Encode or decode text. Supports base64, URL encoding, hex, and HTML entities.",
    {
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "enum": ["encode", "decode"],
                "description": "'encode' or 'decode'",
            },
            "encoding": {
                "type": "string",
                "enum": ["base64", "url", "hex", "html"],
                "description": "Encoding type",
            },
            "text": {"type": "string", "description": "Text to encode/decode"},
        },
        "required": ["action", "encoding", "text"],
    },
)
async def do_encode_decode(args: dict) -> dict:
    action = args.get("action", "encode")
    encoding = args.get("encoding", "base64")
    text = args.get("text", "")
    if not text:
        return {"error": "text is required"}

    if encoding == "base64":
        if action == "encode":
            result = base64.b64encode(text.encode("utf-8")).decode("ascii")
        else:
            result = base64.b64decode(text).decode("utf-8")
    elif encoding == "url":
        if action == "encode":
            result = quote(text, safe="")
        else:
            result = unquote(text)
    elif encoding == "hex":
        if action == "encode":
            result = text.encode("utf-8").hex()
        else:
            result = bytes.fromhex(text).decode("utf-8")
    elif encoding == "html":
        if action == "encode":
            result = html.escape(text)
        else:
            result = html.unescape(text)
    else:
        return {"error": f"Unknown encoding: {encoding}. Supported: base64, url, hex, html"}

    return {"action": action, "encoding": encoding, "input": text, "output": result}


@registry.tool(
    "hash",
    "Compute a hash of the given text. Supports md5, sha1, sha256, sha512.",
    {
        "type": "object",
        "properties": {
            "text": {"type": "string", "description": "Text to hash"},
            "algorithm": {
                "type": "string",
                "enum": ["md5", "sha1", "sha256", "sha512"],
                "description": "Hash algorithm (default: sha256)",
            },
        },
        "required": ["text"],
    },
)
async def do_hash(args: dict) -> dict:
    text = args.get("text", "")
    algorithm = args.get("algorithm", "sha256")
    if not text:
        return {"error": "text is required"}

    supported = {"md5", "sha1", "sha256", "sha512"}
    if algorithm not in supported:
        return {"error": f"Unknown algorithm: {algorithm}. Supported: {', '.join(sorted(supported))}"}

    h = hashlib.new(algorithm)
    h.update(text.encode("utf-8"))
    return {"algorithm": algorithm, "hash": h.hexdigest(), "input_length": len(text)}


# ============================================================
# Entry point
# ============================================================


if __name__ == "__main__":
    asyncio.run(run_mcp_server(registry))
