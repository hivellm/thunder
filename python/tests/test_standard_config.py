"""Pins ``Config.standard()`` to ``conformance/standard.yaml`` (PRO-013).

Thunder ships **one** standard and no product knowledge, so this is the whole
registry check: a change to the standard that is not mirrored in the
language-neutral YAML — or vice versa — fails here, in all four languages.
That cross-language agreement was the only job the old per-product registry
legitimately did; it survives without any product name (mirrors
``rust/thunder/tests/standard_config.rs``).
"""

from __future__ import annotations

from pathlib import Path

import yaml

from thunder_rpc import (
    Config,
    ErrorConvention,
    Handshake,
    HelloStyle,
    PushPolicy,
    TlsPolicy,
)

STANDARD_YAML = Path(__file__).resolve().parents[2] / "conformance" / "standard.yaml"

_HANDSHAKE = {
    "none": Handshake.NONE,
    "auth_command": Handshake.AUTH_COMMAND,
    "hello_mandatory": Handshake.HELLO_MANDATORY,
}
_HELLO_STYLE = {
    "not_used": HelloStyle.NOT_USED,
    "arg_less": HelloStyle.ARG_LESS,
    "map_payload": HelloStyle.MAP_PAYLOAD,
}
_PUSH = {"reserved": PushPolicy.RESERVED, "enabled": PushPolicy.ENABLED}
_ERRORS = {
    "none": ErrorConvention.NONE,
    "resp3_prefixes": ErrorConvention.RESP3_PREFIXES,
    "bracket_code": ErrorConvention.BRACKET_CODE,
    "both": ErrorConvention.BOTH,
}
_TLS = {
    "off": TlsPolicy.OFF,
    "optional_rustls": TlsPolicy.OPTIONAL,
    "reserved_config": TlsPolicy.RESERVED,
}


def test_standard_matches_the_conformance_data_file() -> None:
    raw = yaml.safe_load(STANDARD_YAML.read_text(encoding="utf-8"))
    standard = Config.standard()

    assert _HANDSHAKE[raw["handshake"]] is standard.handshake
    assert _HELLO_STYLE[raw["hello_style"]] is standard.hello_style
    assert _PUSH[raw["push"]] is standard.push
    assert raw["max_frame_bytes"] == standard.max_frame_bytes
    assert raw["max_in_flight"] == standard.max_in_flight
    assert _ERRORS[raw["error_codes"]] is standard.error_codes
    # The YAML quotes `"off"` so YAML 1.1 loaders cannot coerce it to False;
    # the token is a plain string on every loader.
    assert _TLS[raw["tls"]] is standard.tls


def test_default_is_the_standard() -> None:
    assert Config() == Config.standard()


def test_the_standard_carries_no_identity() -> None:
    # Identity is the application's: Thunder has no opinion about which
    # scheme or port an implementation answers on.
    standard = Config.standard()
    assert standard.scheme == ""
    assert standard.default_port == 0


def test_an_application_configures_itself_without_a_thunder_release() -> None:
    # The whole point: an application Thunder has never heard of — including
    # one that does not exist yet — is expressible today.
    future = Config.standard().with_scheme("nobody-shipped-this-yet").with_port(4242)
    assert future.scheme == "nobody-shipped-this-yet"
    assert future.default_port == 4242
    # …and it inherits every standard behavior it did not override.
    assert future.handshake is Config.standard().handshake
    assert future.error_codes is Config.standard().error_codes


def test_overrides_compose_and_leave_the_rest_standard() -> None:
    # A deployment that still diverges says so in its own repository.
    diverging = (
        Config.standard()
        .with_scheme("legacy")
        .with_port(15501)
        .with_handshake(Handshake.AUTH_COMMAND)
        .with_hello_style(HelloStyle.NOT_USED)
        .with_push(PushPolicy.ENABLED)
        .with_max_frame_bytes(512 * 1024 * 1024)
        .with_error_codes(ErrorConvention.RESP3_PREFIXES)
    )

    assert diverging.handshake is Handshake.AUTH_COMMAND
    assert diverging.push is PushPolicy.ENABLED
    assert diverging.max_frame_bytes == 512 * 1024 * 1024
    # Untouched dimensions stay standard — convergence is "delete overrides
    # until only identity remains".
    assert diverging.max_in_flight == Config.standard().max_in_flight
    assert diverging.tls is Config.standard().tls


def test_a_config_is_still_a_plain_dataclass() -> None:
    # Configs are data (PRO-003): direct construction must keep working, so
    # nothing forces an application through the builder.
    literal = Config(
        scheme="plain",
        default_port=1,
        handshake=Handshake.NONE,
        hello_style=HelloStyle.NOT_USED,
        push=PushPolicy.RESERVED,
        max_frame_bytes=1024,
        max_in_flight=2,
        error_codes=ErrorConvention.NONE,
        tls=TlsPolicy.OFF,
    )
    assert literal.scheme == "plain"


def test_the_builder_never_mutates_the_receiver() -> None:
    # Frozen dataclass + `replace` semantics: every `with_*` returns a NEW
    # config, so a shared standard can never be edited out from under a
    # caller.
    standard = Config.standard()
    derived = standard.with_scheme("myapp")
    assert standard.scheme == ""
    assert derived.scheme == "myapp"
