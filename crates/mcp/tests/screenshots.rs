//! Screenshots (task 5.3 of the `mcp-server` change, design D6): the
//! screenshot tool answers with image content, the JPEG the daemon returns
//! inline, its long edge at most 1000 px, and the text of its pixels beside
//! the screen's points; with `ifChanged`, a line instead when the image is
//! the session's last; with `path`, the path and the sizes, the daemon
//! having written the file.

mod support;

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use support::{APP, Client, ERAS, Raw, TestEnv, ok, text};

/// The image content of a result: its type and its bytes.
#[track_caller]
fn image(result: &Value) -> (String, Vec<u8>) {
    assert!(result["isError"] != true, "{result}");
    let content = result["content"].as_array().expect("content");
    let images: Vec<&Value> = content.iter().filter(|b| b["type"] == "image").collect();
    assert_eq!(images.len(), 1, "one image: {result}");
    let block = images[0];
    let bytes = STANDARD
        .decode(block["data"].as_str().expect("the image's data"))
        .expect("the data is base64");
    (
        block["mimeType"].as_str().unwrap_or_default().to_owned(),
        bytes,
    )
}

/// The size the fake's placeholder JPEG names: `jpeg <w>x<h> q<quality>
/// frame <n>` (crates/daemon/README.md, The fake driver).
#[track_caller]
fn size(bytes: &[u8]) -> (u32, u32) {
    let placeholder = String::from_utf8_lossy(bytes);
    let named = placeholder
        .split_whitespace()
        .nth(1)
        .and_then(|size| size.split_once('x'))
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
    named.unwrap_or_else(|| panic!("not the fake's placeholder: {placeholder:?}"))
}

#[test]
fn a_screenshot_is_the_daemon_s_image_within_the_long_edge_bound() {
    for era in ERAS {
        let env = TestEnv::start("android");
        let mut client = Client::spawn(&env, era);
        ok(&client.call("open", json!({"app": APP})));
        let result = client.call("screenshot", json!({}));
        let (mime, bytes) = image(&result);
        assert_eq!(mime, "image/jpeg", "{era}");
        // Design D5 of real-app-fixes: the text says the image's pixels
        // beside the screen's points, which a click takes.
        assert_eq!(text(&result), "447x1000 of a 408x912-point screen", "{era}");
        let (width, height) = size(&bytes);
        assert!(
            width.max(height) <= 1000 && width.max(height) > 0,
            "{era}: {width}x{height}"
        );
        // The bytes the daemon gives the same session for the still screen.
        let reply = env.request(json!({
            "action": "screenshot", "session": client.session(), "inline": true,
        }));
        let daemon = STANDARD
            .decode(reply["data"]["image"].as_str().expect("an inline image"))
            .expect("base64");
        assert_eq!(bytes, daemon, "{era}: the daemon's image");
        client.close();
    }
}

#[test]
fn if_changed_answers_a_still_screen_with_a_line_and_a_changed_one_with_the_image() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    ok(&client.call("open", json!({"app": APP})));
    image(&client.call("screenshot", json!({"ifChanged": true})));
    let still = client.call("screenshot", json!({"ifChanged": true}));
    assert_eq!(ok(&still), "unchanged since the last screenshot", "{still}");
    let content = still["content"].as_array().expect("content");
    assert!(content.iter().all(|b| b["type"] == "text"), "{still}");
    // Without ifChanged, the image all the same.
    image(&client.call("screenshot", json!({})));
    ok(&client.call("click", json!({"selector": "#tab-list"})));
    ok(&client.call("wait", json!({"settle": true})));
    image(&client.call("screenshot", json!({"ifChanged": true})));
    let again = client.call("screenshot", json!({"ifChanged": true}));
    assert_eq!(ok(&again), "unchanged since the last screenshot");
    client.close();
}

/// The path a screenshot's answer names, on its first line, with the sizes
/// on the second (design D5 of real-app-fixes).
#[track_caller]
fn path_and_sizes(answer: &str) -> &str {
    let (path, sizes) = answer.split_once('\n').expect("two lines");
    assert_eq!(sizes, "447x1000 of a 408x912-point screen", "{answer}");
    path
}

#[test]
fn a_screenshot_with_a_path_answers_with_the_path_and_the_sizes() {
    let env = TestEnv::start("android");
    let mut client = Raw::spawn(&env);
    ok(&client.call("open", json!({"app": APP})));
    // A relative path is the server's working directory's, as the CLI's is
    // its own.
    let result = client.call("screenshot", json!({"path": "shot.jpg"}));
    let answer = ok(&result);
    let content = result["content"].as_array().expect("content");
    assert_eq!(content.len(), 1, "one text, no image: {result}");
    let written = env.path().join("shot.jpg");
    assert!(written.is_file(), "the daemon wrote {}", written.display());
    assert_eq!(
        Path::new(path_and_sizes(&answer)).canonicalize().ok(),
        written.canonicalize().ok(),
        "{answer}"
    );
    let absolute = env.path().join("absolute.jpg");
    let named = ok(&client.call("screenshot", json!({"path": absolute.to_string_lossy()})));
    assert_eq!(
        path_and_sizes(&named),
        absolute.to_string_lossy(),
        "{named}"
    );
    assert!(absolute.is_file());
    assert_eq!(
        text(&client.call(
            "screenshot",
            json!({"path": absolute.to_string_lossy(), "ifChanged": true})
        )),
        "unchanged since the last screenshot",
        "ifChanged with a path"
    );
    client.close();
}
