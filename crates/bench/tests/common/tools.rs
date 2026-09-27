//! Fake Maestro, maestro-runner and agent-device that print their tools' own
//! output shapes, and the screens the reference flows end on as every tool
//! reads them.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use bench::baseline::BaselineConfig;
use bench::target::Platform;
use serde_json::{Value, json};

/// An element as every tool reads it: its identifier and the text it shows.
pub type Shown = (Option<String>, String);

/// What the Expo fixture shows at the end of the reference flow `name`.
pub fn end_screen(name: &str) -> Vec<Shown> {
    let mut screen: Vec<Shown> = Vec::new();
    let mut add = |id: &str, text: &str| screen.push((Some(id.to_owned()), text.to_owned()));
    match name {
        "login" => {
            add("login-title", "Login");
            add("login-username", "agent");
            add("login-password", "•••••••");
            add("login-submit", "Sign in");
            add("login-status", "Welcome, agent");
        }
        "list-scroll" => {
            add("detail-200", "Detail 200");
            add("detail-back", "Back");
        }
        "big-screen" => {
            add("big-count", "500 elements");
            for n in 1..=500 {
                add(&format!("cell-{n}"), &format!("Cell {n}"));
            }
        }
        _ => {}
    }
    for (id, label) in [
        ("tab-login", "Login"),
        ("tab-list", "List"),
        ("tab-big", "Big"),
        ("tab-anim", "Anim"),
        ("tab-system", "System"),
    ] {
        add(id, label);
    }
    screen.push((None, "12:30".to_owned()));
    screen
}

/// `maestro hierarchy` for `screen` on `platform`: the root, the app's
/// window, then a node per element with Maestro's attributes for the
/// platform: the text in `text` on Android, the label in `accessibilityText`
/// on iOS.
pub fn maestro_hierarchy(screen: &[Shown], platform: Platform) -> String {
    let node = |id: &str, text: &str| match platform {
        Platform::Android => json!({
            "attributes": {"text": text, "resource-id": id, "bounds": "[0,0][10,10]",
                           "class": "android.widget.TextView", "enabled": "true"},
            "children": [], "clickable": false, "enabled": true, "focused": false,
            "checked": false, "selected": false
        }),
        Platform::Ios => json!({
            "attributes": {"accessibilityText": text, "title": "", "value": "", "text": "",
                           "hintText": "", "resource-id": id, "bounds": "[0,0][10,10]",
                           "enabled": "true", "focused": "false", "selected": "false",
                           "checked": "false"},
            "children": [], "enabled": true
        }),
    };
    let elements: Vec<Value> = screen
        .iter()
        .map(|(id, text)| node(id.as_deref().unwrap_or(""), text))
        .collect();
    let root = json!({"attributes": {}, "children": [{
        "attributes": {"resource-id": "", "text": "", "bounds": "[0,0][1080,2400]"},
        "children": elements
    }]});
    serde_json::to_string_pretty(&root).unwrap()
}

/// `maestro-runner hierarchy` for `screen`: its normalized tree.
pub fn runner_hierarchy(screen: &[Shown]) -> String {
    let elements: Vec<Value> = screen
        .iter()
        .map(|(id, text)| {
            let mut node = json!({"type": "TextView", "text": text,
                                  "bounds": {"x": 0, "y": 0, "width": 10, "height": 10}});
            if let Some(id) = id {
                node["id"] = id.as_str().into();
            }
            node
        })
        .collect();
    let root = json!({"type": "Application", "text": "FixtureExpo", "children": elements});
    serde_json::to_string_pretty(&root).unwrap()
}

/// `agent-device snapshot --json` for `screen`: an application node, a
/// window, then a node per element, printed as agent-device prints JSON.
pub fn agent_device_snapshot(screen: &[Shown]) -> String {
    let mut nodes = vec![
        json!({"index": 0, "type": "Application", "label": "FixtureExpo", "depth": 0, "ref": "e1"}),
        json!({"index": 1, "type": "Window", "depth": 1, "parentIndex": 0, "ref": "e2"}),
    ];
    for (i, (id, text)) in screen.iter().enumerate() {
        let mut node = json!({"index": i + 2, "type": "StaticText", "label": text, "depth": 2,
                              "parentIndex": 1, "ref": format!("e{}", i + 3),
                              "rect": {"x": 0, "y": 0, "width": 10, "height": 10}});
        if let Some(id) = id {
            node["identifier"] = id.as_str().into();
        }
        nodes.push(node);
    }
    let reply = json!({"success": true, "data": {
        "nodes": nodes, "truncated": false,
        "appName": "FixtureExpo", "appBundleId": "dev.mobileagent.fixture.expo"
    }});
    serde_json::to_string_pretty(&reply).unwrap()
}

/// Fake Maestro, maestro-runner and agent-device in one directory. A flow
/// run keeps the end screen of the flow its file names; a hierarchy read
/// (`hierarchy`, or agent-device's `snapshot`) prints the screen kept, in the
/// tool's shape. Each tool logs its arguments to `<tool>.log`.
pub struct Tools {
    pub dir: PathBuf,
    pub bin: PathBuf,
}

impl Tools {
    /// The tools on `platform`, showing `screen(flow)` after each flow.
    pub fn new(name: &str, platform: Platform, screen: impl Fn(&str) -> Vec<Shown>) -> Self {
        let dir = super::scratch(name);
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let tools = Tools { dir, bin };
        let flows = ["login", "list-scroll", "big-screen"];
        for flow in flows {
            let shown = screen(flow);
            tools.write(
                &format!("maestro-{flow}.out"),
                &maestro_hierarchy(&shown, platform),
            );
            tools.write(
                &format!("maestro-runner-{flow}.out"),
                &runner_hierarchy(&shown),
            );
            tools.write(
                &format!("agent-device-{flow}.out"),
                &agent_device_snapshot(&shown),
            );
        }
        tools.write(
            "agent-device-batch.out",
            r#"{"success":true,"data":{"total":3,"executed":3,"totalDurationMs":42,"results":[]}}"#,
        );
        let maestro_passes = "echo '[Passed] flow'";
        let batch_passes = "cat \"$dir/agent-device-batch.out\"";
        tools.tool("maestro", "2.10.0", "hierarchy", maestro_passes);
        tools.tool("maestro-runner", "1.1.0", "hierarchy", maestro_passes);
        tools.tool("agent-device", "0.21.14", "snapshot", batch_passes);
        tools
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.bin.join(name), contents).unwrap();
    }

    fn tool(&self, name: &str, version: &str, read: &str, passes: &str) {
        let mut runs = String::new();
        for flow in ["login", "list-scroll", "big-screen"] {
            runs.push_str(&format!(
                "*-{flow}-*) cp \"$dir/{name}-{flow}.out\" \"$dir/{name}-screen.out\"; {passes} ;;\n"
            ));
        }
        let path = self.bin.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\n\
                 dir=\"${{0%/*}}\"\n\
                 if [ \"$1\" = \"--version\" ]; then echo '{version}'; exit 0; fi\n\
                 echo \"$@\" >> \"$dir/{name}.log\"\n\
                 case \" $* \" in\n\
                 *\" {read} \"*) cat \"$dir/{name}-screen.out\" ;;\n\
                 {runs}\
                 *) {passes} ;;\n\
                 esac\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    pub fn config(&self) -> BaselineConfig {
        BaselineConfig {
            search_path: vec![self.bin.clone()],
            work_dir: self.dir.join("work"),
            flow_timeout: Duration::from_secs(30),
            version_timeout: Duration::from_secs(10),
        }
    }

    /// Each line `tool` logged, reduced to what it did: `run <flow>` for a
    /// flow run, `read` for a hierarchy read.
    pub fn log(&self, tool: &str) -> Vec<String> {
        let log = fs::read_to_string(self.bin.join(format!("{tool}.log"))).unwrap_or_default();
        log.lines()
            .map(|line| {
                if line.contains(" hierarchy") || line.starts_with("snapshot ") {
                    "read".to_owned()
                } else {
                    let flow = ["login", "list-scroll", "big-screen", "tab"]
                        .into_iter()
                        .find(|flow| line.contains(&format!("-{flow}-")))
                        .unwrap_or("?");
                    format!("run {flow}")
                }
            })
            .collect()
    }
}
