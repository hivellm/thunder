<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

/**
 * A frame's body is not something this layer can decode (WIRE-023).
 *
 * WIRE-024 requires the zero-length frame to stay *distinguishable* from this
 * on the typed path, which {@see KeepAliveException} provides by extending it.
 */
class DecodeException extends WireException
{
}
