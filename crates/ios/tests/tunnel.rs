//! Task 3.2 on the device: the CoreDevice tunnel opened in this unprivileged
//! process, with no network interface of its own, and the developer services
//! its RSD handshake lists.

#![cfg(target_os = "macos")]

mod common;

use std::process::Command;
use std::time::Instant;

use ios::{Devices, DiskImage, IosDriver, NEEDED};

/// The host's `utun` interfaces, which a kernel tunnel would add to.
fn utun_interfaces() -> Vec<String> {
    let output = Command::new("ifconfig")
        .arg("-l")
        .output()
        .expect("ifconfig runs");
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .filter(|name| name.starts_with("utun"))
        .map(str::to_owned)
        .collect()
}

fn effective_user() -> String {
    let output = Command::new("id").arg("-u").output().expect("id runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn the_tunnel_opens_in_this_unprivileged_process_and_lists_the_developer_services() {
    let Some(udid) = common::gated("the_tunnel_opens") else {
        return;
    };
    let _device = common::exclusive();
    assert_ne!(effective_user(), "0", "the tests run unprivileged");
    let before = utun_interfaces();
    let driver = IosDriver::open().expect("the driver opens");
    let started = Instant::now();
    let connected = driver.connect(&udid).expect("the phone connects");
    let elapsed = started.elapsed();
    assert_eq!(
        utun_interfaces(),
        before,
        "the tunnel adds no network interface"
    );
    assert!(
        matches!(connected.disk_image.image, DiskImage::Mounted { .. }),
        "{:?}",
        connected.disk_image
    );
    assert_eq!(connected.tunnel.opens, 1);
    assert!(
        connected.tunnel.missing.is_empty(),
        "every needed service is listed; missing {:?}",
        connected.tunnel.missing
    );
    for needed in NEEDED {
        assert!(
            connected
                .tunnel
                .services
                .iter()
                .any(|name| name == needed.name),
            "{}",
            needed.name
        );
    }
    eprintln!(
        "connect: {:.1} ms, the tunnel and RSD's handshake {:.1} ms, {} services listed, \
         disk image {:?} with {} mount attempts",
        elapsed.as_secs_f64() * 1e3,
        connected.tunnel.opened_in.as_secs_f64() * 1e3,
        connected.tunnel.services.len(),
        connected.disk_image.image,
        connected.disk_image.attempts.len()
    );

    let again = driver.connect(&udid).expect("connects again");
    assert_eq!(again.tunnel.opens, 1, "the tunnel is kept");
    let status = driver.status(&udid).expect("status");
    assert_eq!(status.tunnel.map(|tunnel| tunnel.opens), Some(1));
    assert_eq!(
        status.services_open,
        vec!["com.apple.instruments.server.services.processcontrol"],
        "DVT's process control is kept"
    );
    driver.reset_target(&udid).expect("resets");
    assert_eq!(driver.status(&udid).expect("status").tunnel, None);
    let reopened = driver.connect(&udid).expect("connects after a reset");
    assert_eq!(
        reopened.tunnel.opens, 1,
        "a new tunnel for the reset target"
    );
    driver.reset_target(&udid).expect("resets");
}
