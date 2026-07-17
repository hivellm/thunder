## 1. Implementation
- [ ] 1.1 rust/thunder-server: add a feature-gated `tokio-rustls` accept path — when `tls.cert_path`/`tls.key_path` are set, wrap the accepted stream in a `TlsAcceptor`; plaintext path untouched when unset; no STARTTLS (SRV-040)
- [ ] 1.2 rust/thunder-server: config surface for cert/key paths (+ optional client-CA for mTLS-later), off by default; error cleanly on misconfig (cert missing/unreadable)
- [ ] 1.3 rust/thunder-client: optional TLS connector (rustls with native/webpki roots or a configured CA), opt-in via endpoint/profile/client config; plaintext default; TLS/handshake failures classify as Connection errors (FR-29)
- [ ] 1.4 typescript client: TLS connect option via Node `tls.connect` gated by client config, off by default; mirror the Rust knobs + Connection error mapping
- [ ] 1.5 python client (sync + async): TLS via `ssl.SSLContext` gated by config, off by default, both clients identical
- [ ] 1.6 csharp client: TLS via `SslStream` gated by config, off by default
- [ ] 1.7 Docs: SPEC-004/SPEC-008 TLS notes finalized; each package README gains a short "enabling TLS" section; state clearly it is off by default and opt-in

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — README + spec docs cover enabling TLS at both ends and the off-by-default posture
- [ ] 2.2 Write tests covering the new behavior — a self-signed round-trip per language proves encrypted req/resp; a plaintext-still-works test proves the default is unchanged; a cert-mismatch test proves a Connection-class error
- [ ] 2.3 Run tests and confirm they pass — full gate green per language with the TLS feature/dep present AND absent (default build stays plaintext-only)
