<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Tests;

use HiveLLM\Thunder\Wire\Codec;
use HiveLLM\Thunder\Wire\DecodeException;
use HiveLLM\Thunder\Wire\Frame;
use HiveLLM\Thunder\Wire\FrameTooLargeException;
use HiveLLM\Thunder\Wire\KeepAliveException;
use HiveLLM\Thunder\Wire\Kind;
use HiveLLM\Thunder\Wire\MapEntry;
use HiveLLM\Thunder\Wire\Request;
use HiveLLM\Thunder\Wire\Response;
use HiveLLM\Thunder\Wire\Value;
use PHPUnit\Framework\TestCase;

/**
 * Wire-layer behaviour the corpus does not pin.
 *
 * The corpus proves this lane agrees with the other five on the bytes. It does
 * not cover the edges that only exist in *this* language — PHP's single string
 * type, its lack of an unsigned integer — nor the encode-side cap, since a
 * vector can only carry bytes that were successfully produced.
 */
final class WireTest extends TestCase
{
    // ─── The PHP-specific hazard: bytes and text are the same type ───────────

    public function testBytesAndStrAreDistinctDespiteBothBeingPhpStrings(): void
    {
        $codec = new Codec();
        $text = $codec->encodeValue(Value::str('AB'));
        $binary = $codec->encodeValue(Value::bytes('AB'));

        // Same PHP string, different wire families (WIRE-010/015) — str vs bin.
        self::assertNotSame(bin2hex($text), bin2hex($binary));
        self::assertStringContainsString('a2', bin2hex($text), 'text must use the str family');
        self::assertStringContainsString('c4', bin2hex($binary), 'binary must use the bin family');
    }

    public function testBytesAndStrOfEqualContentAreNotEqualValues(): void
    {
        self::assertFalse(Value::bytes('AB')->equals(Value::str('AB')));
    }

    public function testStrReadsAsBytesButBytesDoesNotReadAsStr(): void
    {
        self::assertSame('hi', Value::str('hi')->asBytes());
        self::assertNull(Value::bytes('hi')->asStr());
    }

    // ─── Float identity (WIRE-014) ──────────────────────────────────────────

    /**
     * `NAN == NAN` is false in PHP, as everywhere. A naive equality would
     * therefore fail the corpus's NaN vector on every run, so equality compares
     * bit patterns instead.
     */
    public function testNanEqualsItselfStructurally(): void
    {
        self::assertTrue(Value::float(NAN)->equals(Value::float(NAN)));
    }

    /**
     * The mirror hazard: `-0.0 == 0.0` is *true* in PHP, so a naive equality
     * would hide a lost sign bit — the drift WIRE-014 pins.
     */
    public function testNegativeZeroIsNotZero(): void
    {
        self::assertFalse(Value::float(-0.0)->equals(Value::float(0.0)));
    }

    // ─── The cap, on the side no vector can reach (WIRE-020) ────────────────

    public function testEncodeRefusesABodyOverTheCap(): void
    {
        $this->expectException(FrameTooLargeException::class);
        Frame::encode(str_repeat("\x00", 11), 10);
    }

    public function testDecodeRejectsFromThePrefixAloneWithNoBodyPresent(): void
    {
        // Header claims 5 bytes against a cap of 4, and no body follows. A
        // decoder that waited for the body would hang instead of rejecting.
        try {
            Frame::trySplit("\x05\x00\x00\x00", 4);
            self::fail('expected the cap to fire from the prefix alone');
        } catch (FrameTooLargeException $e) {
            self::assertSame(5, $e->body);
            self::assertSame(4, $e->limit);
        }
    }

    // ─── Partial input is not an error (WIRE-022) ───────────────────────────

    public function testPartialFrameAsksForMoreBytes(): void
    {
        self::assertNull(Frame::trySplit(''), 'empty buffer');
        self::assertNull(Frame::trySplit("\x08\x00\x00"), 'prefix itself incomplete');
        self::assertNull(Frame::trySplit("\x08\x00\x00\x00\x93\x01"), 'body incomplete');
    }

    public function testTwoFramesInOneBufferAreConsumedOneAtATime(): void
    {
        $codec = new Codec();
        $first = Frame::encode($codec->encodeRequest(new Request(1, 'A')));
        $second = Frame::encode($codec->encodeRequest(new Request(2, 'B')));

        $buffer = $first . $second;
        $split = Frame::trySplit($buffer);
        self::assertNotNull($split);
        self::assertSame(strlen($first), $split[1], 'must consume exactly one frame');

        $rest = Frame::trySplit(substr($buffer, $split[1]));
        self::assertNotNull($rest);
        self::assertSame(strlen($second), $rest[1]);
    }

    // ─── The keep-alive is valid, and distinguishable (WIRE-024) ────────────

    public function testZeroLengthFrameIsAValidRawFrameCarryingNoBody(): void
    {
        $split = Frame::trySplit("\x00\x00\x00\x00");
        self::assertNotNull($split, 'a zero-length frame is valid, not incomplete');
        self::assertSame('', $split[0]);
        self::assertSame(4, $split[1]);
    }

    public function testZeroLengthFrameFailsTypedDecodeWithItsOwnErrorClass(): void
    {
        try {
            Frame::decodeRequest("\x00\x00\x00\x00");
            self::fail('a typed decode has no message to produce');
        } catch (KeepAliveException $e) {
            // Distinct from a malformed body, so a caller can treat liveness
            // ticks as normal traffic instead of dropping the connection.
            self::assertInstanceOf(DecodeException::class, $e, 'stays catchable as a decode error');
        }
    }

    public function testMalformedBodyIsADecodeErrorButNotAKeepAlive(): void
    {
        $garbage = Frame::encode("\xc1\xc1\xc1");
        try {
            Frame::decodeRequest($garbage);
            self::fail('garbage must not decode');
        } catch (DecodeException $e) {
            self::assertNotInstanceOf(KeepAliveException::class, $e);
        }
    }

    // ─── PHP has no unsigned integer (WIRE-002 is i64) ──────────────────────

    public function testIdOutsideU32IsRefusedOnEncode(): void
    {
        $this->expectException(DecodeException::class);
        (new Codec())->encodeRequest(new Request(-1, 'PING'));
    }

    public function testPushIdIsRepresentableAndNotNegative(): void
    {
        // 0xFFFFFFFF fits a 64-bit PHP int; the risk is sign extension, not size.
        self::assertGreaterThan(0, Frame::PUSH_ID);
        $codec = new Codec();
        $frame = Frame::encode($codec->encodeResponse(Response::ok(Frame::PUSH_ID, Value::null())));
        $decoded = Frame::decodeResponse($frame);
        self::assertNotNull($decoded);
        self::assertSame(Frame::PUSH_ID, $decoded[0]->id);
    }

    // ─── Decode tolerances: accepted, never emitted ─────────────────────────

    public function testBytesArrivingAsAnIntArrayIsAcceptedAndNormalised(): void
    {
        // WIRE-011: {"Bytes": [1, 2, 3]} — the Synap ≤1.x legacy shape.
        $legacy = "\x81\xa5Bytes\x93\x01\x02\x03";
        $codec = new Codec();
        $value = $codec->decodeValue(new \MessagePack\BufferUnpacker($legacy), $legacy);

        self::assertSame(Kind::Bytes, $value->kind);
        self::assertSame("\x01\x02\x03", $value->asBytes());
        // …and we must not emit it back that way.
        self::assertNotSame(bin2hex($legacy), bin2hex($codec->encodeValue($value)));
        self::assertStringContainsString('c4', bin2hex($codec->encodeValue($value)));
    }

    public function testMapShapedRequestIsAcceptedAndNeverEmitted(): void
    {
        // WIRE-013: {"id": 1, "command": "PING", "args": []} plus an unknown key
        // that must be skipped rather than rejected.
        $codec = new Codec();
        $body = "\x84\xa2id\x01\xa7command\xa4PING\xa4args\x90\xa5extra\xc0";

        $request = $codec->decodeRequestBody($body);
        self::assertSame(1, $request->id);
        self::assertSame('PING', $request->command);
        self::assertSame([], $request->args);

        self::assertNotSame(
            bin2hex($body),
            bin2hex($codec->encodeRequest($request)),
            'the canonical form is the array encoding',
        );
    }

    public function testMapShapedRequestMissingAFieldIsRejected(): void
    {
        $this->expectException(DecodeException::class);
        (new Codec())->decodeRequestBody("\x81\xa2id\x01");
    }

    // ─── Value model (WIRE-002) ─────────────────────────────────────────────

    public function testMapKeysMayBeAnyValueAndOrderIsPreserved(): void
    {
        $map = Value::map([
            new MapEntry(Value::int(2), Value::str('two')),
            new MapEntry(Value::int(1), Value::str('one')),
        ]);

        $pairs = $map->asMap();
        self::assertNotNull($pairs);
        self::assertSame(2, $pairs[0]->key->asInt(), 'insertion order, not key order');

        $codec = new Codec();
        $roundTripped = $codec->decodeValue(
            new \MessagePack\BufferUnpacker($encoded = $codec->encodeValue($map)),
            $encoded,
        );
        self::assertTrue($map->equals($roundTripped));
    }

    public function testAccessorsReturnNullOnKindMismatchRatherThanThrowing(): void
    {
        $value = Value::int(1);
        self::assertNull($value->asStr());
        self::assertNull($value->asBool());
        self::assertNull($value->asArray());
        self::assertSame(1.0, $value->asFloat(), 'an Int widens to float');
    }

    public function testEmptyContainersAreLegalAndRoundTrip(): void
    {
        $codec = new Codec();
        foreach ([Value::array([]), Value::map([]), Value::bytes(''), Value::str('')] as $empty) {
            $encoded = $codec->encodeValue($empty);
            $back = $codec->decodeValue(new \MessagePack\BufferUnpacker($encoded), $encoded);
            self::assertTrue($empty->equals($back), "empty {$empty->kind->value} must round-trip");
        }
    }
}
