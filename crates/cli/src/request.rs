//! Turns a parsed command into the daemon request, built from the request
//! types of `crates/wire`: the action with its parameters, in the envelope
//! the settings fill. A request leaves a parameter at its default out of
//! its line, which the daemon reads the same.

use std::path::Path;

use serde_json::{Map, Value};

use crate::Settings;
use crate::args::{Command, DiffWhat, GetWhat, SetWhat};
use crate::client::Failure;
use crate::code::INVALID_ARGUMENT;

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new(INVALID_ARGUMENT, message)
}

fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Whether an argument looks like a ref rather than a selector or text.
pub(crate) fn looks_like_ref(text: &str) -> bool {
    let text = text.trim();
    let rest = text
        .strip_prefix('@')
        .or_else(|| text.strip_prefix("ref="))
        .unwrap_or(text);
    text.starts_with('@')
        || text.starts_with("ref=")
        || rest
            .strip_prefix('e')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The action and parameters of `command`; `cwd` resolves relative paths.
pub fn build(command: &Command, cwd: &Path) -> Result<wire::Command, Failure> {
    Ok(match command {
        Command::Batch(_) => return Err(invalid("batch runs its commands one by one")),
        Command::Help(_) => return Err(invalid("help is answered without the daemon")),
        Command::Doctor => return Err(invalid("doctor runs its checks on this host")),
        Command::Set(set) => {
            let SetWhat::Timeout { ms } = &set.what;
            wire::Command::SetTimeout(wire::SetTimeout {
                ms: Some(*ms).into(),
            })
        }
        Command::Devices => wire::Command::Devices(wire::Devices {}),
        Command::Close => wire::Command::Close(wire::Close::default()),
        Command::Status => wire::Command::Status(wire::Status {}),
        Command::Shutdown => wire::Command::Shutdown(wire::Shutdown {}),
        Command::Open(open) => wire::Command::Open(wire::Open {
            app: open.app.clone(),
            relaunch: open.relaunch,
            env: open.env.clone().into(),
            timeout: open.timeout.into(),
        }),
        Command::Snapshot(snapshot) => wire::Command::Snapshot(wire::Snapshot {
            interactive: snapshot.interactive,
            max_depth: snapshot.depth.into(),
            delta: snapshot.delta,
            full: snapshot.full,
        }),
        Command::Click(click) => {
            let mut args = wire::Click {
                hold: click.hold.into(),
                timeout: click.timeout.into(),
                ..wire::Click::default()
            };
            match click.target.as_slice() {
                [target] => args.target = Some(target.clone()),
                [x, y] => {
                    let (Some(x), Some(y)) = (number(x), number(y)) else {
                        return Err(invalid(format!(
                            "click takes one target, or x and y in points; {x:?} {y:?} are not numbers"
                        )));
                    };
                    args.x = Some(x).into();
                    args.y = Some(y).into();
                }
                _ => return Err(invalid("click takes one target, or x and y in points")),
            }
            wire::Command::Click(args)
        }
        Command::Fill(fill) => wire::Command::Fill(wire::Fill {
            target: fill.target.clone(),
            text: Some(fill.text.clone()),
            timeout: fill.timeout.into(),
        }),
        Command::Type(typed) => {
            let (target, text) = match typed.args.as_slice() {
                [text] if looks_like_ref(text) => {
                    return Err(invalid(format!(
                        "type {text} needs the text to type after it"
                    )));
                }
                [text] => (None, text.clone()),
                [target, text] => (Some(target.clone()), text.clone()),
                _ => return Err(invalid("type takes [TARGET] TEXT")),
            };
            wire::Command::Type(wire::TypeText {
                text: Some(text),
                target,
                timeout: typed.timeout.into(),
            })
        }
        Command::Press(press) => wire::Command::Press(wire::Press {
            key: press.key.clone(),
        }),
        Command::Swipe(swipe) => {
            let mut args = wire::Swipe::default();
            match swipe.args.as_slice() {
                [direction] => args.direction = direction.clone(),
                [direction, distance] => {
                    let distance = number(distance).ok_or_else(|| {
                        invalid(format!("{distance:?} is not a distance in points"))
                    })?;
                    args.direction = direction.clone();
                    args.distance = distance.into();
                }
                [x1, y1, x2, y2] => {
                    let points: Option<Vec<f64>> =
                        [x1, y1, x2, y2].iter().map(|v| number(v)).collect();
                    let points =
                        points.ok_or_else(|| invalid("swipe X1 Y1 X2 Y2 takes four numbers"))?;
                    args.from = Some([points[0], points[1]]).into();
                    args.to = Some([points[2], points[3]]).into();
                }
                _ => {
                    return Err(invalid(
                        "swipe takes DIRECTION [DISTANCE], or X1 Y1 X2 Y2 in points",
                    ));
                }
            }
            if let Some(duration) = swipe.duration {
                args.duration = duration.into();
            }
            wire::Command::Swipe(args)
        }
        Command::Scroll(scroll) => {
            let mut args = wire::Scroll {
                target: scroll.selector.clone(),
                until: scroll.until.clone(),
                timeout: scroll.timeout.into(),
                ..wire::Scroll::default()
            };
            if let Some(direction) = &scroll.direction {
                args.direction = direction.clone();
            }
            if let Some(amount) = scroll.amount {
                args.amount = amount.into();
            }
            wire::Command::Scroll(args)
        }
        Command::Screenshot(shot) => {
            let mut args = wire::Screenshot {
                path: shot
                    .path
                    .as_ref()
                    .map(|path| cwd.join(path).to_string_lossy().into_owned()),
                ..wire::Screenshot::default()
            };
            if let Some(quality) = shot.quality {
                args.quality = u64::from(quality).into();
            }
            if let Some(px) = shot.max_dimension {
                args.max_dimension = u64::from(px).into();
            }
            wire::Command::Screenshot(args)
        }
        Command::Wait(wait) => {
            let mut args = wire::Wait {
                timeout: wait.timeout.into(),
                ..wire::Wait::default()
            };
            if wait.settle {
                args.settle = true;
                if let Some(quiet) = wait.quiet {
                    args.quiet = quiet.into();
                }
                args.tree_only = wait.tree_only;
            } else if let Some(text) = &wait.text {
                args.text = Some(text.clone());
            } else if let Some(target) = &wait.target {
                match target.parse::<u64>() {
                    Ok(ms) => args.ms = Some(ms).into(),
                    Err(_) => args.target = Some(target.clone()),
                }
            } else {
                return Err(invalid(
                    "wait takes an element, a number of milliseconds, --text or --settle",
                ));
            }
            if let Some(state) = &wait.state {
                args.state = state.clone();
            }
            wire::Command::Wait(args)
        }
        Command::Get(get) => {
            let (what, element) = match &get.what {
                GetWhat::Text(element) => ("text", element),
                GetWhat::Value(element) => ("value", element),
                GetWhat::Box(element) => ("box", element),
            };
            wire::Command::Get(wire::Get {
                what: what.to_owned(),
                target: element.target.clone(),
                timeout: element.timeout.into(),
            })
        }
        Command::Diff(diff) => {
            let DiffWhat::Snapshot(snapshot) = &diff.what;
            wire::Command::DiffSnapshot(wire::DiffSnapshot {
                interactive: snapshot.interactive,
                max_depth: snapshot.depth.into(),
            })
        }
    })
}

/// The request for `command`, with `id`, and the settings' session, target,
/// force flag and default timeout.
pub fn request(
    command: &Command,
    settings: &Settings,
    cwd: &Path,
    id: String,
) -> Result<wire::Request, Failure> {
    let mut request = bare(build(command, cwd)?, &settings.session, id);
    request.platform = settings
        .platform
        .as_ref()
        .map(|platform| platform.to_ascii_lowercase());
    request.device = settings.device.clone();
    request.force = settings.force;
    request.default_timeout = settings.default_timeout.into();
    Ok(request)
}

/// The request for `command` in `session`, with `id` and no target named.
pub fn bare(command: wire::Command, session: &str, id: String) -> wire::Request {
    let mut request = wire::Request::new(command);
    request.id = id.into();
    request.session = session.to_owned();
    request
}

/// The JSON object `request` goes out as.
pub fn line(request: &wire::Request) -> Map<String, Value> {
    match serde_json::to_value(request) {
        Ok(Value::Object(line)) => line,
        other => unreachable!("a request serializes to an object, not {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Settings;
    use crate::args::Cli;
    use clap::Parser;

    /// The line the CLI sent for each command and option before it built its
    /// requests from the request types, with the id it gave them.
    const LINES: &str = include_str!("../../wire/tests/lines.json");

    /// The settings `global` gives, without a settings file.
    fn settings(global: &crate::args::Global) -> Settings {
        Settings {
            json: global.json,
            session: global
                .session
                .clone()
                .unwrap_or_else(|| "default".to_owned()),
            platform: global.platform.clone(),
            device: global.device.clone(),
            force: global.force,
            default_timeout: global.default_timeout,
            endpoint: crate::client::Endpoint {
                dir: "/work".into(),
                daemon: None,
                idle_timeout: None,
                emulator_controller: None,
            },
        }
    }

    /// `line` less the parameters it gives at their default, which the
    /// request types leave out of a line: those with which the action alone
    /// reads as without them.
    fn less_defaults(line: &Map<String, Value>) -> Map<String, Value> {
        let alone = |entry: Option<(&String, &Value)>| -> wire::Request {
            let mut alone = Map::new();
            alone.insert("action".into(), line["action"].clone());
            if let Some((key, value)) = entry {
                alone.insert(key.clone(), value.clone());
            }
            serde_json::from_value(Value::Object(alone)).expect("the line reads")
        };
        let bare = alone(None);
        line.iter()
            .filter(|(key, value)| {
                matches!(key.as_str(), "action" | "id" | "session")
                    || alone(Some((key, value))) != bare
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }

    #[test]
    fn every_command_sends_the_line_it_sent_before_less_the_parameters_at_their_default() {
        let cases: Vec<Value> = serde_json::from_str(LINES).expect("lines.json is JSON");
        for case in cases {
            let argv: Vec<String> = serde_json::from_value(case["argv"].clone()).unwrap();
            let mut words = vec!["mobile-agent".to_owned()];
            words.extend(argv.iter().cloned());
            let cli = Cli::try_parse_from(&words).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
            let settings = settings(&cli.global);
            let id = "r123456".to_owned();
            let request = match &cli.command {
                // doctor asks the daemon with a request of its own.
                crate::args::Command::Doctor => bare(
                    wire::Command::Doctor(wire::Doctor {}),
                    &settings.session,
                    id,
                ),
                command => request(command, &settings, Path::new("/work"), id)
                    .unwrap_or_else(|e| panic!("{argv:?}: {e:?}")),
            };
            let expected = less_defaults(case["line"].as_object().expect("a line"));
            assert_eq!(
                Value::Object(line(&request)),
                Value::Object(expected),
                "{argv:?}"
            );
        }
    }

    #[test]
    fn a_lone_ref_is_not_text_to_type() {
        for text in ["@e3", "ref=e3", "e3", "@nope"] {
            assert!(looks_like_ref(text), "{text}");
        }
        for text in ["hello", "e", "else", "3"] {
            assert!(!looks_like_ref(text), "{text}");
        }
    }
}
