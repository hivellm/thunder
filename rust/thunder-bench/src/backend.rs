//! The one no-op dispatch backend every lane serves (BEN-001).
//!
//! Zero storage, zero business logic — the engine must never be in the
//! measurement. Mode selection is by command name (byte-exact
//! pass-through, SRV-022):
//!
//! | Command | Mode | Reply |
//! |---|---|---|
//! | `ECHO` / `PING` | echo | `args[0]`, or `"PONG"` when bare |
//! | `STATIC` | static-reply | a fixed [`STATIC_REPLY_BYTES`] string |
//! | `SINK` | sink | arguments dropped, `Null` |
//!
//! The same [`NoopBackend::respond`] surface backs both the Thunder RPC
//! listener (through [`Dispatch`]) and the HTTP lane, so the two lanes can
//! never diverge on semantics.

use thunder::server::{AuthError, Credentials, Dispatch, Principal, Session};
use thunder::wire::Value;

/// Size of the fixed `STATIC` reply — the medium-4KiB scenario's payload.
pub const STATIC_REPLY_BYTES: usize = 4096;

/// The shared no-op backend (BEN-001). Cheap to build, trivially `Sync`;
/// the 4 KiB static reply is precomputed once.
#[derive(Debug)]
pub struct NoopBackend {
    static_reply: String,
}

impl NoopBackend {
    /// Build the backend with its precomputed 4 KiB static reply.
    pub fn new() -> Self {
        Self {
            static_reply: "x".repeat(STATIC_REPLY_BYTES),
        }
    }

    /// The one dispatch surface every listener shares. `args` is taken by
    /// value so echo moves the payload out instead of cloning it — the
    /// backend must stay out of the measurement.
    pub fn respond(&self, command: &str, mut args: Vec<Value>) -> Result<Value, String> {
        match command {
            "ECHO" | "PING" => Ok(if args.is_empty() {
                Value::Str("PONG".to_owned())
            } else {
                args.swap_remove(0)
            }),
            "STATIC" => Ok(Value::Str(self.static_reply.clone())),
            "SINK" => {
                drop(args);
                Ok(Value::Null)
            }
            other => Err(format!("ERR unknown command '{other}'")),
        }
    }
}

impl Default for NoopBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Dispatch for NoopBackend {
    type Identity = ();

    async fn dispatch(
        &self,
        _session: &Session,
        command: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        self.respond(command, args)
    }

    /// The bench profile uses `Handshake::None`, so this hook is never on
    /// the measured path; it accepts everything for completeness.
    async fn authenticate(&self, _creds: Credentials) -> Result<Principal, AuthError> {
        Ok(Principal::new("bench".to_owned()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn echo_returns_first_arg() {
        let backend = NoopBackend::new();
        let reply = backend
            .respond("ECHO", vec![Value::Str("hello".to_owned())])
            .unwrap();
        assert_eq!(reply, Value::Str("hello".to_owned()));
    }

    #[test]
    fn bare_echo_and_ping_return_pong() {
        let backend = NoopBackend::new();
        for command in ["ECHO", "PING"] {
            let reply = backend.respond(command, vec![]).unwrap();
            assert_eq!(reply, Value::Str("PONG".to_owned()));
        }
    }

    #[test]
    fn static_reply_is_exactly_4096_bytes() {
        let backend = NoopBackend::new();
        let reply = backend.respond("STATIC", vec![]).unwrap();
        match reply {
            Value::Str(s) => assert_eq!(s.len(), STATIC_REPLY_BYTES),
            other => panic!("expected Str, got {other:?}"),
        }
    }

    #[test]
    fn sink_discards_args_and_returns_null() {
        let backend = NoopBackend::new();
        let reply = backend
            .respond("SINK", vec![Value::bytes(vec![0u8; 1024])])
            .unwrap();
        assert_eq!(reply, Value::Null);
    }

    #[test]
    fn unknown_command_is_a_resp_style_error() {
        let backend = NoopBackend::new();
        let err = backend.respond("NOPE", vec![]).unwrap_err();
        assert_eq!(err, "ERR unknown command 'NOPE'");
    }
}
