<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * The eight value variants of WIRE-002, and nothing else.
 *
 * The string values are the corpus's node type names, so a vector's
 * `{type: bytes}` maps straight onto a case without a translation table that
 * could drift from the YAML.
 */
enum Kind: string
{
    case Null = 'null';
    case Bool = 'bool';
    case Int = 'int';
    case Float = 'float';
    case Bytes = 'bytes';
    case Str = 'str';
    case Array = 'array';
    case Map = 'map';

    /**
     * The externally-tagged name this variant carries on the wire (WIRE-003) —
     * capitalised, because the tag is the Rust enum variant name and the corpus
     * pins those bytes.
     */
    public function tag(): string
    {
        return match ($this) {
            self::Null => 'Null',
            self::Bool => 'Bool',
            self::Int => 'Int',
            self::Float => 'Float',
            self::Bytes => 'Bytes',
            self::Str => 'Str',
            self::Array => 'Array',
            self::Map => 'Map',
        };
    }

    /** The variant a wire tag names, or null if the tag is not one of the eight. */
    public static function fromTag(string $tag): ?self
    {
        return match ($tag) {
            'Null' => self::Null,
            'Bool' => self::Bool,
            'Int' => self::Int,
            'Float' => self::Float,
            'Bytes' => self::Bytes,
            'Str' => self::Str,
            'Array' => self::Array,
            'Map' => self::Map,
            default => null,
        };
    }
}
