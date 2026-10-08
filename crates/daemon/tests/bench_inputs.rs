//! The input of the daemon's microbenchmarks (`benches/daemon.rs`, design
//! D14), checked here so that it cannot drift silently: the warm snapshot of
//! the fixture's Big tab that the socket benchmark times, and whose reply the
//! wire benchmark writes, lists the 500 cells and the five tabs.

mod support;

use serde_json::Value;
use support::{Daemon, INTERACTIVE_SNAPSHOT};

/// After `open_big`, the line the CLI sends for `snapshot -i` gets the reply
/// the benchmarks verify: the 500 cells and the five tab buttons, 505
/// elements each with a ref, no ref retired, served from the replica.
#[test]
fn a_warm_snapshot_of_the_big_tab_lists_its_500_cells_and_five_tabs_with_refs() {
    let daemon = Daemon::start("android");
    support::open_big(&daemon);
    let line = support::raw_request(&daemon.socket(), INTERACTIVE_SNAPSHOT);
    let reply: Value = serde_json::from_str(&line).expect("the reply is JSON");
    support::check_big(&reply);
}
