//! Task 3.5: golden frames shared with the Kotlin codec.
//!
//! `golden/<name>.bin` is the frame this crate writes for sample `<name>`; run
//! with `PROTOCOL_UPDATE_GOLDEN=1` to rewrite them after a deliberate change.
//! `golden/kotlin/<name>.bin` is the frame the Kotlin codec writes for the same
//! sample, built independently in Kotlin; `agent-android/codec-test.sh
//! --write-golden` refreshes them.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use common::samples::samples;
use minicbor::Decoder;
use minicbor::data::Type;
use protocol::frame::{HEADER_LEN, write_frame};
use protocol::message::Message;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/golden");

fn kotlin_dir() -> PathBuf {
    Path::new(GOLDEN).join("kotlin")
}

fn frame(msg: &Message) -> Vec<u8> {
    let mut out = Vec::new();
    write_frame(msg, u32::MAX, &mut out).expect("no limit");
    out
}

fn show(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The names of the `.bin` files in `dir`.
fn bin_names(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "bin"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn sample_names() -> BTreeSet<String> {
    samples()
        .into_iter()
        .map(|(name, _)| name.to_owned())
        .collect()
}

/// Checks the header of `frame` and decodes its payload.
fn decode_frame(name: &str, frame: &[u8]) -> Message {
    let len = u32::from_be_bytes(frame[..HEADER_LEN].try_into().unwrap()) as usize;
    assert_eq!(len, frame.len() - HEADER_LEN, "{name}: header length");
    let payload = &frame[HEADER_LEN..];
    let mut d = Decoder::new(payload);
    assert_definite(&mut d, name);
    Message::decode(payload).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Walks one CBOR item and fails on any indefinite-length item.
fn assert_definite(d: &mut Decoder<'_>, name: &str) {
    match d.datatype().expect("valid CBOR") {
        Type::Array => {
            for _ in 0..d.array().unwrap().unwrap() {
                assert_definite(d, name);
            }
        }
        Type::Map => {
            for _ in 0..2 * d.map().unwrap().unwrap() {
                assert_definite(d, name);
            }
        }
        Type::ArrayIndef | Type::MapIndef | Type::BytesIndef | Type::StringIndef | Type::Break => {
            panic!("{name}: indefinite-length item at byte {}", d.position())
        }
        _ => d.skip().unwrap(),
    }
}

#[test]
fn the_rust_encoding_matches_the_golden_files() {
    let update = std::env::var_os("PROTOCOL_UPDATE_GOLDEN").is_some();
    for (name, msg) in samples() {
        let path = Path::new(GOLDEN).join(format!("{name}.bin"));
        let bytes = frame(&msg);
        if update {
            fs::write(&path, &bytes).unwrap();
            continue;
        }
        let golden = fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}; run with PROTOCOL_UPDATE_GOLDEN=1 to write it",
                path.display()
            )
        });
        assert_eq!(show(&golden), show(&bytes), "{name}");
    }
}

#[test]
fn the_golden_directory_holds_exactly_the_samples() {
    assert_eq!(bin_names(Path::new(GOLDEN)), sample_names());
}

#[test]
fn every_golden_frame_decodes_to_its_sample_with_definite_lengths() {
    for (name, msg) in samples() {
        let golden = fs::read(Path::new(GOLDEN).join(format!("{name}.bin"))).unwrap();
        assert_eq!(decode_frame(name, &golden), msg, "{name}");
    }
}

#[test]
fn the_samples_cover_every_message_kind() {
    let kinds: BTreeSet<u8> = samples().iter().map(|(_, m)| m.kind()).collect();
    assert_eq!(kinds, (0..=6).collect());
}

/// The frames the Kotlin encoder wrote for the same samples decode here to
/// the same messages and match this crate's bytes.
#[test]
fn kotlin_frames_decode_to_the_same_messages() {
    let dir = kotlin_dir();
    assert!(
        dir.is_dir(),
        "{} is missing; run agent-android/codec-test.sh --write-golden",
        dir.display()
    );
    assert_eq!(bin_names(&dir), sample_names());
    for (name, msg) in samples() {
        let kotlin = fs::read(dir.join(format!("{name}.bin"))).unwrap();
        assert_eq!(decode_frame(name, &kotlin), msg, "{name}");
        assert_eq!(show(&kotlin), show(&frame(&msg)), "{name}");
    }
}
