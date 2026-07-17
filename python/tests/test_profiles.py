"""Pins the Python ``Profile`` registry constants to the language-neutral
data files in ``conformance/profiles/`` (PRO-010/013): a registry edit that
is not mirrored in the YAML — or vice versa — fails here (mirrors
``rust/thunder-wire/tests/profiles.rs``)."""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

from thunder_rpc import (
    LEXUM,
    NEXUS,
    SYNAP,
    VECTORIZER,
    ErrorConvention,
    Handshake,
    HelloStyle,
    Profile,
    Profiles,
    PushPolicy,
    TlsPolicy,
    registry,
)

PROFILE_DIR = Path(__file__).resolve().parents[2] / "conformance" / "profiles"

_HANDSHAKE = {
    "none": Handshake.NONE,
    "auth_command": Handshake.AUTH_COMMAND,
    "hello_mandatory": Handshake.HELLO_MANDATORY,
}
_HELLO_STYLE = {
    None: HelloStyle.NOT_USED,
    "positional_version": HelloStyle.POSITIONAL_VERSION,
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


def _yaml_token(raw: object) -> object:
    """PyYAML speaks YAML 1.1, where the bare scalar ``off`` parses as
    boolean False — undo that so registry tokens stay strings."""
    if raw is False:
        return "off"
    if raw is True:
        return "on"
    return raw


@pytest.mark.parametrize("profile", registry(), ids=[p.name for p in registry()])
def test_registry_constant_matches_conformance_profile(profile: Profile) -> None:
    raw = yaml.safe_load(
        (PROFILE_DIR / f"{profile.name}.yaml").read_text(encoding="utf-8")
    )
    assert raw["name"] == profile.name
    assert raw["scheme"] == profile.scheme
    assert raw["default_port"] == profile.default_port
    assert raw["max_frame_bytes"] == profile.max_frame_bytes
    assert raw["max_in_flight"] == profile.max_in_flight
    assert _HANDSHAKE[raw["handshake"]] is profile.handshake
    assert _HELLO_STYLE[raw["hello_style"]] is profile.hello_style
    assert _PUSH[raw["push"]] is profile.push
    assert _ERRORS[raw["error_codes"]] is profile.error_codes
    assert _TLS[_yaml_token(raw["tls"])] is profile.tls


def test_all_four_family_profiles_pinned() -> None:
    assert [p.name for p in registry()] == ["synap", "nexus", "vectorizer", "lexum"]
    assert Profiles.synap is SYNAP
    assert Profiles.nexus is NEXUS
    assert Profiles.vectorizer is VECTORIZER
    assert Profiles.lexum is LEXUM


def test_custom_profiles_stay_constructible() -> None:
    """PRO-020: a new product must never wait for a Thunder release."""
    custom = Profile(name="acme", scheme="acme", default_port=4242)
    assert custom.handshake is Handshake.NONE
    assert custom.max_frame_bytes == 64 * 1024 * 1024
    assert custom.max_in_flight == 256
