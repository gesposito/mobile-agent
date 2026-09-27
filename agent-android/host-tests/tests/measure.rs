//! Measurements for the report, not checks: run with
//! `MOBILE_AGENT_ANDROID_SERIAL=<serial> cargo test --test measure -- --ignored --nocapture --test-threads 1`.

use std::time::{Duration, Instant};

use agent_android_host_tests::agent::fields;
use agent_android_host_tests::client::Client;
use agent_android_host_tests::tree::{self, center, find};
use agent_android_host_tests::{Agent, Launch, device, fixture, ms, percentile};
use protocol::event::Stream;
use protocol::message::{
    BoundsParams, Call, Reply, ResyncParams, SnapshotParams, TapParams, TextMethod, TextParams,
};

fn summary(label: &str, values: &[f64]) {
    println!(
        "{label}: n={} p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
        values.len(),
        percentile(values, 50.0),
        percentile(values, 95.0),
        percentile(values, 100.0)
    );
}

fn no_wait() -> Call {
    Call::Snapshot(SnapshotParams {
        idle_ms: Some(0),
        ..SnapshotParams::default()
    })
}

#[test]
#[ignore]
fn measure_attach() {
    let Some(dev) = device("measure_attach") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let (mut ready, mut handshake, mut first) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..10 {
        let mut agent = Agent::launch(&dev, Launch::default());
        let t0 = Instant::now();
        let mut client = agent.connect();
        handshake.push(ms(t0.elapsed()));
        let t1 = Instant::now();
        assert!(matches!(client.call(no_wait()), Ok(Reply::Snapshot(_))));
        first.push(ms(t1.elapsed()));
        ready.push(ms(agent.ready_after));
        let _ = client.call(Call::Shutdown);
        agent.wait_exit(Duration::from_secs(5));
    }
    summary("spawn to ready line", &ready);
    summary("connect and handshake", &handshake);
    summary("first snapshot, Compose Login", &first);
}

fn reads(package: &str) {
    let Some(dev) = device("measure reads") else {
        return;
    };
    fixture::launch(&dev, package, "big");
    let mut agent = Agent::launch(
        &dev,
        Launch::default()
            .arg("--trace")
            .arg("--compose-poll-ms")
            .arg("0"),
    );
    let mut client = agent.connect();
    let sample = |client: &mut Client, agent: &mut Agent| {
        agent.drain_lines();
        let t0 = Instant::now();
        assert!(matches!(client.call(no_wait()), Ok(Reply::Snapshot(_))));
        let host = ms(t0.elapsed());
        let line = agent
            .wait_line(Duration::from_secs(2), |l| l.starts_with("trace snapshot "))
            .expect("trace");
        let f = fields(&line);
        (
            host,
            f["read_ms"].parse::<f64>().unwrap(),
            f["round_trips"].parse::<f64>().unwrap(),
        )
    };
    let (mut cold_host, mut cold_read, mut cold_rt) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..10 {
        // A tree resync empties the node cache.
        let _ = client.call(Call::Resync(ResyncParams {
            stream: Stream::TREE,
        }));
        let (h, r, t) = sample(&mut client, &mut agent);
        cold_host.push(h);
        cold_read.push(r);
        cold_rt.push(t);
    }
    for _ in 0..3 {
        sample(&mut client, &mut agent);
    }
    let (mut warm_host, mut warm_read, mut warm_rt) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..20 {
        let (h, r, t) = sample(&mut client, &mut agent);
        warm_host.push(h);
        warm_read.push(r);
        warm_rt.push(t);
    }
    summary(&format!("{package} Big cold, agent read"), &cold_read);
    summary(&format!("{package} Big cold, host round trip"), &cold_host);
    println!(
        "{package} Big cold round trips p50 {}",
        percentile(&cold_rt, 50.0)
    );
    summary(&format!("{package} Big warm, agent read"), &warm_read);
    summary(&format!("{package} Big warm, host round trip"), &warm_host);
    println!(
        "{package} Big warm round trips p50 {}",
        percentile(&warm_rt, 50.0)
    );
}

#[test]
#[ignore]
fn measure_reads_compose() {
    reads(fixture::COMPOSE);
}

#[test]
#[ignore]
fn measure_reads_expo() {
    reads(fixture::EXPO);
}

#[test]
#[ignore]
fn measure_taps() {
    let Some(dev) = device("measure_taps") else {
        return;
    };
    fixture::launch(&dev, fixture::COMPOSE, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = match client.call(no_wait()) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("{other:?}"),
    };
    let (x, y) = tree::nodes(&s)
        .into_iter()
        .find(|(f, _)| f.node.text.as_deref() == Some("Login") && f.depth > 3)
        .map(|(f, _)| center(f.node.bounds))
        .expect("title");
    let mut acks = Vec::new();
    for i in 0..220 {
        let t0 = Instant::now();
        let reply = client.call(Call::Tap(TapParams {
            x,
            y,
            duration_ms: None,
        }));
        assert!(
            matches!(reply, Ok(Reply::Input(ref a)) if a.accepted),
            "{reply:?}"
        );
        if i >= 20 {
            acks.push(ms(t0.elapsed()));
        }
    }
    summary("tap acknowledgement, host round trip", &acks);
    println!(
        "taps over 50 ms: {}",
        acks.iter().filter(|&&a| a > 50.0).count()
    );
}

#[test]
#[ignore]
fn measure_text() {
    let Some(dev) = device("measure_text") else {
        return;
    };
    fixture::launch(&dev, fixture::EXPO, "login");
    let agent = Agent::launch(&dev, Launch::default());
    let mut client = agent.connect();
    let s = match client.call(no_wait()) {
        Ok(Reply::Snapshot(s)) => s,
        other => panic!("{other:?}"),
    };
    let (field, _) = find(&s, "login-username").expect("field");
    let (x, y) = tree::tap_point(&s, &field);
    let _ = client.call(Call::Tap(TapParams {
        x,
        y,
        duration_ms: None,
    }));
    client.idle(Duration::from_millis(800));
    let text = "forty characters of text set in one go!!";
    let mut run = |mode: Option<TextMethod>, n: usize| {
        let mut times = Vec::new();
        for _ in 0..n {
            let t0 = Instant::now();
            let reply = client.call(Call::TextSet(TextParams {
                text: text.to_owned(),
                node: None,
                mode,
            }));
            assert!(matches!(reply, Ok(Reply::Text(_))), "{reply:?}");
            times.push(ms(t0.elapsed()));
            client.idle(Duration::from_millis(200));
        }
        times
    };
    let direct = run(None, 20);
    let keys = run(Some(TextMethod::KEYS), 5);
    fixture::force_stop(&dev, fixture::EXPO);
    summary("40 characters, direct", &direct);
    summary("40 characters, keys", &keys);
}

/// Task 10.11: what a bounds probe costs on a still screen, per request of
/// 1, 2, 4 and 8 nodes, in the agent and from the host: the Expo Login
/// screen and the Compose Big screen (500 cells under one parent).
#[test]
#[ignore]
fn measure_bounds_probes() {
    let Some(dev) = device("measure_bounds_probes") else {
        return;
    };
    let screens: [(&str, &str, [&str; 8]); 2] = [
        (
            fixture::EXPO,
            "login",
            [
                "login-username",
                "tab-system",
                "login-submit",
                "tab-login",
                "login-password",
                "tab-big",
                "tab-list",
                "tab-anim",
            ],
        ),
        (
            fixture::COMPOSE,
            "big",
            [
                "cell-1",
                "cell-500",
                "cell-250",
                "tab-login",
                "cell-125",
                "cell-375",
                "tab-big",
                "cell-60",
            ],
        ),
    ];
    for (package, tab, ids) in screens {
        fixture::launch(&dev, package, tab);
        let mut agent = Agent::launch(
            &dev,
            Launch::default()
                .arg("--trace")
                .arg("--compose-poll-ms")
                .arg("0"),
        );
        let mut client = agent.connect();
        client.idle(Duration::from_millis(1000));
        let Ok(Reply::Snapshot(screen)) = client.call(no_wait()) else {
            panic!("no snapshot");
        };
        let nodes: Vec<u32> = ids
            .iter()
            .map(|id| {
                find(&screen, id)
                    .unwrap_or_else(|| panic!("no {id}"))
                    .0
                    .node
                    .id
            })
            .collect();
        for count in [1, 2, 4, 8] {
            let (mut host, mut inside) = (Vec::new(), Vec::new());
            for _ in 0..30 {
                agent.drain_lines();
                let t0 = Instant::now();
                let reply = client.call(Call::Bounds(BoundsParams {
                    nodes: nodes[..count].to_vec(),
                }));
                host.push(ms(t0.elapsed()));
                assert!(
                    matches!(&reply, Ok(Reply::Bounds(p)) if p.nodes.len() == count),
                    "{reply:?}"
                );
                let line = agent
                    .wait_line(Duration::from_secs(2), |l| l.starts_with("trace bounds "))
                    .expect("trace");
                inside.push(fields(&line)["us"].parse::<f64>().unwrap() / 1e3);
            }
            summary(
                &format!("{package} {tab}: probe of {count} nodes, in the agent"),
                &inside,
            );
            summary(
                &format!("{package} {tab}: probe of {count} nodes, host round trip"),
                &host,
            );
        }
        let _ = client.call(Call::Shutdown);
        agent.wait_exit(Duration::from_secs(5));
        fixture::force_stop(&dev, package);
    }
}
