//! Pairwise-fuzz seed generator, Rust lane (TST-040).
//!
//! A checked-in, dependency-free generator produces a pseudo-random `Value`
//! tree per seed. The algorithm is deliberately trivial so every language
//! lane can replicate it exactly and the four byte outputs can be compared
//! pairwise (the cross-language comparison and TST-041 auto-shrink activate
//! when the other language lanes land — today only Rust exists, so this
//! lane asserts the two single-language properties):
//!
//! 1. encode → decode round-trips structurally (floats by bit pattern);
//! 2. re-encoding the decoded tree is byte-stable (canonical encoding is a
//!    fixed point — decode∘encode never changes the bytes).
//!
//! ## Generator definition (normative for other lanes)
//!
//! - RNG: 64-bit LCG, `state = state * 6364136223846793005 + 1442695040888963407`
//!   (Knuth MMIX), seeded with `state = seed`, advanced BEFORE each draw;
//!   a bounded draw is `(state >> 33) % n`.
//! - Tree: see `gen_value` — variant choice, sizes, and recursion order are
//!   the specification.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use thunder_wire::{decode_frame, encode_frame, Request, Response, Value};

const SEEDS: u64 = 200;
const MAX_DEPTH: u32 = 3;

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }

    /// Draw a value in `0..n` from the high bits (the low LCG bits are weak).
    fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 33) % n
    }
}

/// Alphabet for generated strings — includes multi-byte UTF-8 on purpose.
const ALPHABET: [&str; 8] = ["a", "b", "c", "-", "é", "ø", "字", "🚀"];

fn gen_str(rng: &mut Lcg) -> String {
    let len = rng.below(9);
    (0..len).map(|_| ALPHABET[rng.below(8) as usize]).collect()
}

fn gen_value(rng: &mut Lcg, depth: u32) -> Value {
    // At the depth limit only scalar variants (0..=5) are drawn.
    let variant = if depth == 0 {
        rng.below(6)
    } else {
        rng.below(8)
    };
    match variant {
        0 => Value::Null,
        1 => Value::Bool(rng.below(2) == 1),
        2 => Value::Int(rng.next() as i64),
        // Raw bit patterns: covers NaN payloads, infinities, subnormals,
        // -0.0 — the encode/decode pair must be bit-preserving (WIRE-014).
        3 => Value::Float(f64::from_bits(rng.next())),
        4 => Value::Bytes(
            (0..rng.below(9))
                .map(|_| (rng.next() >> 33) as u8)
                .collect(),
        ),
        5 => Value::Str(gen_str(rng)),
        6 => Value::Array(
            (0..rng.below(4))
                .map(|_| gen_value(rng, depth - 1))
                .collect(),
        ),
        7 => Value::Map(
            (0..rng.below(3))
                .map(|_| (gen_value(rng, depth - 1), gen_value(rng, depth - 1)))
                .collect(),
        ),
        _ => unreachable!(),
    }
}

/// One deterministic frame per seed: even seeds yield a `Request`, odd
/// seeds a `Response` (ok for `seed % 4 == 1`, err otherwise).
fn gen_frame(seed: u64) -> Vec<u8> {
    let mut rng = Lcg::new(seed);
    if seed.is_multiple_of(2) {
        let args = (0..rng.below(3) + 1)
            .map(|_| gen_value(&mut rng, MAX_DEPTH))
            .collect();
        encode_frame(&Request {
            id: seed as u32,
            command: format!("FUZZ{}", rng.below(10)),
            args,
        })
        .unwrap()
    } else if seed % 4 == 1 {
        encode_frame(&Response::ok(seed as u32, gen_value(&mut rng, MAX_DEPTH))).unwrap()
    } else {
        encode_frame(&Response::err(
            seed as u32,
            format!("ERR fuzz {}", gen_str(&mut rng)),
        ))
        .unwrap()
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| values_eq(x, y))
        }
        (Value::Map(xs), Value::Map(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys)
                    .all(|((ka, va), (kb, vb))| values_eq(ka, kb) && values_eq(va, vb))
        }
        _ => a == b,
    }
}

fn requests_eq(a: &Request, b: &Request) -> bool {
    a.id == b.id
        && a.command == b.command
        && a.args.len() == b.args.len()
        && a.args.iter().zip(&b.args).all(|(x, y)| values_eq(x, y))
}

fn responses_eq(a: &Response, b: &Response) -> bool {
    a.id == b.id
        && match (&a.result, &b.result) {
            (Ok(x), Ok(y)) => values_eq(x, y),
            (Err(x), Err(y)) => x == y,
            _ => false,
        }
}

#[test]
fn fuzz_seeds_round_trip_and_re_encode_byte_stable() {
    for seed in 0..SEEDS {
        let frame = gen_frame(seed);
        // Round-trip + fixed point: decode(frame) re-encodes to the same bytes.
        if seed.is_multiple_of(2) {
            let (decoded, used): (Request, usize) = decode_frame(&frame)
                .unwrap_or_else(|e| panic!("seed {seed}: decode failed: {e}"))
                .unwrap_or_else(|| panic!("seed {seed}: incomplete"));
            assert_eq!(used, frame.len(), "seed {seed}: consumed");
            let again = encode_frame(&decoded).unwrap();
            assert_eq!(again, frame, "seed {seed}: re-encode must be byte-stable");
            // And the re-decoded tree agrees structurally.
            let (twice, _): (Request, usize) = decode_frame(&again).unwrap().unwrap();
            assert!(
                requests_eq(&decoded, &twice),
                "seed {seed}: {decoded:?} != {twice:?}"
            );
        } else {
            let (decoded, used): (Response, usize) = decode_frame(&frame)
                .unwrap_or_else(|e| panic!("seed {seed}: decode failed: {e}"))
                .unwrap_or_else(|| panic!("seed {seed}: incomplete"));
            assert_eq!(used, frame.len(), "seed {seed}: consumed");
            let again = encode_frame(&decoded).unwrap();
            assert_eq!(again, frame, "seed {seed}: re-encode must be byte-stable");
            let (twice, _): (Response, usize) = decode_frame(&again).unwrap().unwrap();
            assert!(
                responses_eq(&decoded, &twice),
                "seed {seed}: {decoded:?} != {twice:?}"
            );
        }
    }
}

#[test]
fn generator_is_deterministic_per_seed() {
    for seed in 0..SEEDS {
        assert_eq!(
            gen_frame(seed),
            gen_frame(seed),
            "seed {seed}: generator must be a pure function of the seed"
        );
    }
    // Adjacent seeds must not collapse onto the same stream.
    assert_ne!(gen_frame(0), gen_frame(2));
    assert_ne!(gen_frame(1), gen_frame(5));
}
