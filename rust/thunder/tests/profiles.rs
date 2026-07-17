//! Pins the Rust `Profile` registry constants to the language-neutral data
//! files in `conformance/profiles/` (PRO-010/013): a registry edit that is
//! not mirrored in the YAML — or vice versa — fails here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use serde::Deserialize;
use thunder::wire::profile::{
    ErrorConvention, Handshake, HelloStyle, Profile, PushPolicy, TlsPolicy,
};

#[derive(Deserialize)]
struct Yaml {
    name: String,
    scheme: String,
    default_port: u16,
    handshake: String,
    hello_style: Option<String>,
    push: String,
    max_frame_bytes: usize,
    max_in_flight: usize,
    error_codes: String,
    tls: String,
}

#[test]
fn registry_constants_match_conformance_profiles() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/profiles");
    let mut matched = 0usize;
    for p in Profile::registry() {
        let raw = std::fs::read_to_string(dir.join(format!("{}.yaml", p.name)))
            .unwrap_or_else(|e| panic!("profile yaml for {} must exist: {e}", p.name));
        let y: Yaml = serde_yaml::from_str(&raw).unwrap();
        assert_eq!(y.name, p.name);
        assert_eq!(y.scheme, p.scheme, "{}", p.name);
        assert_eq!(y.default_port, p.default_port, "{}", p.name);
        assert_eq!(y.max_frame_bytes, p.max_frame_bytes, "{}", p.name);
        assert_eq!(y.max_in_flight, p.max_in_flight, "{}", p.name);
        let handshake = match y.handshake.as_str() {
            "none" => Handshake::None,
            "auth_command" => Handshake::AuthCommand,
            "hello_mandatory" => Handshake::HelloMandatory,
            other => panic!("unknown handshake {other}"),
        };
        assert_eq!(handshake, p.handshake, "{}", p.name);
        let hello = match y.hello_style.as_deref() {
            None => HelloStyle::NotUsed,
            Some("positional_version") => HelloStyle::PositionalVersion,
            Some("map_payload") => HelloStyle::MapPayload,
            Some(other) => panic!("unknown hello_style {other}"),
        };
        assert_eq!(hello, p.hello_style, "{}", p.name);
        let push = match y.push.as_str() {
            "reserved" => PushPolicy::Reserved,
            "enabled" => PushPolicy::Enabled,
            other => panic!("unknown push {other}"),
        };
        assert_eq!(push, p.push, "{}", p.name);
        let errors = match y.error_codes.as_str() {
            "none" => ErrorConvention::None,
            "resp3_prefixes" => ErrorConvention::Resp3Prefixes,
            "bracket_code" => ErrorConvention::BracketCode,
            "both" => ErrorConvention::Both,
            other => panic!("unknown error_codes {other}"),
        };
        assert_eq!(errors, p.error_codes, "{}", p.name);
        let tls = match y.tls.as_str() {
            "off" => TlsPolicy::Off,
            "optional_rustls" => TlsPolicy::Optional,
            "reserved_config" => TlsPolicy::Reserved,
            other => panic!("unknown tls {other}"),
        };
        assert_eq!(tls, p.tls, "{}", p.name);
        matched += 1;
    }
    assert_eq!(matched, 4, "all four family profiles pinned");
}
