<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * A frame's declared length exceeds the cap (WIRE-020/021).
 *
 * Thrown from the length prefix alone, before the body is read or any buffer
 * the size of the claim is allocated — that is the whole point of the
 * requirement, and the `framing-cap-plus-one` vector proves it by supplying
 * only a 4-byte header: a decoder that waits for the body before judging the
 * size hangs on that vector instead of rejecting it.
 */
final class FrameTooLargeException extends WireException
{
    public function __construct(
        public readonly int $body,
        public readonly int $limit,
    ) {
        parent::__construct("frame body {$body} bytes exceeds limit {$limit} bytes");
    }
}
