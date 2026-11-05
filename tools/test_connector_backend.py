#!/usr/bin/env python3
"""Tests for the connector backend.

The helper is what turns a reply into rows, so the parsing is tested here
rather than in a second copy in Rust.

    python3 tools/test_connector_backend.py
"""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import sys
import tempfile

_spec = importlib.util.spec_from_file_location(
    "connector_backend",
    pathlib.Path(__file__).resolve().parent / "connector_backend.py",
)
cb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(cb)


def test_a_json_array_of_objects_becomes_a_header_and_rows() -> None:
    reply = '[{"name":"a","n":1},{"name":"b","n":2}]'
    header, rows, note = cb.rows_from_json(reply, "")
    assert header == ["name", "n"], header
    assert rows == [["a", "1"], ["b", "2"]], rows
    assert "2 rows" in note, note


def test_the_key_order_is_the_reply_order() -> None:
    """The columns come out as the server sent them, not sorted."""
    reply = '[{"zebra":1,"apple":2}]'
    header, _rows, _note = cb.rows_from_json(reply, "")
    assert header == ["zebra", "apple"], header


def test_a_missing_key_leaves_a_gap_rather_than_shifting_the_row() -> None:
    reply = '[{"a":1},{"b":2}]'
    header, rows, _note = cb.rows_from_json(reply, "")
    assert header == ["a", "b"], header
    assert rows == [["1", ""], ["", "2"]], rows


def test_a_path_finds_the_array_inside_the_envelope() -> None:
    reply = '{"data":{"items":[{"x":"1"}]},"ok":true}'
    header, rows, _note = cb.rows_from_json(reply, "data.items")
    assert header == ["x"], header
    assert rows == [["1"]], rows


def test_a_missing_path_is_an_error_the_user_can_read() -> None:
    try:
        cb.rows_from_json('{"data":{}}', "data.items")
    except RuntimeError as e:
        assert "data.items" in str(e), e
    else:
        raise AssertionError("a missing path was accepted")


def test_a_single_object_becomes_one_row() -> None:
    header, rows, _note = cb.rows_from_json('{"x":1}', "")
    assert header == ["x"], header
    assert len(rows) == 1, rows


def test_csv_quoting_is_followed() -> None:
    header, rows, _note = cb.rows_from_csv('a,b\n"x,y",2\n')
    assert header == ["a", "b"], header
    assert rows == [["x,y", "2"]], rows


def test_csv_rows_are_padded_to_the_header() -> None:
    _header, rows, _note = cb.rows_from_csv("a,b,c\n1\n")
    assert rows == [["1", "", ""]], rows


def test_an_env_header_is_resolved_at_call_time() -> None:
    os.environ["IRIDIUM_TEST_TOKEN"] = "secret-value"
    try:
        headers = cb.resolved_headers({"headers": [["Authorization", "env:IRIDIUM_TEST_TOKEN"]]})
        assert headers["Authorization"] == "secret-value", headers
    finally:
        del os.environ["IRIDIUM_TEST_TOKEN"]


def test_an_env_header_that_is_not_set_is_left_out() -> None:
    headers = cb.resolved_headers({"headers": [["Authorization", "env:NOT_SET_ANYWHERE"]]})
    assert "Authorization" not in headers, headers


def test_a_plain_header_is_kept_as_it_is() -> None:
    headers = cb.resolved_headers({"headers": [["Accept", "application/json"]]})
    assert headers["Accept"] == "application/json", headers


def test_a_local_file_is_a_source_too() -> None:
    with tempfile.NamedTemporaryFile("w", suffix=".csv", delete=False) as f:
        f.write("a,b\n1,2\n")
        path = f.name
    try:
        header, rows, _note = cb.fetch({"url": path, "source": "csv"})
        assert header == ["a", "b"], header
        assert rows == [["1", "2"]], rows
    finally:
        os.unlink(path)


def test_a_connector_needs_a_url() -> None:
    try:
        cb.fetch({"url": ""})
    except RuntimeError as e:
        assert "no URL" in str(e), e
    else:
        raise AssertionError("a connector with no URL was accepted")


def test_the_store_keeps_the_connectors_and_is_private() -> None:
    """The saved connectors are on disk, readable only by the user."""
    with tempfile.TemporaryDirectory() as d:
        original = cb.STORE
        cb.STORE = pathlib.Path(d) / "connectors.json"
        try:
            connector = {"name": "x", "url": "https://example.invalid", "headers": []}
            cb.save_all([connector])
            assert cb.load() == [connector], cb.load()
            mode = cb.STORE.stat().st_mode & 0o777
            assert mode == 0o600, oct(mode)
        finally:
            cb.STORE = original


if __name__ == "__main__":
    for fn in [v for k, v in sorted(globals().items()) if k.startswith("test_")]:
        fn()
        print(f"ok  {fn.__name__}")
    print("all tests passed")
