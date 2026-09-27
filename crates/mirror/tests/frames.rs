//! Task 7.8: frame access. The mirror decides the size and quality of the
//! latest frame, no larger than the requested long edge, upright and never
//! enlarged, and reports when it was captured, how old it is and how it
//! stands against the last tree read; the frame source does the encoding.

mod support;

use mirror::backend::EncodeRequest;
use mirror::testing::{FakeTarget, Step};
use mirror::{
    DEFAULT_MAX_DIMENSION, DEFAULT_QUALITY, FrameRequest, Mirror, MirrorConfig, MirrorError,
    TreeSkew, code,
};
use protocol::message::Rotation;
use support::{fixture, ms};

fn at_most(max_dimension: u32) -> FrameRequest {
    FrameRequest {
        max_dimension: Some(max_dimension),
        quality: None,
    }
}

#[test]
fn a_scaled_frame_fits_the_requested_long_edge_upright_with_its_timestamp() {
    let target = FakeTarget::new(fixture("login-ios"));
    target.frame_size(1206, 2622, Rotation::R0);
    target.at(ms(50), Step::Frame(9));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    target.advance(ms(80));

    let frame = mirror.frame(&at_most(1000)).unwrap();
    assert_eq!((frame.width, frame.height), (460, 1000));
    assert!(frame.width.max(frame.height) <= 1000);
    assert_eq!(frame.quality, DEFAULT_QUALITY);
    assert_eq!(frame.seq, 1);
    assert_eq!(frame.captured_at, target.instant(ms(50)));
    assert_eq!(frame.age, ms(30));
    assert_eq!(frame.tree_skew, TreeSkew::FrameNewer(ms(50)));
    assert_eq!(frame.data, b"jpeg 460x1000 q80 frame 1");
    assert_eq!(
        target.encodes(),
        [EncodeRequest {
            seq: 1,
            width: 460,
            height: 1000,
            quality: 80,
            rotation: Rotation::R0,
        }]
    );
}

#[test]
fn a_rotated_frame_is_encoded_upright() {
    let target = FakeTarget::new(fixture("login-ios"));
    target.frame_size(1206, 2622, Rotation::R90);
    target.at(ms(0), Step::Frame(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    let frame = mirror.frame(&at_most(1000)).unwrap();
    assert_eq!(
        (frame.width, frame.height),
        (1000, 460),
        "landscape once turned upright"
    );
    assert_eq!(target.encodes()[0].rotation, Rotation::R90);
}

#[test]
fn small_frames_are_not_enlarged_and_defaults_apply() {
    let target = FakeTarget::new(fixture("login-ios"));
    target.frame_size(390, 844, Rotation::R0);
    target.at(ms(0), Step::Frame(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    let frame = mirror.frame(&FrameRequest::default()).unwrap();
    assert_eq!((frame.width, frame.height), (390, 844));
    assert_eq!(DEFAULT_MAX_DIMENSION, 1000);

    target.frame_size(1080, 2400, Rotation::R0);
    target.at(ms(10), Step::Frame(2));
    target.advance(ms(10));
    let frame = mirror.frame(&FrameRequest::default()).unwrap();
    assert_eq!((frame.width, frame.height), (450, 1000));
    assert_eq!(frame.seq, 2, "always the latest frame");
}

#[test]
fn quality_defaults_to_80_and_is_clamped() {
    let target = FakeTarget::new(fixture("login-ios"));
    target.at(ms(0), Step::Frame(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    mirror.pump().unwrap();
    let quality = |mirror: &mut Mirror, quality: Option<u8>| {
        mirror
            .frame(&FrameRequest {
                max_dimension: None,
                quality,
            })
            .unwrap()
            .quality
    };
    assert_eq!(quality(&mut mirror, None), 80);
    assert_eq!(quality(&mut mirror, Some(0)), 1);
    assert_eq!(quality(&mut mirror, Some(55)), 55);
    assert_eq!(quality(&mut mirror, Some(150)), 100);
}

#[test]
fn a_frame_older_than_the_last_tree_read_says_so() {
    let target = FakeTarget::new(fixture("login-ios"));
    target.read_cost(ms(5));
    target.at(ms(0), Step::Frame(1));
    let mut mirror = Mirror::new(target.backend(), MirrorConfig::default());
    target.advance(ms(40));
    mirror.pump().unwrap();
    let frame = mirror.frame(&FrameRequest::default()).unwrap();
    assert_eq!(frame.captured_at, target.instant(ms(0)));
    assert_eq!(frame.age, ms(45));
    assert_eq!(
        frame.tree_skew,
        TreeSkew::FrameOlder(ms(45)),
        "the read finished at 45 ms"
    );

    let target = FakeTarget::new(fixture("login-ios"));
    target.at(ms(0), Step::Frame(1));
    let mut unread = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        unread.frame(&FrameRequest::default()).unwrap().tree_skew,
        TreeSkew::NoTree
    );
}

#[test]
fn no_frame_is_an_error_with_its_code() {
    let target = FakeTarget::new(fixture("login"));
    let mut blind = Mirror::new(target.backend_without_frames(), MirrorConfig::default());
    let error = blind.frame(&FrameRequest::default()).unwrap_err();
    assert_eq!(error, MirrorError::NoFrame);
    assert_eq!(error.code(), code::NO_FRAME);

    let mut waiting = Mirror::new(target.backend(), MirrorConfig::default());
    assert_eq!(
        waiting.frame(&FrameRequest::default()).unwrap_err().code(),
        code::NO_FRAME,
        "no frame has arrived yet"
    );
    assert!(target.encodes().is_empty());
}
