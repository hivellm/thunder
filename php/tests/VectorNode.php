<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Tests;

/**
 * Typed reads of the YAML the corpus loader parses.
 *
 * A parsed YAML document is `mixed` all the way down, and the tempting fix is a
 * cast at every use site. That would work right up until a vector is
 * malformed — a missing `id`, a `value` where `bits` belongs — at which point
 * PHP would quietly coerce (`(int) null` is `0`, `(string) null` is `''`) and
 * the corpus would compare against a value nobody wrote, reporting a byte
 * mismatch far from the actual mistake.
 *
 * So every read is checked, and a bad vector fails saying which field of which
 * vector is wrong.
 */
final class VectorNode
{
    /** @param array<array-key, mixed> $data */
    public function __construct(
        private readonly array $data,
        private readonly string $where,
    ) {
    }

    public function str(string $key): string
    {
        $value = $this->require($key);
        if (!is_string($value)) {
            throw new \RuntimeException("{$this->where}.{$key} must be a string, got " . get_debug_type($value));
        }

        return $value;
    }

    public function int(string $key): int
    {
        $value = $this->require($key);
        if (!is_int($value)) {
            throw new \RuntimeException("{$this->where}.{$key} must be an int, got " . get_debug_type($value));
        }

        return $value;
    }

    public function float(string $key): float
    {
        $value = $this->require($key);
        if (!is_float($value) && !is_int($value)) {
            throw new \RuntimeException("{$this->where}.{$key} must be a number, got " . get_debug_type($value));
        }

        return (float) $value;
    }

    public function bool(string $key): bool
    {
        $value = $this->require($key);
        if (!is_bool($value)) {
            throw new \RuntimeException("{$this->where}.{$key} must be a bool, got " . get_debug_type($value));
        }

        return $value;
    }

    /** A nested mapping, e.g. a value node or a `decoded` block. */
    public function node(string $key): self
    {
        $value = $this->require($key);
        if (!is_array($value)) {
            throw new \RuntimeException("{$this->where}.{$key} must be a mapping, got " . get_debug_type($value));
        }

        return new self($value, "{$this->where}.{$key}");
    }

    /**
     * A list of nested nodes, e.g. `args` or a map's pair list.
     *
     * @return list<self>
     */
    public function nodes(string $key): array
    {
        if (!$this->has($key)) {
            return [];
        }
        $value = $this->data[$key];
        if (!is_array($value)) {
            throw new \RuntimeException("{$this->where}.{$key} must be a list, got " . get_debug_type($value));
        }

        $out = [];
        foreach (array_values($value) as $i => $item) {
            if (!is_array($item)) {
                throw new \RuntimeException("{$this->where}.{$key}[{$i}] must be a mapping");
            }
            $out[] = new self($item, "{$this->where}.{$key}[{$i}]");
        }

        return $out;
    }

    /** Positional access, for a map's `[key, value]` pair. */
    public function at(int $index): self
    {
        $value = $this->data[$index] ?? null;
        if (!is_array($value)) {
            throw new \RuntimeException("{$this->where}[{$index}] must be a mapping");
        }

        return new self($value, "{$this->where}[{$index}]");
    }

    public function has(string $key): bool
    {
        return array_key_exists($key, $this->data);
    }

    public function strOr(string $key, string $default): string
    {
        return $this->has($key) ? $this->str($key) : $default;
    }

    public function intOr(string $key, int $default): int
    {
        return $this->has($key) ? $this->int($key) : $default;
    }

    private function require(string $key): mixed
    {
        if (!array_key_exists($key, $this->data)) {
            throw new \RuntimeException("{$this->where} is missing required field '{$key}'");
        }

        return $this->data[$key];
    }
}
