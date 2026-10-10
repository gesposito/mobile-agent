//! A fake Appium server attached to a fake WebDriverAgent on an iPhone that
//! runs a fixture: the WebDriver endpoints the WebDriverAgent adapter uses,
//! answering over HTTP on the loopback as Appium does, and the fixture's
//! screens, enough for the three reference flows: the Login tab and its
//! sign-in, the 200-row list a swipe moves by a screen, a row's detail, and
//! the 500-cell Big tab.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

/// The W3C key of an element reference.
const ELEMENT: &str = "element-6066-11e4-a52e-4f735466cecf";

/// Rows a screen of the list shows, and a swipe moves it by.
const ROWS_ON_SCREEN: u32 = 11;

#[derive(Clone, Debug, PartialEq)]
enum Screen {
    /// No app in front, before a launch.
    Home,
    Login,
    /// The list, its first row on screen.
    List(u32),
    Detail(u32),
    Big,
}

/// The app's state and what the server was asked.
#[derive(Debug)]
struct State {
    /// The log the fake `appium` writes a line to as it starts and `ended`
    /// to as it exits: the server answers `/status` only while that
    /// process runs, as Appium serves only while it runs.
    process_log: PathBuf,
    screen: Screen,
    username: String,
    password: String,
    status: String,
    session: Option<String>,
    /// The capabilities of the last session asked for.
    capabilities: Value,
    /// Each request: its method and path, and for an execute its script.
    requests: Vec<String>,
}

/// The fake server: where it listens and what it was asked.
pub struct FakeAppium {
    pub port: u16,
    state: Arc<Mutex<State>>,
}

impl FakeAppium {
    /// A server on a free loopback port, serving until the test ends, for
    /// the fake `appium` that logs its starts and exits to `process_log`.
    pub fn start(process_log: &Path) -> FakeAppium {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(State {
            process_log: process_log.to_owned(),
            screen: Screen::Home,
            username: String::new(),
            password: String::new(),
            status: String::new(),
            session: None,
            capabilities: Value::Null,
            requests: Vec::new(),
        }));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &shared);
            }
        });
        FakeAppium { port, state }
    }

    /// Each request so far: `GET /status`, `POST /session/S1/element`,
    /// `POST execute mobile: swipe`, ...
    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    /// The capabilities of the last session asked for.
    pub fn capabilities(&self) -> Value {
        self.state.lock().unwrap().capabilities.clone()
    }

    /// Whether a session is open.
    pub fn session_open(&self) -> bool {
        self.state.lock().unwrap().session.is_some()
    }
}

fn serve(stream: TcpStream, state: &Mutex<State>) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let (method, path) = (
        parts.next().unwrap_or_default().to_owned(),
        parts.next().unwrap_or_default().to_owned(),
    );
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0_u8; length];
    let _ = reader.read_exact(&mut body);
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (code, reply) = answer(&mut state.lock().unwrap(), &method, &path, &body);
    let text = reply.to_string();
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 {code} {}\r\nContent-Type: application/json; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
        if code == 200 { "OK" } else { "Error" },
        text.len()
    );
}

/// A W3C error.
fn error(code: u16, error: &str, message: &str) -> (u16, Value) {
    (
        code,
        json!({"value": {"error": error, "message": message, "stacktrace": ""}}),
    )
}

fn answer(state: &mut State, method: &str, path: &str, body: &Value) -> (u16, Value) {
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    let logged = match (method, segments.as_slice()) {
        ("POST", ["session", _, "execute", "sync"]) => format!(
            "POST execute {}{}",
            body["script"].as_str().unwrap_or("?"),
            body["args"][0]["direction"]
                .as_str()
                .map(|d| format!(" {d}"))
                .unwrap_or_default()
        ),
        _ => format!("{method} {path}"),
    };
    state.requests.push(logged);
    match (method, segments.as_slice()) {
        ("GET", ["status"]) => {
            let log = std::fs::read_to_string(&state.process_log).unwrap_or_default();
            if log.lines().last().is_none_or(|line| line == "ended") {
                return error(503, "unknown error", "Appium is not running");
            }
            (
                200,
                json!({"value": {"ready": true, "message": "The server is ready to accept new connections", "build": {"version": "3.8.0"}}}),
            )
        }
        ("POST", ["session"]) => {
            state.capabilities = body.clone();
            state.session = Some("S1".to_owned());
            (
                200,
                json!({"value": {"sessionId": "S1", "capabilities": {"platformName": "iOS"}}}),
            )
        }
        ("DELETE", ["session", "S1"]) => {
            state.session = None;
            (200, json!({"value": null}))
        }
        (_, ["session", id, ..]) if state.session.as_deref() != Some(id) => error(
            404,
            "invalid session id",
            "A session is either terminated or not started",
        ),
        ("POST", ["session", _, "execute", "sync"]) => execute(state, body),
        ("POST", ["session", _, "element"]) => {
            let id = match body["using"].as_str() {
                Some("accessibility id") => body["value"].as_str().unwrap_or_default().to_owned(),
                _ => return error(400, "invalid argument", "unsupported locator"),
            };
            if elements(state).contains_key(&id) {
                (200, json!({"value": {ELEMENT: format!("el:{id}")}}))
            } else {
                error(
                    404,
                    "no such element",
                    "An element could not be located on the page using the given search parameters.",
                )
            }
        }
        (_, ["session", _, "element", element, action]) => {
            let id = element.strip_prefix("el:").unwrap_or(element).to_owned();
            let Some(text) = elements(state).get(&id).cloned() else {
                return error(404, "stale element reference", "The element is not present");
            };
            match (method, *action) {
                ("GET", "displayed") => (200, json!({"value": true})),
                ("GET", "text") => (200, json!({"value": text})),
                ("POST", "click") => {
                    click(state, &id);
                    (200, json!({"value": null}))
                }
                ("POST", "value") => {
                    let typed = body["text"].as_str().unwrap_or_default().to_owned();
                    match id.as_str() {
                        "login-username" => state.username.push_str(&typed),
                        "login-password" => state.password.push_str(&typed),
                        _ => return error(400, "invalid element state", "not a text field"),
                    }
                    (200, json!({"value": null}))
                }
                _ => error(404, "unknown command", path),
            }
        }
        _ => error(404, "unknown command", path),
    }
}

fn execute(state: &mut State, body: &Value) -> (u16, Value) {
    let args = &body["args"][0];
    match body["script"].as_str().unwrap_or_default() {
        "mobile: terminateApp" => {
            state.screen = Screen::Home;
            (200, json!({"value": true}))
        }
        "mobile: launchApp" => {
            state.screen = Screen::Login;
            state.username.clear();
            state.password.clear();
            state.status.clear();
            (200, json!({"value": null}))
        }
        "mobile: swipe" => {
            if let Screen::List(first) = state.screen {
                state.screen = Screen::List(match args["direction"].as_str() {
                    Some("up") => (first + ROWS_ON_SCREEN).min(200 - ROWS_ON_SCREEN + 1),
                    _ => first.saturating_sub(ROWS_ON_SCREEN).max(1),
                });
            }
            (200, json!({"value": null}))
        }
        "mobile: source" if args["format"] == "json" => (200, json!({"value": source(state)})),
        script => error(404, "unknown method", script),
    }
}

fn click(state: &mut State, id: &str) {
    match id {
        "login-submit" if state.username == "agent" && state.password == "hunter2" => {
            state.status = "Welcome, agent".to_owned();
        }
        "login-submit" => state.status = "Invalid credentials".to_owned(),
        "tab-login" => state.screen = Screen::Login,
        "tab-list" => state.screen = Screen::List(1),
        "tab-big" => state.screen = Screen::Big,
        "detail-back" => state.screen = Screen::List(1),
        row if row.starts_with("row-") => {
            if let Ok(n) = row[4..].parse() {
                state.screen = Screen::Detail(n);
            }
        }
        _ => {}
    }
}

/// The identified elements on screen and the text each shows.
fn elements(state: &State) -> BTreeMap<String, String> {
    let mut shown = BTreeMap::new();
    let mut add = |id: String, text: String| {
        shown.insert(id, text);
    };
    match state.screen {
        Screen::Home => return shown,
        Screen::Login => {
            add("login-username".into(), state.username.clone());
            add("login-password".into(), "•".repeat(state.password.len()));
            add("login-submit".into(), "Sign in".into());
            add("login-status".into(), state.status.clone());
        }
        Screen::List(first) => {
            for n in first..first + ROWS_ON_SCREEN {
                add(format!("row-{n}"), format!("Row {n}"));
            }
        }
        Screen::Detail(n) => {
            add(format!("detail-{n}"), format!("Detail {n}"));
            add("detail-back".into(), "Back".into());
        }
        Screen::Big => {
            add("big-count".into(), "500 elements".into());
            for n in 1..=500 {
                add(format!("cell-{n}"), format!("Cell {n}"));
            }
        }
    }
    if !matches!(state.screen, Screen::Detail(_)) {
        for (id, label) in [
            ("tab-login", "Login"),
            ("tab-list", "List"),
            ("tab-big", "Big"),
            ("tab-anim", "Anim"),
            ("tab-system", "System"),
        ] {
            add(id.into(), label.into());
        }
    }
    shown
}

/// `mobile: source` in its JSON form: the application, its window, and a
/// node per element, with WebDriverAgent's keys.
fn source(state: &State) -> Value {
    let nodes: Vec<Value> = elements(state)
        .into_iter()
        .map(|(id, text)| {
            json!({"type": "StaticText", "label": text, "value": text, "rawIdentifier": id,
                   "isVisible": "1", "isEnabled": "1", "frame": "{{0, 0}, {10, 10}}"})
        })
        .collect();
    json!({
        "type": "Application", "label": "FixtureSwiftUI", "value": null, "rawIdentifier": null,
        "isVisible": "1",
        "children": [{"type": "Window", "label": null, "value": null, "rawIdentifier": null,
                      "isVisible": "1", "children": nodes}]
    })
}
