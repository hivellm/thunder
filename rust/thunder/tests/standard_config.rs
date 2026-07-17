//! Pins `Config::standard()` to `conformance/standard.yaml` (PRO-013).
//!
//! Thunder ships **one** standard and no product knowledge, so this is the
//! whole registry check: a change to the standard that is not mirrored in
//! the language-neutral YAML — or vice versa — fails here, in all four
//! languages. That cross-language agreement was the only job the old
//! per-product registry legitimately did; it survives without any product
//! name.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use serde::Deserialize;
use thunder::wire::config::{
    Config, ErrorConvention, Handshake, HelloStyle, PushPolicy, TlsPolicy,
};

#[derive(Deserialize)]
struct Yaml {
    handshake: String,
    hello_style: String,
    push: String,
    max_frame_bytes: usize,
    max_in_flight: usize,
    error_codes: String,
    tls: String,
}

fn standard_yaml() -> Yaml {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/standard.yaml");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} must exist: {e}", path.display()));
    serde_yaml::from_str(&raw).unwrap()
}

#[test]
fn standard_matches_the_conformance_data_file() {
    let y = standard_yaml();
    let s = Config::standard();

    let handshake = match y.handshake.as_str() {
        "none" => Handshake::None,
        "auth_command" => Handshake::AuthCommand,
        "hello_mandatory" => Handshake::HelloMandatory,
        other => panic!("unknown handshake {other}"),
    };
    assert_eq!(handshake, s.handshake);

    let hello = match y.hello_style.as_str() {
        "not_used" => HelloStyle::NotUsed,
        "arg_less" => HelloStyle::ArgLess,
        "map_payload" => HelloStyle::MapPayload,
        other => panic!("unknown hello_style {other}"),
    };
    assert_eq!(hello, s.hello_style);

    let push = match y.push.as_str() {
        "reserved" => PushPolicy::Reserved,
        "enabled" => PushPolicy::Enabled,
        other => panic!("unknown push {other}"),
    };
    assert_eq!(push, s.push);

    assert_eq!(y.max_frame_bytes, s.max_frame_bytes);
    assert_eq!(y.max_in_flight, s.max_in_flight);

    let errors = match y.error_codes.as_str() {
        "none" => ErrorConvention::None,
        "resp3_prefixes" => ErrorConvention::Resp3Prefixes,
        "bracket_code" => ErrorConvention::BracketCode,
        "both" => ErrorConvention::Both,
        other => panic!("unknown error_codes {other}"),
    };
    assert_eq!(errors, s.error_codes);

    let tls = match y.tls.as_str() {
        "off" => TlsPolicy::Off,
        "optional_rustls" => TlsPolicy::Optional,
        "reserved_config" => TlsPolicy::Reserved,
        other => panic!("unknown tls {other}"),
    };
    assert_eq!(tls, s.tls);
}

#[test]
fn default_is_the_standard() {
    assert_eq!(Config::default(), Config::standard());
}

#[test]
fn the_standard_carries_no_identity() {
    // Identity is the application's: Thunder has no opinion about which
    // scheme or port an implementation answers on.
    let s = Config::standard();
    assert_eq!(s.scheme, "");
    assert_eq!(s.default_port, 0);
}

#[test]
fn an_application_configures_itself_without_a_thunder_release() {
    // The whole point: a product Thunder has never heard of — including
    // one that does not exist yet — is expressible today.
    let future = Config::standard()
        .scheme("nobody-shipped-this-yet")
        .port(4242);
    assert_eq!(future.scheme, "nobody-shipped-this-yet");
    assert_eq!(future.default_port, 4242);
    // …and it inherits every standard behavior it did not override.
    assert_eq!(future.handshake, Config::standard().handshake);
    assert_eq!(future.error_codes, Config::standard().error_codes);
}

#[test]
fn overrides_compose_and_leave_the_rest_standard() {
    // A deployment that still diverges says so in its own repository.
    let diverging = Config::standard()
        .scheme("legacy")
        .port(15501)
        .handshake(Handshake::AuthCommand)
        .hello_style(HelloStyle::NotUsed)
        .push(PushPolicy::Enabled)
        .max_frame_bytes(512 * 1024 * 1024)
        .error_codes(ErrorConvention::Resp3Prefixes);

    assert_eq!(diverging.handshake, Handshake::AuthCommand);
    assert_eq!(diverging.push, PushPolicy::Enabled);
    assert_eq!(diverging.max_frame_bytes, 512 * 1024 * 1024);
    // Untouched dimensions stay standard — convergence is "delete
    // overrides until only identity remains".
    assert_eq!(diverging.max_in_flight, Config::standard().max_in_flight);
    assert_eq!(diverging.tls, Config::standard().tls);
}

#[test]
fn a_config_is_still_a_plain_struct() {
    // Configs are data (PRO-003): struct construction must keep working,
    // so nothing forces an application through the builder.
    let literal = Config {
        scheme: "plain",
        default_port: 1,
        handshake: Handshake::None,
        hello_style: HelloStyle::NotUsed,
        push: PushPolicy::Reserved,
        max_frame_bytes: 1024,
        max_in_flight: 2,
        error_codes: ErrorConvention::None,
        tls: TlsPolicy::Off,
    };
    assert_eq!(literal.scheme, "plain");
}
