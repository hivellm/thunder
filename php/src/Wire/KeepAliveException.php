<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * A zero-length frame reached a typed decode (WIRE-024).
 *
 * The frame itself is **valid** — it is the liveness tick, and a raw split
 * returns it as an empty body. It only fails here because a typed decode
 * promised a `Request` or `Response` and there is no message to produce.
 *
 * It extends {@see DecodeException} so the corpus's `decode`-class reject
 * vector is satisfied and existing catch sites keep working, while staying
 * distinct for a caller that wants to treat keep-alives as ordinary traffic —
 * treating them as corruption would drop healthy connections.
 */
final class KeepAliveException extends DecodeException
{
    public function __construct()
    {
        parent::__construct('zero-length frame: a keep-alive tick carries no message body');
    }
}
