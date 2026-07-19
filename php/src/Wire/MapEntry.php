<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * One `[key, value]` pair of a Thunder `Map`.
 *
 * A map is an ordered pair list and its keys may be any {@see Value}
 * (WIRE-002), so it cannot be a PHP array: that would restrict keys to
 * int|string and collapse duplicates. Order is observable — the HELLO payload's
 * field order is pinned by the corpus.
 *
 * @psalm-immutable
 */
final class MapEntry
{
    public function __construct(
        public readonly Value $key,
        public readonly Value $value,
    ) {
    }
}
