<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * Length-prefixed framing: `u32 LE` body length, then the MessagePack body.
 *
 * Pure by WIRE-030 — this operates on byte strings and knows nothing about
 * sockets, timers or products.
 *
 * ## The cap is checked before the body exists
 *
 * WIRE-020/021 require the frame cap to be validated against the length prefix
 * **before any body allocation**, and the corpus proves it with a vector that
 * supplies only a 4-byte header claiming 64 MiB + 1: a decoder that waits for
 * the body before judging the size will hang on that vector instead of
 * rejecting it. So the size check happens on the prefix alone, before the
 * "do I have the whole body yet?" question is even asked.
 */
final class Frame
{
    public const WIRE_VERSION = 1;

    /** Reserved for server-initiated frames (WIRE-005). Never a request id. */
    public const PUSH_ID = 0xFFFFFFFF;

    public const DEFAULT_MAX_FRAME_BYTES = 64 * 1024 * 1024;

    public const PREFIX_BYTES = 4;

    /**
     * Wrap an encoded body in its length prefix.
     *
     * The cap applies on encode too (WIRE-020), so a local bug produces a local
     * error instead of a frame the peer must reject.
     */
    public static function encode(string $body, int $max = self::DEFAULT_MAX_FRAME_BYTES): string
    {
        $length = strlen($body);
        if ($length > $max) {
            throw new FrameTooLargeException($length, $max);
        }

        return pack('V', $length) . $body;
    }

    /**
     * Split one frame off the front of a buffer without copying the body.
     *
     * Returns `[body, consumed]`, or `null` when the buffer does not yet hold a
     * complete frame — "need more bytes" is a normal outcome and not an error
     * (WIRE-022). A zero-length frame is *valid* here and comes back as an
     * empty body: it is the keep-alive tick of WIRE-024, and only the typed
     * decoders below have reason to object to it.
     *
     * @return array{0: string, 1: int}|null
     */
    public static function trySplit(string $buffer, int $max = self::DEFAULT_MAX_FRAME_BYTES): ?array
    {
        if (strlen($buffer) < self::PREFIX_BYTES) {
            return null;
        }

        /** @var array{1: int} $unpacked */
        $unpacked = unpack('V', substr($buffer, 0, self::PREFIX_BYTES));
        $length = $unpacked[1];

        // Before anything else, and specifically before asking whether the body
        // has arrived — see the class note.
        if ($length > $max) {
            throw new FrameTooLargeException($length, $max);
        }

        $total = self::PREFIX_BYTES + $length;
        if (strlen($buffer) < $total) {
            return null;
        }

        return [substr($buffer, self::PREFIX_BYTES, $length), $total];
    }

    /**
     * Decode one request from the front of a buffer.
     *
     * @return array{0: Request, 1: int}|null null when more bytes are needed
     */
    public static function decodeRequest(
        string $buffer,
        int $max = self::DEFAULT_MAX_FRAME_BYTES,
        ?Codec $codec = null,
    ): ?array {
        $split = self::trySplit($buffer, $max);
        if (null === $split) {
            return null;
        }
        [$body, $consumed] = $split;
        self::rejectEmptyBody($body);

        return [($codec ?? new Codec())->decodeRequestBody($body), $consumed];
    }

    /**
     * Decode one response from the front of a buffer.
     *
     * @return array{0: Response, 1: int}|null null when more bytes are needed
     */
    public static function decodeResponse(
        string $buffer,
        int $max = self::DEFAULT_MAX_FRAME_BYTES,
        ?Codec $codec = null,
    ): ?array {
        $split = self::trySplit($buffer, $max);
        if (null === $split) {
            return null;
        }
        [$body, $consumed] = $split;
        self::rejectEmptyBody($body);

        return [($codec ?? new Codec())->decodeResponseBody($body), $consumed];
    }

    /**
     * WIRE-024: a zero-length frame is a valid keep-alive, but a typed decode
     * promised a message and there is none — so it fails with an error that is
     * *distinct* from the malformed-body error of WIRE-023. A caller that wants
     * to treat liveness ticks as ordinary traffic catches this one type;
     * treating them as corruption would drop healthy connections.
     */
    private static function rejectEmptyBody(string $body): void
    {
        if ('' === $body) {
            throw new KeepAliveException();
        }
    }
}
