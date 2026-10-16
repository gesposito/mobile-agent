//! Sessions and claims (design D9 of the `mcp-server` change). The server
//! mints a session of its own, `mcp-<pid>`, which a call's `session`
//! overrides for a client that resumes a transcript with another session's
//! refs, and keeps, for every session a call went out in, the targets its
//! replies named. On its way out, whichever way that is, it sends `close`
//! with `keep` and the target's id for each target its sessions hold, so
//! that the claims are released and the apps keep running for the next
//! session to attach to as they are.

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, PoisonError};

use cli::client::{Endpoint, Reply};
use cli::request;

/// The sessions the server's calls went out in.
pub struct Sessions {
    /// The session the server minted.
    pub own: String,
    /// Each session a call went out in, with the targets its replies named.
    held: Mutex<BTreeMap<String, BTreeSet<String>>>,
}

impl Sessions {
    /// The server's sessions, its own `mcp-<pid>` first.
    pub fn new(own: String) -> Sessions {
        Sessions {
            own,
            held: Mutex::new(BTreeMap::new()),
        }
    }

    /// Notes what the reply to a call of `action` in `session` says of the
    /// session's targets: a reply that names its target, as `open`'s and a
    /// snapshot's do, holds it, and `close`'s releases it.
    pub fn record(&self, session: &str, action: &str, reply: &Reply) {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        let targets = held.entry(session.to_owned()).or_default();
        let named = reply
            .data
            .get("target")
            .and_then(serde_json::Value::as_str)
            .filter(|_| reply.success);
        match named {
            Some(target) if action == "close" => {
                targets.remove(target);
            }
            Some(target) => {
                targets.insert(target.to_owned());
            }
            None => {}
        }
    }

    /// Releases every target the server's sessions hold, each with `close`
    /// and `keep`: those their replies named, then those `status` lists for
    /// them, since a command whose reply names no target, such as a click
    /// on a selector, claims one too. A daemon that no longer serves holds
    /// no claims, so none is started to release them.
    pub fn release(&self, endpoint: &Endpoint) {
        let held = self
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if held.is_empty() || UnixStream::connect(endpoint.socket()).is_err() {
            return;
        }
        let mut released = BTreeSet::new();
        for (session, targets) in &held {
            for target in targets {
                close(endpoint, session, target);
                released.insert((session.clone(), target.clone()));
            }
        }
        let status = request::bare(
            wire::Command::Status(wire::Status {}),
            &self.own,
            cli::client::request_id(),
        );
        let listed = match cli::client::call(endpoint, &request::line(&status)) {
            Ok(reply) if reply.success => serde_json::from_value::<wire::reply::Status>(reply.data)
                .map(|status| status.sessions)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        for session in listed
            .iter()
            .filter(|session| held.contains_key(&session.name))
        {
            for target in &session.targets {
                if !released.contains(&(session.name.clone(), target.clone())) {
                    close(endpoint, &session.name, target);
                }
            }
        }
    }
}

/// Releases `target` from `session` with the app left running, saying on
/// standard error when the daemon refuses.
fn close(endpoint: &Endpoint, session: &str, target: &str) {
    let mut close = request::bare(
        wire::Command::Close(wire::Close { keep: true }),
        session,
        cli::client::request_id(),
    );
    close.device = Some(target.to_owned());
    let failure = match cli::client::call(endpoint, &request::line(&close)) {
        Ok(reply) if reply.success => return,
        Ok(reply) => reply.error.unwrap_or_default(),
        Err(failure) => failure.message,
    };
    eprintln!(
        "{}: cannot release {target} from session {session}: {failure}",
        crate::server::NAME
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn reply(success: bool, data: Value) -> Reply {
        Reply {
            success,
            data,
            error: None,
            code: None,
            warning: None,
        }
    }

    fn held(sessions: &Sessions) -> BTreeMap<String, BTreeSet<String>> {
        sessions.held.lock().expect("unpoisoned").clone()
    }

    #[test]
    fn a_reply_that_names_its_target_holds_it_and_close_releases_it() {
        let sessions = Sessions::new("mcp-1".into());
        sessions.record("mcp-1", "devices", &reply(true, json!({"devices": []})));
        sessions.record(
            "mcp-1",
            "open",
            &reply(true, json!({"opened": "app", "target": "emulator-5554"})),
        );
        sessions.record(
            "resumed",
            "snapshot",
            &reply(true, json!({"target": "FAKE-IOS-1"})),
        );
        sessions.record(
            "mcp-1",
            "open",
            &reply(false, json!({"target": "emulator-5556"})),
        );
        let expected = BTreeMap::from([
            (
                "mcp-1".to_owned(),
                BTreeSet::from(["emulator-5554".to_owned()]),
            ),
            (
                "resumed".to_owned(),
                BTreeSet::from(["FAKE-IOS-1".to_owned()]),
            ),
        ]);
        assert_eq!(held(&sessions), expected, "a failure holds nothing");
        sessions.record(
            "mcp-1",
            "close",
            &reply(true, json!({"closed": true, "target": "emulator-5554"})),
        );
        assert_eq!(held(&sessions)["mcp-1"], BTreeSet::new());
    }
}
