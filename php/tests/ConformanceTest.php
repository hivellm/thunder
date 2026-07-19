<?php

declare(strict_types=1);

namespace HiveLLM\Thunder\Tests;

use HiveLLM\Thunder\Wire\Codec;
use HiveLLM\Thunder\Wire\DecodeException;
use HiveLLM\Thunder\Wire\Frame;
use HiveLLM\Thunder\Wire\FrameTooLargeException;
use HiveLLM\Thunder\Wire\MapEntry;
use HiveLLM\Thunder\Wire\Request;
use HiveLLM\Thunder\Wire\Response;
use HiveLLM\Thunder\Wire\Value;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;
use Symfony\Component\Yaml\Yaml;

/**
 * The golden-vector corpus (TST-020).
 *
 * This runs in the default test command — never gated, never skipped. It is the
 * only thing that proves this lane agrees with the other five byte for byte,
 * and a corpus that runs "when someone remembers" proves nothing.
 *
 * The vectors live in the monorepo at `conformance/vectors/`. Should this
 * package ever be mirrored to a standalone repository (as the Go lane is), they
 * are absent there and these tests skip — they still run for real upstream,
 * which is where a wire change actually lands.
 */
final class ConformanceTest extends TestCase
{
    /**
     * The corpus may grow but must never quietly shrink. A loader that finds
     * zero vectors and reports success is the failure this floor forecloses.
     */
    private const VECTOR_FLOOR = 39;

    private static function vectorDirectory(): ?string
    {
        $path = realpath(__DIR__ . '/../../conformance/vectors');

        return is_string($path) && is_dir($path) ? $path : null;
    }

    public function testCorpusIsPresentAndDoesNotShrink(): void
    {
        $dir = self::vectorDirectory();
        if (null === $dir) {
            self::markTestSkipped('corpus not present (standalone mirror); it runs in the monorepo');
        }

        $count = count(glob($dir . '/*.yaml') ?: []);
        self::assertGreaterThanOrEqual(
            self::VECTOR_FLOOR,
            $count,
            "the corpus must not silently shrink (found {$count}, floor " . self::VECTOR_FLOOR . ')',
        );
    }

    #[DataProvider('vectors')]
    public function testVector(VectorNode $vector): void
    {
        $codec = new Codec();
        $bytes = self::hexToBytes($vector->str('frame_hex'));
        $max = $vector->intOr('max_frame_bytes', Frame::DEFAULT_MAX_FRAME_BYTES);

        switch ($vector->str('mode')) {
            case 'bidirectional':
                $expected = $vector->node('decoded');
                [$message, $consumed] = $this->decodeOne($bytes, $expected, $max, $codec);
                self::assertSame(strlen($bytes), $consumed, 'decode must consume exactly one frame');
                $this->assertMatchesExpected($message, $expected);
                // The encode half. This is what makes the corpus a
                // cross-language contract rather than a per-language round trip:
                // the bytes must come back identical, not merely equivalent.
                self::assertSame(
                    bin2hex($bytes),
                    bin2hex($this->encodeExpected($expected, $codec, $max)),
                    'canonical encoding must reproduce the vector bytes exactly',
                );
                break;

            case 'decode-only':
                $expected = $vector->node('decoded');
                [$message] = $this->decodeOne($bytes, $expected, $max, $codec);
                $this->assertMatchesExpected($message, $expected);
                // A tolerated legacy form must decode and must NOT be what we
                // emit: WIRE-011/013 accept these shapes and forbid producing
                // them, so re-encoding has to differ.
                self::assertNotSame(
                    bin2hex($bytes),
                    bin2hex($this->encodeExpected($expected, $codec, $max)),
                    'a decode-only form must not be re-emitted',
                );
                break;

            case 'stream':
                $offset = 0;
                foreach ($vector->nodes('frames') as $expected) {
                    [$message, $consumed] = $this->decodeOne(substr($bytes, $offset), $expected, $max, $codec);
                    $this->assertMatchesExpected($message, $expected);
                    $offset += $consumed;
                }
                self::assertSame(strlen($bytes), $offset, 'sequential decodes must consume the buffer exactly');
                break;

            case 'incomplete':
                self::assertNull(
                    Frame::trySplit($bytes, $max),
                    'a partial frame must report "need more bytes", not an error',
                );
                break;

            case 'reject':
                $this->assertRejects($bytes, $vector->str('error'), $max, $codec);
                break;

            default:
                self::fail("unknown vector mode '{$vector->str('mode')}'");
        }
    }

    /** @return array{0: Request|Response, 1: int} */
    private function decodeOne(string $bytes, VectorNode $expected, int $max, Codec $codec): array
    {
        $result = 'request' === $expected->str('kind')
            ? Frame::decodeRequest($bytes, $max, $codec)
            : Frame::decodeResponse($bytes, $max, $codec);

        self::assertNotNull($result, 'vector should decode to a complete frame');

        return $result;
    }

    private function encodeExpected(VectorNode $expected, Codec $codec, int $max): string
    {
        $body = 'request' === $expected->str('kind')
            ? $codec->encodeRequest(self::toRequest($expected))
            : $codec->encodeResponse(self::toResponse($expected));

        return Frame::encode($body, $max);
    }

    /**
     * A reject vector names the error **class**, not a message — that is the
     * part which is public API (CLT-052).
     */
    private function assertRejects(string $bytes, string $expectedClass, int $max, Codec $codec): void
    {
        try {
            $split = Frame::trySplit($bytes, $max);
            if (null === $split) {
                self::fail("vector should have been rejected as {$expectedClass}, but asked for more bytes");
            }
            // Past the framing layer the body must be what fails. Which typed
            // decoder is used does not matter: a body that is garbage to one is
            // garbage to both, and an empty body is the WIRE-024 case for each.
            $codec->decodeResponseBody($split[0]);
            self::fail("vector should have been rejected as {$expectedClass}");
        } catch (FrameTooLargeException) {
            self::assertSame('frame_too_large', $expectedClass);
        } catch (DecodeException) {
            self::assertSame('decode', $expectedClass);
        }
    }

    private function assertMatchesExpected(Request|Response $actual, VectorNode $expected): void
    {
        if ('request' === $expected->str('kind')) {
            self::assertInstanceOf(Request::class, $actual);
            self::assertSame($expected->int('id'), $actual->id);
            self::assertSame($expected->str('command'), $actual->command);

            $args = $expected->nodes('args');
            self::assertCount(count($args), $actual->args);
            foreach ($args as $i => $node) {
                self::assertTrue(
                    self::toValue($node)->equals($actual->args[$i]),
                    "arg {$i} differs from the vector",
                );
            }

            return;
        }

        self::assertInstanceOf(Response::class, $actual);
        self::assertSame($expected->int('id'), $actual->id);

        if ($expected->has('err')) {
            self::assertTrue($actual->isErr, 'expected an Err response');
            self::assertSame($expected->str('err'), $actual->err);

            return;
        }

        self::assertFalse($actual->isErr, 'expected an Ok response');
        self::assertTrue(
            self::toValue($expected->node('ok'))->equals($actual->ok),
            'Ok payload differs from the vector',
        );
    }

    // ─── Vector → object conversion ─────────────────────────────────────────

    private static function toRequest(VectorNode $node): Request
    {
        return new Request(
            $node->int('id'),
            $node->str('command'),
            array_map(static fn (VectorNode $arg): Value => self::toValue($arg), $node->nodes('args')),
        );
    }

    private static function toResponse(VectorNode $node): Response
    {
        return $node->has('err')
            ? Response::err($node->int('id'), $node->str('err'))
            : Response::ok($node->int('id'), self::toValue($node->node('ok')));
    }

    private static function toValue(VectorNode $node): Value
    {
        $type = $node->str('type');

        return match ($type) {
            'null' => Value::null(),
            'bool' => Value::bool($node->bool('value')),
            'int' => Value::int($node->int('value')),
            'float' => self::toFloat($node),
            'str' => Value::str($node->str('value')),
            'bytes' => Value::bytes(self::hexToBytes($node->strOr('value', ''))),
            'array' => Value::array(array_map(
                static fn (VectorNode $item): Value => self::toValue($item),
                $node->nodes('value'),
            )),
            'map' => Value::map(array_map(
                static fn (VectorNode $pair): MapEntry => new MapEntry(
                    self::toValue($pair->at(0)),
                    self::toValue($pair->at(1)),
                ),
                $node->nodes('value'),
            )),
            default => throw new \RuntimeException("unknown vector node type '{$type}'"),
        };
    }

    /**
     * A float node carries either a readable `value` or an exact `bits`
     * pattern. `bits` exists because some values cannot survive a decimal
     * round trip — NaN has no literal, and the corpus pins its exact payload.
     */
    private static function toFloat(VectorNode $node): Value
    {
        if (!$node->has('bits')) {
            return Value::float($node->float('value'));
        }

        $raw = hex2bin(str_pad($node->str('bits'), 16, '0', STR_PAD_LEFT));
        if (false === $raw) {
            throw new \RuntimeException('float `bits` is not valid hex');
        }
        /** @var array{1: float} $unpacked */
        $unpacked = unpack('E', $raw); // big-endian double: the bits as written

        return Value::float($unpacked[1]);
    }

    private static function hexToBytes(string $hex): string
    {
        $clean = preg_replace('/\s+/', '', $hex) ?? '';
        if ('' === $clean) {
            return '';
        }
        $bytes = hex2bin($clean);
        if (false === $bytes) {
            throw new \RuntimeException("frame_hex is not valid hex: {$hex}");
        }

        return $bytes;
    }

    /** @return iterable<string, array{0: VectorNode}> */
    public static function vectors(): iterable
    {
        $dir = self::vectorDirectory();
        if (null === $dir) {
            // Nothing to enumerate in a mirror; the skip test above reports it.
            return;
        }

        foreach (glob($dir . '/*.yaml') ?: [] as $file) {
            $parsed = Yaml::parseFile($file);
            if (!is_array($parsed)) {
                throw new \RuntimeException("vector {$file} is not a mapping");
            }
            $name = basename($file, '.yaml');
            yield $name => [new VectorNode($parsed, $name)];
        }
    }
}
