<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * Base class for every wire-layer failure, so a caller can catch the layer.
 *
 * The subclasses are separate types rather than one class with a code field,
 * because the corpus's reject vectors assert the **class**
 * (`frame_too_large` vs `decode`) and CLT-052 makes error classes public API
 * that user code branches on. A caller must be able to tell "the peer sent
 * something too big" from "the peer sent garbage" without parsing a message.
 */
abstract class WireException extends \RuntimeException
{
}
