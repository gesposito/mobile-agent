//! Microbenchmarks of the codec and the framing (design D14), run by
//! `mobile-agent-bench micro` with criterion's default sample settings.
//!
//! The inputs are the golden `response_snapshot` (4 nodes) and the datasets
//! of `support/generate.rs`: snapshots of 500 and 1,049 nodes, 10,000 events
//! and 1,000 requests. Each is timed through `encode`, into a vector reused
//! and cleared on every iteration, `decode`, `to_vec` and `write_frame`, into
//! a reused vector under the default limit, every message of the input on
//! each iteration; the event stream, written once as frames, is also timed
//! through a fresh `FrameDecoder` fed 64 KiB at a time. Ids are
//! `protocol/<function>/<input>`.
//!
//! Before each benchmark, one extra call of its routine checks the result
//! against the input outside the timed region and counts what the call
//! allocated, printed as `allocations <id> <allocations> <bytes>`.

use std::hint::black_box;

use criterion::measurement::WallTime;
use criterion::{BenchmarkGroup, BenchmarkId, Criterion, criterion_group, criterion_main};
use protocol::Message;
use protocol::frame::{DEFAULT_MAX_FRAME, FrameDecoder, FrameTooLarge, write_frame};

#[path = "support/alloc.rs"]
mod alloc;
#[path = "support/generate.rs"]
mod generate;
// For the golden `response_snapshot`.
#[allow(dead_code)]
#[path = "../tests/common/samples.rs"]
mod samples;

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

/// The first part of every benchmark's id.
const GROUP: &str = "protocol";

/// How many bytes of the event stream the frame decoder is given at a time.
const PIECE: usize = 64 * 1024;

/// One input, with what its benchmarks check their results against.
struct Input {
    name: &'static str,
    messages: Vec<Message>,
    /// Every message's encoding, back to back.
    payloads: Vec<u8>,
    /// Where each message's encoding ends in `payloads`.
    ends: Vec<usize>,
    /// Every message as a frame, back to back: a 4-byte big-endian length,
    /// then the encoding.
    frames: Vec<u8>,
}

impl Input {
    fn new(name: &'static str, messages: Vec<Message>) -> Input {
        let mut payloads = Vec::new();
        let mut ends = Vec::with_capacity(messages.len());
        let mut frames = Vec::new();
        for message in &messages {
            let bytes = message.to_vec();
            let len = u32::try_from(bytes.len()).expect("a message under 4 GiB");
            payloads.extend_from_slice(&bytes);
            ends.push(payloads.len());
            frames.extend_from_slice(&len.to_be_bytes());
            frames.extend_from_slice(&bytes);
        }
        Input {
            name,
            messages,
            payloads,
            ends,
            frames,
        }
    }

    /// The encoding of message `i`.
    fn payload(&self, i: usize) -> &[u8] {
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        &self.payloads[start..self.ends[i]]
    }

    /// The id of this input's benchmark of `function`, as criterion names it.
    fn id(&self, function: &str) -> String {
        format!("{GROUP}/{function}/{}", self.name)
    }
}

/// Encodes every message into `out`, cleared first.
fn encode(messages: &[Message], out: &mut Vec<u8>) {
    out.clear();
    for message in messages {
        message.encode(out);
    }
}

/// Decodes every message of `input` and passes each to `each`, which drops
/// it.
fn decode(input: &Input, mut each: impl FnMut(usize, Message)) {
    let mut start = 0;
    for (i, &end) in input.ends.iter().enumerate() {
        let message = Message::decode(&input.payloads[start..end]).expect("the input decodes");
        each(i, message);
        start = end;
    }
}

/// Encodes every message into a vector of its own and passes each to `each`,
/// which drops it.
fn to_vec(messages: &[Message], mut each: impl FnMut(usize, Vec<u8>)) {
    for (i, message) in messages.iter().enumerate() {
        each(i, message.to_vec());
    }
}

/// Writes every message as a frame into `out`, cleared first, under the
/// default limit.
fn write_frames(messages: &[Message], out: &mut Vec<u8>) -> Result<(), FrameTooLarge> {
    out.clear();
    for message in messages {
        write_frame(message, DEFAULT_MAX_FRAME, out)?;
    }
    Ok(())
}

/// Splits `stream` into frames with a fresh decoder fed [`PIECE`] bytes at a
/// time, taking every complete frame after each piece, and passes each
/// payload to `each`. Returns the number of frames and the bytes the decoder
/// still holds.
fn split(stream: &[u8], mut each: impl FnMut(usize, &[u8])) -> (usize, usize) {
    let mut decoder = FrameDecoder::new(DEFAULT_MAX_FRAME);
    let mut frames = 0;
    for piece in stream.chunks(PIECE) {
        decoder.push(piece);
        while let Some(frame) = decoder.next_frame() {
            each(frames, frame.expect("no frame exceeds the limit"));
            frames += 1;
        }
    }
    (frames, decoder.buffered())
}

/// `out` has room for the largest input, so no call grows it: the count is
/// that of every timed iteration.
fn bench_encode(group: &mut BenchmarkGroup<'_, WallTime>, input: &Input, out: &mut Vec<u8>) {
    let ((), counts) = alloc::counted(|| encode(&input.messages, out));
    assert!(
        *out == input.payloads,
        "{}: other bytes encoded",
        input.name
    );
    alloc::report(&input.id("encode"), counts);
    group.bench_with_input(BenchmarkId::new("encode", input.name), input, |b, input| {
        b.iter(|| encode(black_box(&input.messages), black_box(&mut *out)));
    });
}

fn bench_decode(group: &mut BenchmarkGroup<'_, WallTime>, input: &Input) {
    let ((), counts) = alloc::counted(|| {
        decode(input, |i, message| {
            assert!(
                message == input.messages[i],
                "{}: message {i} decodes to another",
                input.name
            );
        });
    });
    alloc::report(&input.id("decode"), counts);
    // Each decoded message is dropped inside the timed region, as spike S8
    // timed `Message::decode`.
    group.bench_with_input(BenchmarkId::new("decode", input.name), input, |b, input| {
        b.iter(|| decode(black_box(input), |_, message| drop(black_box(message))));
    });
}

fn bench_to_vec(group: &mut BenchmarkGroup<'_, WallTime>, input: &Input) {
    let ((), counts) = alloc::counted(|| {
        to_vec(&input.messages, |i, bytes| {
            assert!(
                bytes == input.payload(i),
                "{}: other bytes for message {i}",
                input.name
            );
        });
    });
    alloc::report(&input.id("to_vec"), counts);
    group.bench_with_input(BenchmarkId::new("to_vec", input.name), input, |b, input| {
        b.iter(|| {
            to_vec(black_box(&input.messages), |_, bytes| {
                drop(black_box(bytes))
            })
        });
    });
}

/// `out` has room for the largest input, so no call grows it: the count is
/// that of every timed iteration.
fn bench_write_frame(group: &mut BenchmarkGroup<'_, WallTime>, input: &Input, out: &mut Vec<u8>) {
    let (written, counts) = alloc::counted(|| write_frames(&input.messages, out));
    written.unwrap_or_else(|e| panic!("{}: {e}", input.name));
    assert!(*out == input.frames, "{}: other frames written", input.name);
    alloc::report(&input.id("write_frame"), counts);
    let id = BenchmarkId::new("write_frame", input.name);
    group.bench_with_input(id, input, |b, input| {
        b.iter(|| write_frames(black_box(&input.messages), black_box(&mut *out)));
    });
}

fn bench_frame_decoder(group: &mut BenchmarkGroup<'_, WallTime>, input: &Input) {
    let ((frames, left), counts) = alloc::counted(|| {
        split(&input.frames, |i, payload| {
            assert!(
                payload == input.payload(i),
                "{}: frame {i} carries other bytes",
                input.name
            );
        })
    });
    assert_eq!(frames, input.messages.len(), "{}: frames", input.name);
    assert_eq!(left, 0, "{}: bytes left in the decoder", input.name);
    alloc::report(&input.id("frame-decoder"), counts);
    let id = BenchmarkId::new("frame-decoder", input.name);
    group.bench_with_input(id, input, |b, input| {
        b.iter(|| {
            split(black_box(&input.frames), |_, payload| {
                black_box(payload);
            })
        });
    });
}

fn benches(c: &mut Criterion) {
    let (_, golden) = samples::samples()
        .into_iter()
        .find(|&(name, _)| name == "response_snapshot")
        .expect("a golden response_snapshot");
    let mut inputs = vec![Input::new("response-snapshot", vec![golden])];
    for (name, messages) in generate::datasets() {
        inputs.push(Input::new(name, messages));
    }
    let largest = inputs.iter().map(|input| input.frames.len()).max();
    let mut out = Vec::with_capacity(largest.unwrap_or(0));

    let mut group = c.benchmark_group(GROUP);
    for input in &inputs {
        bench_encode(&mut group, input, &mut out);
        bench_decode(&mut group, input);
        bench_to_vec(&mut group, input);
        bench_write_frame(&mut group, input, &mut out);
    }
    let events = inputs
        .iter()
        .find(|input| input.name == "events-10k")
        .expect("the event stream");
    bench_frame_decoder(&mut group, events);
    group.finish();
}

criterion_group!(codec, benches);
criterion_main!(codec);
