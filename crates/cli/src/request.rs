//! Turns a parsed command into the daemon request: `{"action", ...}` with
//! the parameters the daemon reads. The session, force flag and id are added
//! by the caller.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::args::{Command, DiffWhat, GetWhat, SetWhat};
use crate::client::Failure;

/// A usage error the parser cannot catch.
pub const INVALID_ARGUMENT: &str = "invalid_argument";

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new(INVALID_ARGUMENT, message)
}

/// Sets `key` when `value` is present.
fn put(map: &mut Map<String, Value>, key: &str, value: Option<impl Into<Value>>) {
    if let Some(value) = value {
        map.insert(key.to_owned(), value.into());
    }
}

fn flag(map: &mut Map<String, Value>, key: &str, on: bool) {
    if on {
        map.insert(key.to_owned(), Value::Bool(true));
    }
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

/// The request for `command`; `cwd` resolves relative paths.
pub fn build(command: &Command, cwd: &Path) -> Result<Map<String, Value>, Failure> {
    let mut request = Map::new();
    let action = match command {
        Command::Batch(_) => return Err(invalid("batch runs its commands one by one")),
        Command::Help(_) => return Err(invalid("help is answered without the daemon")),
        Command::Set(set) => {
            let SetWhat::Timeout { ms } = &set.what;
            request.insert("ms".into(), (*ms).into());
            "set_timeout"
        }
        Command::Devices => "devices",
        Command::Close => "close",
        Command::Status => "status",
        Command::Shutdown => "shutdown",
        Command::Open(open) => {
            request.insert("app".into(), open.app.clone().into());
            flag(&mut request, "relaunch", open.relaunch);
            if !open.env.is_empty() {
                request.insert("env".into(), json!(open.env));
            }
            put(&mut request, "timeout", open.timeout);
            "open"
        }
        Command::Snapshot(snapshot) => {
            flag(&mut request, "interactive", snapshot.interactive);
            put(&mut request, "maxDepth", snapshot.depth);
            flag(&mut request, "delta", snapshot.delta);
            flag(&mut request, "full", snapshot.full);
            "snapshot"
        }
        Command::Click(click) => {
            match click.target.as_slice() {
                [target] => {
                    request.insert("target".into(), target.clone().into());
                }
                [x, y] => {
                    let (Some(x), Some(y)) = (number(x), number(y)) else {
                        return Err(invalid(format!(
                            "click takes one target, or x and y in points; {x:?} {y:?} are not numbers"
                        )));
                    };
                    request.insert("x".into(), x.into());
                    request.insert("y".into(), y.into());
                }
                _ => return Err(invalid("click takes one target, or x and y in points")),
            }
            put(&mut request, "hold", click.hold);
            put(&mut request, "timeout", click.timeout);
            "click"
        }
        Command::Fill(fill) => {
            request.insert("target".into(), fill.target.clone().into());
            request.insert("text".into(), fill.text.clone().into());
            put(&mut request, "timeout", fill.timeout);
            "fill"
        }
        Command::Type(typed) => {
            match typed.args.as_slice() {
                [text] if looks_like_ref(text) => {
                    return Err(invalid(format!(
                        "type {text} needs the text to type after it"
                    )));
                }
                [text] => {
                    request.insert("text".into(), text.clone().into());
                }
                [target, text] => {
                    request.insert("target".into(), target.clone().into());
                    request.insert("text".into(), text.clone().into());
                }
                _ => return Err(invalid("type takes [TARGET] TEXT")),
            }
            put(&mut request, "timeout", typed.timeout);
            "type"
        }
        Command::Press(press) => {
            request.insert("key".into(), press.key.clone().into());
            "press"
        }
        Command::Swipe(swipe) => {
            match swipe.args.as_slice() {
                [direction] => {
                    request.insert("direction".into(), direction.clone().into());
                }
                [direction, distance] => {
                    let distance = number(distance).ok_or_else(|| {
                        invalid(format!("{distance:?} is not a distance in points"))
                    })?;
                    request.insert("direction".into(), direction.clone().into());
                    request.insert("distance".into(), distance.into());
                }
                [x1, y1, x2, y2] => {
                    let points: Option<Vec<f64>> =
                        [x1, y1, x2, y2].iter().map(|v| number(v)).collect();
                    let points =
                        points.ok_or_else(|| invalid("swipe X1 Y1 X2 Y2 takes four numbers"))?;
                    request.insert("from".into(), json!([points[0], points[1]]));
                    request.insert("to".into(), json!([points[2], points[3]]));
                }
                _ => {
                    return Err(invalid(
                        "swipe takes DIRECTION [DISTANCE], or X1 Y1 X2 Y2 in points",
                    ));
                }
            }
            put(&mut request, "duration", swipe.duration);
            "swipe"
        }
        Command::Scroll(scroll) => {
            put(&mut request, "direction", scroll.direction.clone());
            put(&mut request, "amount", scroll.amount);
            put(&mut request, "target", scroll.selector.clone());
            put(&mut request, "timeout", scroll.timeout);
            "scroll"
        }
        Command::Screenshot(shot) => {
            if let Some(path) = &shot.path {
                request.insert(
                    "path".into(),
                    cwd.join(path).to_string_lossy().into_owned().into(),
                );
            }
            put(&mut request, "quality", shot.quality);
            put(&mut request, "maxDimension", shot.max_dimension);
            "screenshot"
        }
        Command::Wait(wait) => {
            if wait.settle {
                request.insert("settle".into(), Value::Bool(true));
                put(&mut request, "quiet", wait.quiet);
                flag(&mut request, "treeOnly", wait.tree_only);
            } else if let Some(text) = &wait.text {
                request.insert("text".into(), text.clone().into());
            } else if let Some(target) = &wait.target {
                match target.parse::<u64>() {
                    Ok(ms) => {
                        request.insert("ms".into(), ms.into());
                    }
                    Err(_) => {
                        request.insert("target".into(), target.clone().into());
                    }
                }
            } else {
                return Err(invalid(
                    "wait takes an element, a number of milliseconds, --text or --settle",
                ));
            }
            put(&mut request, "state", wait.state.clone());
            put(&mut request, "timeout", wait.timeout);
            "wait"
        }
        Command::Get(get) => {
            let (what, element) = match &get.what {
                GetWhat::Text(element) => ("text", element),
                GetWhat::Value(element) => ("value", element),
                GetWhat::Box(element) => ("box", element),
            };
            request.insert("what".into(), what.into());
            request.insert("target".into(), element.target.clone().into());
            put(&mut request, "timeout", element.timeout);
            "get"
        }
        Command::Diff(diff) => {
            let DiffWhat::Snapshot(snapshot) = &diff.what;
            flag(&mut request, "interactive", snapshot.interactive);
            put(&mut request, "maxDepth", snapshot.depth);
            "diff_snapshot"
        }
    };
    request.insert("action".into(), action.into());
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

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
