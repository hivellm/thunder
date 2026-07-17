"""Endpoint parsing (CLT-070/071) — mirrors thunder-client's
``endpoint.rs`` test suite exactly."""

from __future__ import annotations

import pytest

from thunder_rpc import Endpoint, parse_endpoint, registry
from thunder_rpc.errors import ConnectionError
from thunder_rpc.profile import LEXUM, SYNAP


def test_every_registered_scheme_resolves_its_default_port() -> None:
    # CLT-071: scheme -> default port comes from the registry.
    for profile in registry():
        endpoint = parse_endpoint(f"{profile.scheme}://db.example.com")
        assert endpoint.host == "db.example.com"
        assert endpoint.port == profile.default_port, profile.scheme


def test_explicit_port_wins_over_default() -> None:
    assert parse_endpoint("nexus://10.0.0.7:9999") == Endpoint(
        host="10.0.0.7", port=9999
    )


def test_bare_host_port_is_accepted_rpc_implied() -> None:
    assert parse_endpoint("localhost:15501") == Endpoint(host="localhost", port=15501)


def test_bare_host_without_port_is_rejected() -> None:
    with pytest.raises(ConnectionError):
        parse_endpoint("localhost")


def test_http_and_https_are_rejected_with_pointer_to_http_client() -> None:
    for url in ("http://vec.example.com:8080", "https://vec.example.com"):
        with pytest.raises(ConnectionError) as excinfo:
            parse_endpoint(url)
        message = str(excinfo.value)
        assert "RPC-only" in message and "HTTP client" in message


def test_unknown_scheme_is_rejected_listing_the_registry() -> None:
    with pytest.raises(ConnectionError) as excinfo:
        parse_endpoint("redis://h:1")
    message = str(excinfo.value)
    for scheme in ("synap", "nexus", "vectorizer", "lexum"):
        assert scheme in message, f"must list '{scheme}': {message}"


def test_ipv6_literals_parse_with_and_without_brackets() -> None:
    assert parse_endpoint("[::1]:8080") == Endpoint(host="::1", port=8080)
    endpoint = parse_endpoint("synap://[fe80::1]")
    assert endpoint.host == "fe80::1"
    assert endpoint.port == SYNAP.default_port


def test_trailing_slash_is_tolerated_but_paths_are_not() -> None:
    assert parse_endpoint("lexum://h/").port == LEXUM.default_port
    with pytest.raises(ConnectionError):
        parse_endpoint("lexum://h/db")


def test_invalid_ports_are_rejected() -> None:
    for bad in ("host:99999", "synap://host:abc", ":1234"):
        with pytest.raises(ConnectionError):
            parse_endpoint(bad)
