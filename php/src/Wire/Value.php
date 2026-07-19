<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * A Thunder value: exactly the eight variants of WIRE-002.
 *
 * ## Why this is not just a PHP value
 *
 * PHP has one string type, used for both text and binary. WIRE-015 forbids
 * exactly that conflation — `Bytes` must never smuggle text and `Str` must
 * never smuggle binary — and the two encode to different MessagePack families
 * (`bin` vs `str`), which the corpus pins byte for byte. A design that inferred
 * the variant from the PHP value could not tell them apart, so the variant is
 * carried explicitly and the constructor is private: a `Value` can only be made
 * through a factory that names the variant.
 *
 * The same reasoning applies to `Map`, which is an **ordered list of pairs**
 * with keys of any type (WIRE-002). A PHP array would silently reorder nothing
 * but would restrict keys to int|string and collapse duplicates — so pairs are
 * a list of {@see MapEntry}, not an associative array.
 *
 * @psalm-immutable
 */
final class Value
{
    /**
     * @param list<Value>    $array
     * @param list<MapEntry> $map
     */
    private function __construct(
        public readonly Kind $kind,
        private readonly bool $bool = false,
        private readonly int $int = 0,
        private readonly float $float = 0.0,
        private readonly string $string = '',
        private readonly array $array = [],
        private readonly array $map = [],
    ) {
    }

    public static function null(): self
    {
        return new self(Kind::Null);
    }

    public static function bool(bool $value): self
    {
        return new self(Kind::Bool, bool: $value);
    }

    public static function int(int $value): self
    {
        return new self(Kind::Int, int: $value);
    }

    public static function float(float $value): self
    {
        return new self(Kind::Float, float: $value);
    }

    /** Binary data. Encoded as MessagePack `bin` (WIRE-010). */
    public static function bytes(string $value): self
    {
        return new self(Kind::Bytes, string: $value);
    }

    /** UTF-8 text. Encoded as the MessagePack `str` family (WIRE-015). */
    public static function str(string $value): self
    {
        return new self(Kind::Str, string: $value);
    }

    /** @param list<Value> $items */
    public static function array(array $items): self
    {
        return new self(Kind::Array, array: array_values($items));
    }

    /** @param list<MapEntry> $pairs */
    public static function map(array $pairs): self
    {
        return new self(Kind::Map, map: array_values($pairs));
    }

    /**
     * Build a map from a PHP associative array, for the common case of string
     * keys. Insertion order is preserved, which the HELLO payload depends on.
     *
     * @param array<string, Value> $pairs
     */
    public static function mapOf(array $pairs): self
    {
        $entries = [];
        foreach ($pairs as $key => $value) {
            $entries[] = new MapEntry(self::str((string) $key), $value);
        }

        return new self(Kind::Map, map: $entries);
    }

    public function isNull(): bool
    {
        return Kind::Null === $this->kind;
    }

    /**
     * The accessors below return null rather than throwing on a kind mismatch.
     * Decoded values come off a socket, so "wrong kind" is ordinary input, not
     * a programming error, and forcing every read into a try/catch would push
     * callers toward skipping the check entirely.
     */
    public function asBool(): ?bool
    {
        return Kind::Bool === $this->kind ? $this->bool : null;
    }

    public function asInt(): ?int
    {
        return Kind::Int === $this->kind ? $this->int : null;
    }

    /** An `Int` widens to float here, as it does in every other lane. */
    public function asFloat(): ?float
    {
        return match ($this->kind) {
            Kind::Float => $this->float,
            Kind::Int => (float) $this->int,
            default => null,
        };
    }

    /** A `Str` also reads as bytes (its UTF-8 encoding); the reverse does not hold. */
    public function asBytes(): ?string
    {
        return match ($this->kind) {
            Kind::Bytes, Kind::Str => $this->string,
            default => null,
        };
    }

    public function asStr(): ?string
    {
        return Kind::Str === $this->kind ? $this->string : null;
    }

    /** @return list<Value>|null */
    public function asArray(): ?array
    {
        return Kind::Array === $this->kind ? $this->array : null;
    }

    /** @return list<MapEntry>|null */
    public function asMap(): ?array
    {
        return Kind::Map === $this->kind ? $this->map : null;
    }

    /** First value under a `Str` key, or null. Map keys may repeat, so first wins. */
    public function mapGet(string $key): ?Value
    {
        if (Kind::Map !== $this->kind) {
            return null;
        }
        foreach ($this->map as $entry) {
            if ($key === $entry->key->asStr()) {
                return $entry->value;
            }
        }

        return null;
    }

    /**
     * Structural equality.
     *
     * Floats compare **by bit pattern**, not by `==`. This is load-bearing for
     * the corpus: NaN never equals itself numerically, so a round-trip of the
     * NaN vector would always fail, and `-0.0 == 0.0` is true in PHP, which
     * would hide a lost sign bit — the exact drift WIRE-014 pins.
     */
    public function equals(self $other): bool
    {
        if ($this->kind !== $other->kind) {
            return false;
        }

        switch ($this->kind) {
            case Kind::Null:
                return true;
            case Kind::Bool:
                return $this->bool === $other->bool;
            case Kind::Int:
                return $this->int === $other->int;
            case Kind::Float:
                return pack('E', $this->float) === pack('E', $other->float);
            case Kind::Bytes:
            case Kind::Str:
                return $this->string === $other->string;
            case Kind::Array:
                if (count($this->array) !== count($other->array)) {
                    return false;
                }
                foreach ($this->array as $i => $item) {
                    if (!$item->equals($other->array[$i])) {
                        return false;
                    }
                }

                return true;
            case Kind::Map:
                if (count($this->map) !== count($other->map)) {
                    return false;
                }
                foreach ($this->map as $i => $entry) {
                    $peer = $other->map[$i];
                    if (!$entry->key->equals($peer->key) || !$entry->value->equals($peer->value)) {
                        return false;
                    }
                }

                return true;
        }
    }
}
