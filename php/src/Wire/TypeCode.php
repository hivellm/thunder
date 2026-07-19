<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * The MessagePack type codes this layer needs to *dispatch* on.
 *
 * This is deliberately not a codec. WIRE-031 forbids hand-rolled MessagePack,
 * and none is written here: `rybakit/msgpack` does every encode and decode. But
 * a decoder still has to know which library method to call, and two of
 * Thunder's rules make that unavoidable:
 *
 * - `bin` and `str` both unpack to a PHP `string`, so the generic `unpack()`
 *   erases the difference between `Bytes` and `Str` — the distinction WIRE-015
 *   exists to protect.
 * - An externally-tagged value is either a bare string (`"Null"`) or a
 *   single-key map, and WIRE-011 additionally tolerates `Bytes` arriving as an
 *   array of integers. Telling those apart means looking at the type byte.
 *
 * So this reads the discriminant and nothing else; the payload is always handed
 * to the library.
 */
final class TypeCode
{
    public const NIL = 0xc0;

    public const FALSE = 0xc2;
    public const TRUE = 0xc3;

    public const BIN8 = 0xc4;
    public const BIN16 = 0xc5;
    public const BIN32 = 0xc6;

    public const FLOAT32 = 0xca;
    public const FLOAT64 = 0xcb;

    public const UINT8 = 0xcc;
    public const UINT64 = 0xcf;
    public const INT8 = 0xd0;
    public const INT64 = 0xd3;

    public const STR8 = 0xd9;
    public const STR16 = 0xda;
    public const STR32 = 0xdb;

    public const ARRAY16 = 0xdc;
    public const ARRAY32 = 0xdd;

    public const MAP16 = 0xde;
    public const MAP32 = 0xdf;

    public static function isFixInt(int $code): bool
    {
        return $code <= 0x7f || $code >= 0xe0;
    }

    public static function isFixStr(int $code): bool
    {
        return $code >= 0xa0 && $code <= 0xbf;
    }

    public static function isFixArray(int $code): bool
    {
        return $code >= 0x90 && $code <= 0x9f;
    }

    public static function isFixMap(int $code): bool
    {
        return $code >= 0x80 && $code <= 0x8f;
    }

    public static function isStr(int $code): bool
    {
        return self::isFixStr($code)
            || self::STR8 === $code || self::STR16 === $code || self::STR32 === $code;
    }

    public static function isBin(int $code): bool
    {
        return self::BIN8 === $code || self::BIN16 === $code || self::BIN32 === $code;
    }

    public static function isArray(int $code): bool
    {
        return self::isFixArray($code) || self::ARRAY16 === $code || self::ARRAY32 === $code;
    }

    public static function isMap(int $code): bool
    {
        return self::isFixMap($code) || self::MAP16 === $code || self::MAP32 === $code;
    }

    public static function isInt(int $code): bool
    {
        return self::isFixInt($code)
            || ($code >= self::UINT8 && $code <= self::UINT64)
            || ($code >= self::INT8 && $code <= self::INT64);
    }

    public static function isFloat(int $code): bool
    {
        return self::FLOAT32 === $code || self::FLOAT64 === $code;
    }

    public static function isBool(int $code): bool
    {
        return self::TRUE === $code || self::FALSE === $code;
    }
}
