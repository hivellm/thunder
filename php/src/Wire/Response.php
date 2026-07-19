<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * A Thunder response body: `[id, result]` where result is the externally-tagged
 * `{"Ok": <value>}` or `{"Err": <string>}` of WIRE-003.
 *
 * The success and failure cases are one type with a discriminant rather than
 * two classes, because they are two arms of one wire shape — a decoder must
 * produce whichever arrived, and a caller must always ask which it got.
 *
 * @psalm-immutable
 */
final class Response
{
    private function __construct(
        public readonly int $id,
        public readonly bool $isErr,
        public readonly Value $ok,
        public readonly string $err,
    ) {
    }

    public static function ok(int $id, Value $value): self
    {
        return new self($id, false, $value, '');
    }

    /**
     * Errors are strings on the wire and Thunder preserves them verbatim
     * (WIRE-040) — interpreting a `"[code] message"` prefix is a client
     * concern driven by the profile, not something the wire layer may do.
     */
    public static function err(int $id, string $message): self
    {
        return new self($id, true, Value::null(), $message);
    }
}
