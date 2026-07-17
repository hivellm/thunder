"""Error-string parsing per profile convention (CLT-050..052, WIRE-040) —
mirrors thunder-client's ``error.rs`` test suite exactly."""

from __future__ import annotations

from thunder_rpc import ErrorConvention, from_server_message
from thunder_rpc.errors import AuthError, ServerError


def test_resp3_auth_prefixes_map_to_auth_class() -> None:
    for message in (
        "NOAUTH Authentication required.",
        "WRONGPASS invalid username-password pair or user is disabled.",
        "NOPERM this user has no permissions",
        "NOAUTH",
    ):
        error = from_server_message(message, ErrorConvention.RESP3_PREFIXES)
        assert isinstance(
            error, AuthError
        ), f"{message} must map to the auth class (CLT-051)"
        assert error.message == message


def test_resp3_err_prefix_is_generic_server_error_without_code() -> None:
    error = from_server_message("ERR unknown command", ErrorConvention.RESP3_PREFIXES)
    assert isinstance(error, ServerError)
    assert error.message == "ERR unknown command"
    assert error.code is None


def test_resp3_prefix_must_be_word_aligned() -> None:
    # "NOAUTHx" is not the NOAUTH prefix.
    error = from_server_message("NOAUTHx nope", ErrorConvention.RESP3_PREFIXES)
    assert isinstance(error, ServerError)


def test_bracket_code_extracts_structured_code_and_keeps_raw_message() -> None:
    raw = "[collection_not_found] no such collection: docs"
    error = from_server_message(raw, ErrorConvention.BRACKET_CODE)
    assert isinstance(error, ServerError)
    assert error.message == raw
    assert error.code == "collection_not_found"


def test_bracket_code_still_maps_auth_prefixes_to_auth_class() -> None:
    # CLT-051: auth prefixes win regardless of convention.
    raw = "[unauthorized] NOAUTH token expired"
    error = from_server_message(raw, ErrorConvention.BRACKET_CODE)
    assert isinstance(error, AuthError)
    assert error.message == raw


def test_both_convention_composes_bracket_and_prefixes() -> None:
    error = from_server_message(
        "[wrongpass] WRONGPASS bad credentials", ErrorConvention.BOTH
    )
    assert isinstance(error, AuthError)

    error = from_server_message(
        "[index_missing] ERR no such index", ErrorConvention.BOTH
    )
    assert isinstance(error, ServerError)
    assert error.message == "[index_missing] ERR no such index"
    assert error.code == "index_missing"


def test_none_convention_never_parses() -> None:
    error = from_server_message("NOAUTH raw passthrough", ErrorConvention.NONE)
    assert isinstance(error, ServerError)
    assert error.message == "NOAUTH raw passthrough"
    assert error.code is None


def test_malformed_bracket_prefixes_are_left_alone() -> None:
    for message in ("[] empty", "[has space] x", "[nospace]tail", "[unclosed"):
        error = from_server_message(message, ErrorConvention.BRACKET_CODE)
        assert isinstance(error, ServerError), f"{message} must not yield a code"
        assert error.message == message
        assert error.code is None
