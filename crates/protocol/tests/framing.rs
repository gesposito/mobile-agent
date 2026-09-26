//! Task 3.2: frames are a 32-bit big-endian payload length plus one CBOR item;
//! partial frames are buffered and an oversized frame is refused explicitly,
//! never delivered in part.

use protocol::frame::{
    DEFAULT_MAX_FRAME, FrameDecoder, FrameTooLarge, HEADER_LEN, MIN_MAX_FRAME, Oversized,
    write_frame,
};
use protocol::message::{
    Call, ImageFormat, Message, Method, Ping, Reply, Request, Response, Rotation, Screenshot,
    SnapshotParams, kind,
};

fn screenshot_response(id: u32, data_len: usize) -> Message {
    Message::Response(Response {
        id,
        method: Method::SCREENSHOT,
        result: Ok(Reply::Screenshot(Screenshot {
            format: ImageFormat::PNG,
            width: 1080,
            height: 2400,
            rotation: Rotation::R0,
            data: (0..data_len).map(|i| (i % 251) as u8).collect(),
        })),
    })
}

/// Pushes `bytes` and collects every complete payload, decoded.
fn drain(decoder: &mut FrameDecoder) -> Vec<Result<Message, Oversized>> {
    let mut out = Vec::new();
    while let Some(frame) = decoder.next_frame() {
        out.push(frame.map(|payload| Message::decode(payload).expect("payload decodes")));
    }
    out
}

#[test]
fn default_and_minimum_frame_limits() {
    assert_eq!(HEADER_LEN, 4);
    assert_eq!(DEFAULT_MAX_FRAME, 16 * 1024 * 1024);
    assert_eq!(MIN_MAX_FRAME, 4 * 1024);
}

#[test]
fn a_frame_is_a_big_endian_length_then_one_cbor_item() {
    let mut out = vec![0xee];
    write_frame(
        &Message::Ping(Ping { nonce: 0 }),
        DEFAULT_MAX_FRAME,
        &mut out,
    )
    .unwrap();
    assert_eq!(out, [0xee, 0, 0, 0, 5, 0xa2, 0x00, 0x05, 0x01, 0x00]);
    let big = screenshot_response(1, 70_000);
    let mut out = Vec::new();
    write_frame(&big, DEFAULT_MAX_FRAME, &mut out).unwrap();
    let len = u32::from_be_bytes(out[..4].try_into().unwrap()) as usize;
    assert!(len > 65_535, "needs more than two bytes of length");
    assert_eq!(len, out.len() - HEADER_LEN);
    assert_eq!(Message::decode(&out[HEADER_LEN..]).unwrap(), big);
}

/// Spec, Framed self-describing messages: a well-formed request and its reply
/// round-trip without loss and the reply carries the request's id.
#[test]
fn well_formed_request_and_reply_round_trip() {
    let request = Message::Request(Request {
        id: 7,
        deadline_ms: Some(500),
        call: Call::Snapshot(SnapshotParams::default()),
    });
    let mut wire = Vec::new();
    write_frame(&request, DEFAULT_MAX_FRAME, &mut wire).unwrap();

    let mut agent = FrameDecoder::new(DEFAULT_MAX_FRAME);
    agent.push(&wire);
    let received = drain(&mut agent);
    let [Ok(Message::Request(req))] = received.as_slice() else {
        panic!("expected one request, got {received:?}")
    };
    assert_eq!(Message::Request(req.clone()), request);

    let reply = screenshot_response(req.id, 3000);
    let mut back = Vec::new();
    write_frame(&reply, DEFAULT_MAX_FRAME, &mut back).unwrap();
    let mut host = FrameDecoder::new(DEFAULT_MAX_FRAME);
    host.push(&back);
    let received = drain(&mut host);
    let [Ok(Message::Response(resp))] = received.as_slice() else {
        panic!("expected one response, got {received:?}")
    };
    assert_eq!(resp.id, 7);
    assert_eq!(Message::Response(resp.clone()), reply);
}

#[test]
fn several_frames_in_one_read_come_out_in_order() {
    let messages: Vec<Message> = (0..5)
        .map(|n| Message::Ping(Ping { nonce: n }))
        .chain([screenshot_response(9, 500)])
        .collect();
    let mut wire = Vec::new();
    for m in &messages {
        write_frame(m, DEFAULT_MAX_FRAME, &mut wire).unwrap();
    }
    let mut decoder = FrameDecoder::new(DEFAULT_MAX_FRAME);
    decoder.push(&wire);
    let got: Vec<Message> = drain(&mut decoder)
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(got, messages);
    assert_eq!(decoder.buffered(), 0);
}

#[test]
fn partial_frames_are_buffered_until_complete() {
    let message = screenshot_response(3, 20_000);
    let mut wire = Vec::new();
    write_frame(&message, DEFAULT_MAX_FRAME, &mut wire).unwrap();
    write_frame(
        &Message::Ping(Ping { nonce: 1 }),
        DEFAULT_MAX_FRAME,
        &mut wire,
    )
    .unwrap();

    // One byte at a time: nothing comes out before the last byte of each frame.
    let mut decoder = FrameDecoder::new(DEFAULT_MAX_FRAME);
    let first_end = wire.len() - 9;
    let mut got = Vec::new();
    for (i, byte) in wire.iter().enumerate() {
        decoder.push(std::slice::from_ref(byte));
        let frames = drain(&mut decoder);
        if i + 1 == first_end || i + 1 == wire.len() {
            assert_eq!(frames.len(), 1, "a frame completes at byte {i}");
        } else {
            assert!(frames.is_empty(), "no frame may complete at byte {i}");
        }
        got.extend(frames.into_iter().map(Result::unwrap));
    }
    assert_eq!(got, [message.clone(), Message::Ping(Ping { nonce: 1 })]);

    // A header split across reads, then the payload in uneven chunks.
    let mut decoder = FrameDecoder::new(DEFAULT_MAX_FRAME);
    let mut got = Vec::new();
    for chunk in [&wire[..2], &wire[2..3], &wire[3..1000], &wire[1000..]] {
        decoder.push(chunk);
        got.extend(drain(&mut decoder).into_iter().map(Result::unwrap));
    }
    assert_eq!(got, [message, Message::Ping(Ping { nonce: 1 })]);
}

#[test]
fn a_frame_at_exactly_the_limit_is_accepted() {
    let message = screenshot_response(4, 5000);
    let size = message.to_vec().len() as u32;
    let mut wire = Vec::new();
    write_frame(&message, size, &mut wire).unwrap();
    let mut decoder = FrameDecoder::new(size);
    decoder.push(&wire);
    assert_eq!(drain(&mut decoder), [Ok(message)]);
}

#[test]
fn the_sender_refuses_a_frame_over_the_limit_and_writes_nothing() {
    let message = screenshot_response(5, 5000);
    let size = message.to_vec().len();
    let mut out = vec![1, 2, 3];
    let err = write_frame(&message, 4096, &mut out).unwrap_err();
    assert_eq!(err, FrameTooLarge { size, limit: 4096 });
    assert_eq!(
        out,
        [1, 2, 3],
        "nothing of the refused frame may be written"
    );
    let text = err.to_string();
    assert!(text.contains("4096"), "{text}");
    assert!(text.contains(&size.to_string()), "{text}");
}

/// Spec, Framed self-describing messages: an oversized frame is refused with
/// an error naming the limit and the request, and no partial payload is
/// delivered.
#[test]
fn an_oversized_frame_is_refused_naming_the_limit_and_the_request() {
    let big = screenshot_response(42, 10_000);
    let mut wire = Vec::new();
    write_frame(&big, DEFAULT_MAX_FRAME, &mut wire).unwrap();
    let size = (wire.len() - HEADER_LEN) as u32;
    write_frame(
        &Message::Ping(Ping { nonce: 9 }),
        DEFAULT_MAX_FRAME,
        &mut wire,
    )
    .unwrap();

    let mut decoder = FrameDecoder::new(4096);
    decoder.push(&wire);
    let got = drain(&mut decoder);
    assert_eq!(
        got,
        [
            Err(Oversized {
                size,
                limit: 4096,
                kind: Some(kind::RESPONSE),
                id: Some(42),
                method: Some(Method::SCREENSHOT),
            }),
            Ok(Message::Ping(Ping { nonce: 9 })),
        ]
    );
    let text = got[0].as_ref().unwrap_err().to_string();
    assert!(text.contains("4096"), "{text}");
    assert!(
        text.contains("screenshot response for request 42"),
        "{text}"
    );
}

#[test]
fn an_oversized_frame_arriving_in_pieces_is_discarded_and_the_stream_continues() {
    let big = screenshot_response(43, 50_000);
    let mut wire = Vec::new();
    write_frame(&big, DEFAULT_MAX_FRAME, &mut wire).unwrap();
    let request = Message::Request(Request {
        id: 44,
        deadline_ms: None,
        call: Call::Snapshot(SnapshotParams::default()),
    });
    write_frame(&request, DEFAULT_MAX_FRAME, &mut wire).unwrap();

    let mut decoder = FrameDecoder::new(4096);
    let mut got = Vec::new();
    let mut discarding_seen = false;
    for chunk in wire.chunks(1000) {
        decoder.push(chunk);
        got.extend(drain(&mut decoder));
        discarding_seen |= decoder.is_discarding();
        assert!(
            decoder.buffered() <= 4096 + HEADER_LEN,
            "the oversized payload is not buffered"
        );
    }
    assert!(discarding_seen);
    assert!(!decoder.is_discarding());
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(matches!(got[0], Err(Oversized { id: Some(43), .. })));
    assert_eq!(got[1], Ok(request));
}

#[test]
fn a_lowered_limit_applies_to_later_frames() {
    let message = screenshot_response(6, 8000);
    let mut wire = Vec::new();
    write_frame(&message, DEFAULT_MAX_FRAME, &mut wire).unwrap();
    let mut decoder = FrameDecoder::new(DEFAULT_MAX_FRAME);
    assert_eq!(decoder.limit(), DEFAULT_MAX_FRAME);
    decoder.set_limit(MIN_MAX_FRAME);
    assert_eq!(decoder.limit(), MIN_MAX_FRAME);
    decoder.push(&wire);
    assert!(matches!(
        drain(&mut decoder).as_slice(),
        [Err(Oversized { id: Some(6), .. })]
    ));
}

#[test]
fn an_oversized_frame_whose_header_cannot_be_read_still_names_the_limit() {
    // A declared length over the limit and a payload that is not CBOR.
    let mut wire = 5000u32.to_be_bytes().to_vec();
    wire.extend(std::iter::repeat_n(0xff, 5000));
    let mut decoder = FrameDecoder::new(4096);
    decoder.push(&wire);
    let got = drain(&mut decoder);
    assert_eq!(
        got,
        [Err(Oversized {
            size: 5000,
            limit: 4096,
            kind: None,
            id: None,
            method: None,
        })]
    );
    assert!(got[0].as_ref().unwrap_err().to_string().contains("4096"));
}
