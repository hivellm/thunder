# Thunder — PHP SDK

The PHP lane of the Thunder RPC family (`hivellm/thunder`). Wire bytes are
identical to the Rust, TypeScript, Python, C# and Go lanes — every
implementation pins its default test run to `conformance/vectors/*.yaml`
(SPEC-005), so one PR changes wire behaviour everywhere or fails CI.

> **Status: the wire layer.** The multiplexed client (SPEC-003) is not in this
> package yet. What is here is complete and proven against the corpus; what is
> missing is missing entirely, not stubbed.

## Layout

- `src/Wire/` — the wire layer (SPEC-001): the 8-variant `Value`,
  array-encoded `Request` / `Response`, `PUSH_ID` (= `u32::MAX`), and the
  length-prefixed MessagePack frame codec with the cap checked before body
  allocation. Encoding uses [`rybakit/msgpack`](https://github.com/rybakit/msgpack.php)
  driven at the low level, which reproduces the reference (rmp-serde)
  shortest-form integer packing byte for byte.
- `tests/` — unit tests plus the corpus loader (TST-020), the primary
  cross-language proof, run by the default `phpunit`.

## Usage

```php
use HiveLLM\Thunder\Wire\Codec;
use HiveLLM\Thunder\Wire\Frame;
use HiveLLM\Thunder\Wire\Request;
use HiveLLM\Thunder\Wire\Value;

$codec = new Codec();

// Encode a request into a complete frame, ready for a socket.
$frame = Frame::encode($codec->encodeRequest(
    new Request(id: 1, command: 'PING', args: [Value::str('hello')]),
));

// Decode whatever has arrived so far. `null` means "need more bytes" — a
// normal outcome on a stream, not an error.
$result = Frame::decodeResponse($buffer);
if (null !== $result) {
    [$response, $consumed] = $result;
    $buffer = substr($buffer, $consumed);

    if ($response->isErr) {
        // Error strings travel verbatim (WIRE-040); interpreting a
        // "[code] message" prefix is the client's job, driven by the profile.
        handle($response->err);
    } else {
        echo $response->ok->asStr();
    }
}
```

### Two things PHP forces this lane to do differently

**A `Value` is built through factories, never inferred.** PHP has one string
type for both text and binary, but Thunder does not: `Str` encodes as
MessagePack `str` and `Bytes` as `bin`, and WIRE-015 forbids smuggling one as
the other. A design that guessed the variant from the PHP value could not tell
`Value::str('AB')` from `Value::bytes('AB')` — so the variant is always named.

**A `Map` is a list of pairs, not a PHP array.** Thunder map keys may be any
value and order is observable (WIRE-002); a PHP array would restrict keys to
`int|string` and collapse duplicates. Use `Value::mapOf([...])` for the common
string-keyed case, or `MapEntry` directly when keys are not strings.

## Test / quality gate

```sh
composer install
vendor/bin/phpstan analyse    # type-check first: the faster signal
vendor/bin/phpunit            # unit tests + the corpus, one command
```

The corpus is not a separate target. TST-020 requires it in the **default** test
command — never feature-gated, never skipped — because a conformance run that
has to be remembered is one that stops happening.

## Release train (PKG-011)

This package versions in lockstep with every other lane: one tag, one version,
everywhere. It publishes to [Packagist](https://packagist.org/) as
`hivellm/thunder`, which resolves from a VCS tag — so, like the Go lane, there
is no push step, but the tag must exist.

The shared gate in `release.yml` runs this lane's checks on the tagged commit
before anything ships.

## Where this code lives

The source of truth is the `php/` directory of the
[Thunder monorepo](https://github.com/hivellm/thunder), alongside the other
five lanes and the conformance corpus they all share.

The corpus is read from `../conformance/vectors/` relative to this package. If
this code is ever mirrored to a standalone repository (as the Go lane is), those
vectors are absent there and the corpus tests **skip** — they still run for real
upstream, which is where a wire change actually lands. A skipped corpus in a
mirror is expected; a skipped corpus in the monorepo is a bug.
