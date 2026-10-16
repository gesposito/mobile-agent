//! `screenshot` with `inline` (design D6 of the `mcp-server` change): the
//! reply carries the image's bytes, base64-encoded, with its type, beside
//! the file a path names; without a path no file is written.

mod support;

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use support::{APP, Daemon, TempDir};

/// The screenshots the daemon wrote to `dir`, its temporary directory, as
/// it names them without a path.
fn written(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("the directory lists")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("mobile-agent-"))
        .collect()
}

/// The image an inline screenshot's reply carries, decoded.
#[track_caller]
fn image(reply: &Value) -> Vec<u8> {
    assert_eq!(reply["success"], true, "{reply}");
    assert_eq!(reply["data"]["mimeType"], "image/jpeg", "{reply}");
    let encoded = reply["data"]["image"].as_str().expect("an image");
    STANDARD.decode(encoded).expect("the image is base64")
}

#[test]
fn an_inline_screenshot_carries_the_image_and_writes_a_file_only_where_a_path_names_one() {
    let temp = TempDir::new();
    let daemon = Daemon::builder()
        .fake("android")
        .env("TMPDIR", &temp.path().to_string_lossy())
        .start();
    let opened = daemon.request(json!({"action": "open", "app": APP}));
    assert_eq!(opened["success"], true, "{opened}");

    let inline = daemon.request(json!({"action": "screenshot", "inline": true}));
    assert!(!image(&inline).is_empty(), "{inline}");
    assert_eq!(inline["data"]["path"], Value::Null, "{inline}");
    assert_eq!(inline["data"]["format"], "jpeg", "{inline}");
    assert_eq!(written(temp.path()), Vec::<String>::new(), "no file");

    let path = temp.path().join("shot.jpg");
    let saved = daemon.request(json!({
        "action": "screenshot",
        "inline": true,
        "path": path.to_string_lossy(),
    }));
    assert_eq!(saved["data"]["path"], path.to_string_lossy().as_ref());
    let file = std::fs::read(&path).expect("the file is written");
    assert_eq!(image(&saved), file, "the reply carries the file's bytes");

    let plain = daemon.request(json!({"action": "screenshot"}));
    assert_eq!(plain["success"], true, "{plain}");
    let data = plain["data"].as_object().expect("data");
    assert!(
        !data.contains_key("image") && !data.contains_key("mimeType"),
        "{plain}"
    );
    assert_eq!(written(temp.path()).len(), 1, "the file without a path");
}
