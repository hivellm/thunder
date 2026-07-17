"""Endpoint parsing (CLT-070/071) — mirrors the Rust ``endpoint.rs`` test
suite exactly."""

from __future__ import annotations

import pytest

from thunder_rpc import Config, Endpoint, parse_endpoint
from thunder_rpc.errors import ConnectionError


def app() -> Config:
    """An application's config — Thunder ships no schemes of its own, so the
    tests bring their own, exactly as an application does."""
    return Config.standard().with_scheme("myapp").with_port(9000)


def test_the_configured_scheme_resolves_the_configured_default_port() -> None:
    # CLT-071: scheme -> default port comes from the application's own
    # config, not from any registry Thunder carries.
    endpoint = parse_endpoint("myapp://db.example.com", app())
    assert endpoint.host == "db.example.com"
    assert endpoint.port == 9000


def test_any_application_can_pick_any_scheme_without_a_thunder_release() -> None:
    # The whole point of dropping the registry: a scheme Thunder has never
    # heard of works because the application configured it.
    future = Config.standard().with_scheme("something-new-in-2030").with_port(4242)
    assert parse_endpoint("something-new-in-2030://host", future).port == 4242


def test_explicit_port_wins_over_default() -> None:
    assert parse_endpoint("myapp://10.0.0.7:9999", app()) == Endpoint(
        host="10.0.0.7", port=9999
    )


def test_bare_host_port_is_accepted_rpc_implied() -> None:
    assert parse_endpoint("localhost:15501", app()) == Endpoint(
        host="localhost", port=15501
    )


def test_bare_host_port_works_even_with_no_scheme_configured() -> None:
    # Config.standard() has no identity until an application gives it one; an
    # explicit host:port needs none.
    assert parse_endpoint("localhost:15501", Config.standard()).port == 15501


def test_bare_host_without_port_is_rejected() -> None:
    with pytest.raises(ConnectionError):
        parse_endpoint("localhost", app())


def test_http_and_https_are_rejected_with_pointer_to_http_client() -> None:
    for url in ("http://db.example.com:8080", "https://db.example.com"):
        with pytest.raises(ConnectionError) as excinfo:
            parse_endpoint(url, app())
        message = str(excinfo.value)
        assert "RPC-only" in message and "HTTP client" in message


def test_a_scheme_other_than_the_configured_one_is_rejected() -> None:
    with pytest.raises(ConnectionError) as excinfo:
        parse_endpoint("redis://h:1", app())
    message = str(excinfo.value)
    assert (
        "redis" in message and "myapp" in message
    ), f"the mismatch must name both the given and the configured scheme: {message}"


def test_ipv6_literals_parse_with_and_without_brackets() -> None:
    assert parse_endpoint("[::1]:8080", app()) == Endpoint(host="::1", port=8080)
    endpoint = parse_endpoint("myapp://[fe80::1]", app())
    assert endpoint.host == "fe80::1"
    assert endpoint.port == 9000


def test_trailing_slash_is_tolerated_but_paths_are_not() -> None:
    assert parse_endpoint("myapp://h/", app()).port == 9000
    with pytest.raises(ConnectionError):
        parse_endpoint("myapp://h/db", app())


def test_invalid_ports_are_rejected() -> None:
    for bad in ("host:99999", "myapp://host:abc", ":1234"):
        with pytest.raises(ConnectionError):
            parse_endpoint(bad, app())


def test_empty_host_is_rejected() -> None:
    for bad in ("myapp://:1234", ":1234"):
        with pytest.raises(ConnectionError):
            parse_endpoint(bad, app())
