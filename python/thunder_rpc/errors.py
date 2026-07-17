"""Typed client errors (CLT-050..052) plus the wire-level frame errors.

``Result::Err(string)`` replies are parsed per the profile's ``error_codes``
convention (PRO-014) by :func:`from_server_message` into a typed error
carrying the raw message, an optional machine-readable ``code`` (from a
leading ``"[code] "`` prefix), and a stable error **class**. Product SDKs
and user code branch on the class and ``code``, never on message text
(CLT-052).

Class map (mirrors ``thunder-client``'s ``ClientError``):

- :class:`AuthError` — handshake rejections and ``NOAUTH``/``WRONGPASS``/
  ``NOPERM``-prefixed replies (CLT-003/051)
- :class:`ServerError` — the server answered with ``Err`` (raw message +
  optional bracket ``code``)
- :class:`ConnectionError` — dial/write failures, dead connections, invalid
  endpoints (CLT-004/030/031/070)
- :class:`TimeoutError` — the per-call or connect timeout elapsed (CLT-020)
- :class:`FrameTooLargeError` — a frame exceeded the cap (WIRE-020/021)
- :class:`DecodeError` — malformed frame, or a push frame under a
  ``Reserved`` profile (WIRE-023, CLT-060)

Note: ``ConnectionError`` and ``TimeoutError`` deliberately shadow the
builtins *inside this namespace only* — catch them as
``thunder_rpc.errors.ConnectionError`` / ``errors.TimeoutError``.
"""

from __future__ import annotations

from .profile import ErrorConvention


class ThunderError(Exception):
    """Base class of every typed Thunder client error."""

    def __init__(self, message: str) -> None:
        super().__init__(message)
        #: Raw message, verbatim.
        self.message = message


class AuthError(ThunderError):
    """Authentication / authorization failure — handshake rejections
    (CLT-003) and ``NOAUTH``/``WRONGPASS``/``NOPERM``-prefixed replies
    (CLT-051)."""


class ServerError(ThunderError):
    """The server answered the call with ``Result::Err``."""

    def __init__(self, message: str, code: str | None = None) -> None:
        super().__init__(message)
        #: Machine-readable code extracted from a leading ``"[code] "``
        #: prefix under ``BRACKET_CODE`` / ``BOTH`` conventions (PRO-014).
        self.code = code


class ConnectionError(ThunderError):  # noqa: A001 - deliberate, namespaced
    """Transport-level failure: dial, write, or the connection dying while
    the call was pending (CLT-004/030/031). Also raised for invalid
    endpoints (CLT-070)."""


class TimeoutError(ThunderError):  # noqa: A001 - deliberate, namespaced
    """The per-call (or connect) timeout elapsed (CLT-020). The pending
    entry was removed; a late response is dropped per CLT-013."""

    def __init__(self, message: str = "timed out") -> None:
        super().__init__(message)


class FrameTooLargeError(ThunderError):
    """A frame larger than the cap (WIRE-020/021) — raised from the length
    prefix alone, before any body allocation."""

    def __init__(
        self, message: str, *, body: int | None = None, limit: int | None = None
    ) -> None:
        super().__init__(message)
        #: Declared body size from the length prefix, when known.
        self.body = body
        #: The cap that was exceeded, when known.
        self.limit = limit


class DecodeError(ThunderError):
    """Malformed frame body (WIRE-023), or a push frame received under a
    ``Reserved`` profile (CLT-060)."""


def frame_too_large(body: int, limit: int) -> FrameTooLargeError:
    """Build the typed cap error with the family-pinned message shape."""
    return FrameTooLargeError(
        f"frame body {body} bytes exceeds limit {limit} bytes", body=body, limit=limit
    )


def from_server_message(message: str, convention: ErrorConvention) -> ThunderError:
    """Parse a server error string per the profile's convention
    (CLT-050, PRO-014).

    - ``RESP3_PREFIXES``: ``NOAUTH``/``WRONGPASS``/``NOPERM`` →
      :class:`AuthError`; everything else (``ERR ...`` included) →
      :class:`ServerError`.
    - ``BRACKET_CODE``: a leading ``"[code] "`` is extracted into ``code``;
      the auth prefixes still map to :class:`AuthError` regardless of
      convention (CLT-051).
    - ``BOTH``: composes the two — bracket code first, then prefixes.
    - ``NONE``: no parsing; the raw message becomes :class:`ServerError`.

    ``message`` always carries the raw string, verbatim.
    """
    if convention is ErrorConvention.NONE:
        return ServerError(message)
    if convention is ErrorConvention.RESP3_PREFIXES:
        if _starts_with_auth_prefix(message):
            return AuthError(message)
        return ServerError(message)
    # BRACKET_CODE | BOTH
    code, rest = _split_bracket_code(message)
    if _starts_with_auth_prefix(rest):
        return AuthError(message)
    return ServerError(message, code)


_AUTH_PREFIXES = ("NOAUTH", "WRONGPASS", "NOPERM")


def _starts_with_auth_prefix(message: str) -> bool:
    """True when the message starts with one of the auth prefixes both
    family conventions use for authentication failures (CLT-051)."""
    for prefix in _AUTH_PREFIXES:
        if message.startswith(prefix):
            rest = message[len(prefix) :]
            if rest == "" or rest.startswith(" "):
                return True
    return False


def _split_bracket_code(message: str) -> tuple[str | None, str]:
    """Split a leading ``"[code] "`` prefix. The code must be non-empty and
    whitespace-free (machine-readable, Vectorizer-style); anything else
    leaves the message untouched."""
    if message.startswith("["):
        inner = message[1:]
        end = inner.find("]")
        if end != -1:
            code = inner[:end]
            after = inner[end + 1 :]
            if code and not any(ch.isspace() for ch in code) and after.startswith(" "):
                return code, after[1:]
    return None, message
