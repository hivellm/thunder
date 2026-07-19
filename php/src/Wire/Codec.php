<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Wire;

use MessagePack\BufferUnpacker;
use MessagePack\Exception\UnpackingFailedException;
use MessagePack\Packer;
use MessagePack\PackOptions;

/**
 * The Thunder body codec: MessagePack in rmp-serde's externally-tagged shape.
 *
 * ## The shape, since it is not obvious from the types
 *
 * `Request`  → `[id, command, [args…]]`            (3-element array, WIRE-012)
 * `Response` → `[id, {"Ok": v}｜{"Err": s}]`        (2-element array, WIRE-003)
 * `Value::Null` → the bare string `"Null"`         (unit variant, WIRE-003)
 * every other `Value` → a one-key map `{"Int": 42}` (payload variant)
 * a `Map` payload → an array of `[k, v]` **arrays**, not a MessagePack map,
 * because Thunder map keys may be any value while MessagePack map keys here
 * would imply a different shape than the reference emits.
 *
 * ## Encoding is the strict half, decoding the tolerant one
 *
 * Encode emits exactly one form — the canonical one — because the corpus
 * compares bytes. Decode additionally accepts two legacy shapes that WIRE-011
 * and WIRE-013 require and forbid emitting: `Bytes` as an array of 0..255
 * integers, and a map-shaped `Request`. Anything accepted-but-not-emitted is
 * marked at its site, so the asymmetry is never mistaken for a bug.
 */
final class Codec
{
    private readonly Packer $packer;

    public function __construct()
    {
        // FORCE_STR: a PHP string is text unless this codec explicitly calls
        // packBin. Bytes never reaches the generic path, so the default must be
        // the safe one — a `str` mis-emitted as `bin` is a wire break.
        // FORCE_FLOAT64: floats are always f64 (WIRE-014); f32 would silently
        // destroy the bit patterns the corpus pins.
        $this->packer = new Packer(PackOptions::FORCE_STR | PackOptions::FORCE_FLOAT64);
    }

    // ─── Encoding ───────────────────────────────────────────────────────────

    public function encodeRequest(Request $request): string
    {
        $out = $this->packer->packArrayHeader(3)
            . $this->packUint($request->id)
            . $this->packer->packStr($request->command)
            . $this->packer->packArrayHeader(count($request->args));

        foreach ($request->args as $arg) {
            $out .= $this->encodeValue($arg);
        }

        return $out;
    }

    public function encodeResponse(Response $response): string
    {
        $result = $response->isErr
            ? $this->packer->packMapHeader(1) . $this->packer->packStr('Err')
                . $this->packer->packStr($response->err)
            : $this->packer->packMapHeader(1) . $this->packer->packStr('Ok')
                . $this->encodeValue($response->ok);

        return $this->packer->packArrayHeader(2) . $this->packUint($response->id) . $result;
    }

    public function encodeValue(Value $value): string
    {
        // The unit variant is a bare string, not a one-key map and not nil.
        if (Kind::Null === $value->kind) {
            return $this->packer->packStr('Null');
        }

        $tag = $this->packer->packMapHeader(1) . $this->packer->packStr($value->kind->tag());

        return $tag . match ($value->kind) {
            // packBool's docblock says `string` upstream while the body treats
            // it as a bool; the call is correct, the annotation is not.
            /* @phpstan-ignore argument.type */
            Kind::Bool => $this->packer->packBool((bool) $value->asBool()),
            Kind::Int => $this->packer->packInt((int) $value->asInt()),
            Kind::Float => $this->packer->packFloat64((float) $value->asFloat()),
            Kind::Bytes => $this->packer->packBin((string) $value->asBytes()),
            Kind::Str => $this->packer->packStr((string) $value->asStr()),
            Kind::Array => $this->encodeArrayPayload($value),
            Kind::Map => $this->encodeMapPayload($value),
            // No Null arm: the unit variant returned above. Should that ever
            // stop being true, PHP raises UnhandledMatchError rather than
            // emitting an empty payload that would silently corrupt the frame.
        };
    }

    private function encodeArrayPayload(Value $value): string
    {
        $items = $value->asArray() ?? [];
        $out = $this->packer->packArrayHeader(count($items));
        foreach ($items as $item) {
            $out .= $this->encodeValue($item);
        }

        return $out;
    }

    private function encodeMapPayload(Value $value): string
    {
        $pairs = $value->asMap() ?? [];
        $out = $this->packer->packArrayHeader(count($pairs));
        foreach ($pairs as $pair) {
            // Each pair is itself a 2-element array, so keys stay full values.
            $out .= $this->packer->packArrayHeader(2)
                . $this->encodeValue($pair->key)
                . $this->encodeValue($pair->value);
        }

        return $out;
    }

    /**
     * Frame ids are `u32`. PHP has no unsigned type, so a negative id would
     * pack as a negative int and produce a frame no other lane can read —
     * caught here rather than on someone else's decoder.
     */
    private function packUint(int $id): string
    {
        if ($id < 0 || $id > 0xFFFFFFFF) {
            throw new DecodeException("frame id {$id} is outside u32 range");
        }

        return $this->packer->packInt($id);
    }

    // ─── Decoding ───────────────────────────────────────────────────────────

    public function decodeRequestBody(string $body): Request
    {
        return $this->guard(function () use ($body): Request {
            $unpacker = new BufferUnpacker($body);
            $code = $this->peek($unpacker, $body);

            // WIRE-013 tolerance: a map-shaped Request is accepted on decode and
            // never emitted. Required because a family server must read peers
            // that predate the array encoding.
            if (TypeCode::isMap($code)) {
                return $this->decodeMapShapedRequest($unpacker, $body);
            }

            $size = $unpacker->unpackArrayHeader();
            if (3 !== $size) {
                throw new DecodeException("request must be a 3-element array, got {$size}");
            }

            $id = $this->decodeId($unpacker);
            $command = $unpacker->unpackStr();
            $args = [];
            $count = $unpacker->unpackArrayHeader();
            for ($i = 0; $i < $count; ++$i) {
                $args[] = $this->decodeValue($unpacker, $body);
            }

            return new Request($id, $command, $args);
        });
    }

    public function decodeResponseBody(string $body): Response
    {
        return $this->guard(function () use ($body): Response {
            $unpacker = new BufferUnpacker($body);
            $size = $unpacker->unpackArrayHeader();
            if (2 !== $size) {
                throw new DecodeException("response must be a 2-element array, got {$size}");
            }

            $id = $this->decodeId($unpacker);

            if (1 !== $unpacker->unpackMapHeader()) {
                throw new DecodeException('response result must be a single-key map');
            }
            $arm = $unpacker->unpackStr();

            return match ($arm) {
                'Ok' => Response::ok($id, $this->decodeValue($unpacker, $body)),
                'Err' => Response::err($id, $unpacker->unpackStr()),
                default => throw new DecodeException("unknown response arm '{$arm}'"),
            };
        });
    }

    private function decodeMapShapedRequest(BufferUnpacker $unpacker, string $body): Request
    {
        $count = $unpacker->unpackMapHeader();
        $id = null;
        $command = null;
        $args = null;

        for ($i = 0; $i < $count; ++$i) {
            $key = $unpacker->unpackStr();
            switch ($key) {
                case 'id':
                    $id = $this->decodeId($unpacker);
                    break;
                case 'command':
                    $command = $unpacker->unpackStr();
                    break;
                case 'args':
                    $args = [];
                    $len = $unpacker->unpackArrayHeader();
                    for ($j = 0; $j < $len; ++$j) {
                        $args[] = $this->decodeValue($unpacker, $body);
                    }
                    break;
                default:
                    // Unknown keys are skipped, not rejected (WIRE-013).
                    $unpacker->unpack();
            }
        }

        if (null === $id || null === $command || null === $args) {
            throw new DecodeException('map-shaped request must carry id, command and args');
        }

        return new Request($id, $command, $args);
    }

    public function decodeValue(BufferUnpacker $unpacker, string $buffer): Value
    {
        $code = $this->peek($unpacker, $buffer);

        // The unit variant arrives as a bare string.
        if (TypeCode::isStr($code)) {
            $tag = $unpacker->unpackStr();
            if ('Null' !== $tag) {
                throw new DecodeException("bare string value must be 'Null', got '{$tag}'");
            }

            return Value::null();
        }

        if (!TypeCode::isMap($code)) {
            throw new DecodeException(sprintf('value must be a tag map or "Null", got type 0x%02x', $code));
        }

        if (1 !== $unpacker->unpackMapHeader()) {
            throw new DecodeException('a tagged value must have exactly one key');
        }

        $tag = $unpacker->unpackStr();
        $kind = Kind::fromTag($tag);
        if (null === $kind) {
            throw new DecodeException("unknown value tag '{$tag}'");
        }

        return match ($kind) {
            Kind::Bool => Value::bool($unpacker->unpackBool()),
            Kind::Int => Value::int($this->unpackIntStrict($unpacker)),
            Kind::Float => Value::float($this->decodeFloatPayload($unpacker, $buffer)),
            Kind::Bytes => Value::bytes($this->decodeBytesPayload($unpacker, $buffer)),
            Kind::Str => Value::str($unpacker->unpackStr()),
            Kind::Array => $this->decodeArrayPayload($unpacker, $buffer),
            Kind::Map => $this->decodeMapPayload($unpacker, $buffer),
            Kind::Null => throw new DecodeException('"Null" is a bare string, never a tag map'),
        };
    }

    /**
     * WIRE-011 tolerance: `Bytes` is accepted as an array of integers 0..255
     * (Synap ≤1.x legacy) and normalised to the Bytes variant. Emitting this
     * form is forbidden — {@see encodeValue} always writes `bin`.
     */
    private function decodeBytesPayload(BufferUnpacker $unpacker, string $buffer): string
    {
        $code = $this->peek($unpacker, $buffer);

        if (TypeCode::isBin($code)) {
            return $unpacker->unpackBin();
        }

        if (TypeCode::isArray($code)) {
            $count = $unpacker->unpackArrayHeader();
            $out = '';
            for ($i = 0; $i < $count; ++$i) {
                $byte = $this->unpackIntStrict($unpacker);
                if ($byte < 0 || $byte > 255) {
                    throw new DecodeException("byte array element {$byte} is out of range 0..255");
                }
                $out .= chr($byte);
            }

            return $out;
        }

        throw new DecodeException(sprintf('Bytes payload must be bin or an int array, got 0x%02x', $code));
    }

    /** An integer payload widens to f64, as every other lane accepts. */
    private function decodeFloatPayload(BufferUnpacker $unpacker, string $buffer): float
    {
        $code = $this->peek($unpacker, $buffer);

        if (TypeCode::isInt($code)) {
            return (float) $this->unpackIntStrict($unpacker);
        }

        return $unpacker->unpackFloat();
    }

    private function decodeArrayPayload(BufferUnpacker $unpacker, string $buffer): Value
    {
        $count = $unpacker->unpackArrayHeader();
        $items = [];
        for ($i = 0; $i < $count; ++$i) {
            $items[] = $this->decodeValue($unpacker, $buffer);
        }

        return Value::array($items);
    }

    private function decodeMapPayload(BufferUnpacker $unpacker, string $buffer): Value
    {
        $count = $unpacker->unpackArrayHeader();
        $pairs = [];
        for ($i = 0; $i < $count; ++$i) {
            if (2 !== $unpacker->unpackArrayHeader()) {
                throw new DecodeException('a map entry must be a 2-element [key, value] array');
            }
            $pairs[] = new MapEntry(
                $this->decodeValue($unpacker, $buffer),
                $this->decodeValue($unpacker, $buffer),
            );
        }

        return Value::map($pairs);
    }

    private function decodeId(BufferUnpacker $unpacker): int
    {
        $id = $this->unpackIntStrict($unpacker);
        if ($id < 0 || $id > 0xFFFFFFFF) {
            throw new DecodeException("frame id {$id} is outside u32 range");
        }

        return $id;
    }

    /**
     * A MessagePack integer as a PHP int, or a decode error.
     *
     * The library returns `GMP|Decimal|string` for integers that do not fit a
     * PHP int — a `uint64` above `PHP_INT_MAX`. That is not merely a PHP
     * limitation to route around: Thunder's value model is `Int(i64)`
     * (WIRE-002), so such a number is outside the protocol in **every** lane.
     * Rejecting it as a decode error is the accurate answer; silently wrapping
     * or truncating would invent a value the sender never sent.
     */
    private function unpackIntStrict(BufferUnpacker $unpacker): int
    {
        $value = $unpacker->unpackInt();
        if (!is_int($value)) {
            throw new DecodeException(
                'integer exceeds the i64 range Thunder values are defined over: ' . (string) $value
            );
        }

        return $value;
    }

    /**
     * The next type byte, without consuming it.
     *
     * The unpacker keeps its offset private, but exposes how much is left — so
     * the position is the buffer length minus the remainder. Only the
     * discriminant is read here; the payload always goes through the library.
     */
    private function peek(BufferUnpacker $unpacker, string $buffer): int
    {
        $offset = strlen($buffer) - $unpacker->getRemainingCount();
        if (!isset($buffer[$offset])) {
            throw new DecodeException('unexpected end of body');
        }

        return ord($buffer[$offset]);
    }

    /**
     * Turn the library's failures into ours (WIRE-023): a malformed body is a
     * typed decode error, never an uncontrolled throw from a dependency, and
     * never a crash.
     *
     * @template T
     *
     * @param callable():T $decode
     *
     * @return T
     */
    private function guard(callable $decode)
    {
        try {
            return $decode();
        } catch (WireException $e) {
            throw $e;
        } catch (UnpackingFailedException $e) {
            throw new DecodeException('malformed message body: ' . $e->getMessage(), 0, $e);
        } catch (\TypeError|\ValueError $e) {
            throw new DecodeException('malformed message body: ' . $e->getMessage(), 0, $e);
        }
    }
}
