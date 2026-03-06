"""Tests for common.validation helpers."""

from common.validation import validate_str, validate_int, validate_dict, validate_list


# ── validate_str ──────────────────────────────────────────────


def test_str_normal():
    assert validate_str({"k": "hello"}, "k") == "hello"


def test_str_missing_returns_default():
    assert validate_str({}, "k") == ""
    assert validate_str({}, "k", "fallback") == "fallback"


def test_str_wrong_type_returns_default():
    assert validate_str({"k": 123}, "k") == ""
    assert validate_str({"k": None}, "k", "x") == "x"
    assert validate_str({"k": ["a"]}, "k") == ""


def test_str_empty_is_valid():
    assert validate_str({"k": ""}, "k", "fallback") == ""


# ── validate_int ──────────────────────────────────────────────


def test_int_normal():
    assert validate_int({"k": 42}, "k") == 42


def test_int_zero():
    assert validate_int({"k": 0}, "k", 99) == 0


def test_int_negative():
    assert validate_int({"k": -5}, "k") == -5


def test_int_missing_returns_default():
    assert validate_int({}, "k") == 0
    assert validate_int({}, "k", 10) == 10


def test_int_wrong_type_returns_default():
    assert validate_int({"k": "42"}, "k") == 0
    assert validate_int({"k": 3.14}, "k", 7) == 7
    assert validate_int({"k": None}, "k") == 0


def test_int_bool_excluded():
    """bool is a subclass of int — must be rejected."""
    assert validate_int({"k": True}, "k", 0) == 0
    assert validate_int({"k": False}, "k", 5) == 5


# ── validate_dict ─────────────────────────────────────────────


def test_dict_normal():
    assert validate_dict({"k": {"a": 1}}, "k") == {"a": 1}


def test_dict_missing_returns_empty():
    assert validate_dict({}, "k") == {}


def test_dict_wrong_type_returns_empty():
    assert validate_dict({"k": "not a dict"}, "k") == {}
    assert validate_dict({"k": [1, 2]}, "k") == {}
    assert validate_dict({"k": None}, "k") == {}


def test_dict_empty_is_valid():
    assert validate_dict({"k": {}}, "k") == {}


# ── validate_list ─────────────────────────────────────────────


def test_list_normal():
    assert validate_list({"k": [1, 2, 3]}, "k") == [1, 2, 3]


def test_list_missing_returns_empty():
    assert validate_list({}, "k") == []


def test_list_wrong_type_returns_empty():
    assert validate_list({"k": "not a list"}, "k") == []
    assert validate_list({"k": {"a": 1}}, "k") == []
    assert validate_list({"k": None}, "k") == []


def test_list_empty_is_valid():
    assert validate_list({"k": []}, "k") == []
