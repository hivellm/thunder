<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * A Thunder request body: `[id, command, args]` (WIRE-001/012).
 *
 * @psalm-immutable
 */
final class Request
{
    /** @param list<Value> $args */
    public function __construct(
        public readonly int $id,
        public readonly string $command,
        public readonly array $args = [],
    ) {
    }
}
