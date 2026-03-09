"""Tests for cpersona _clamp_limit boundary values (bug-165 verification)."""

from cpersona.server import _clamp_limit


def test_clamp_normal():
    assert _clamp_limit(10, 500) == 10


def test_clamp_at_cap():
    assert _clamp_limit(500, 500) == 500


def test_clamp_above_cap():
    assert _clamp_limit(999, 500) == 500


def test_clamp_zero():
    assert _clamp_limit(0, 500) == 0


def test_clamp_negative():
    """Negative limit must be clamped to 0, not bypass the cap."""
    assert _clamp_limit(-1, 500) == 0
    assert _clamp_limit(-999, 200) == 0


def test_clamp_cap_200():
    assert _clamp_limit(100, 200) == 100
    assert _clamp_limit(200, 200) == 200
    assert _clamp_limit(201, 200) == 200


def test_clamp_one():
    assert _clamp_limit(1, 500) == 1
