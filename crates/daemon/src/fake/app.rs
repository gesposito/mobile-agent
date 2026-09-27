//! The fake device's apps as state that input changes: a launcher with a
//! heading and two buttons, the fixture app of `fixtures/CONTRACT.md` (tabs
//! Login, List, Big, Anim and System) and a settings app. Each state
//! renders to the windows a driver would read.

use std::collections::HashMap;

use mirror::backend::Owner;
use protocol::message::TextMethod;
use protocol::snapshot::WindowFlags;

use super::tree::{El, Kit, Win};
use crate::driver::{DriverError, Launch, Platform};
use crate::wire::code;

/// The fixture app the launcher's Fixture button opens.
pub(crate) const FIXTURE: &str = "dev.mobileagent.fixture.expo";

/// Row height of the fixture's list, in points.
const ROW: f64 = 48.0;
const ROWS: u32 = 200;
const CELLS: u32 = 500;
const COLUMNS: u32 = 20;

/// Android key codes (`KeyEvent.KEYCODE_*`).
mod android {
    pub const HOME: u32 = 3;
    pub const BACK: u32 = 4;
    pub const DIGIT_0: u32 = 7;
    pub const A: u32 = 29;
    pub const TAB: u32 = 61;
    pub const SPACE: u32 = 62;
    pub const ENTER: u32 = 66;
    pub const DEL: u32 = 67;
    pub const ESCAPE: u32 = 111;
    pub const META_SHIFT_ON: u32 = 1;
}

/// iOS keys as HID usages, `(page << 16) | usage`: the keyboard page 0x07
/// and the consumer page 0x0C for the Home button.
mod ios {
    pub const KEYBOARD: u32 = 0x07 << 16;
    pub const A: u32 = KEYBOARD | 0x04;
    pub const DIGIT_1: u32 = KEYBOARD | 0x1E;
    pub const DIGIT_0: u32 = KEYBOARD | 0x27;
    pub const ENTER: u32 = KEYBOARD | 0x28;
    pub const ESCAPE: u32 = KEYBOARD | 0x29;
    pub const BACKSPACE: u32 = KEYBOARD | 0x2A;
    pub const TAB: u32 = KEYBOARD | 0x2B;
    pub const SPACE: u32 = KEYBOARD | 0x2C;
    pub const HOME: u32 = (0x0C << 16) | 0x40;
    pub const LEFT_SHIFT: u32 = 0x02;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Login,
    List,
    Big,
    Anim,
    System,
}

const TABS: [(Tab, &str, &str); 5] = [
    (Tab::Login, "tab-login", "Login"),
    (Tab::List, "tab-list", "List"),
    (Tab::Big, "tab-big", "Big"),
    (Tab::Anim, "tab-anim", "Anim"),
    (Tab::System, "tab-system", "System"),
];

impl Tab {
    fn parse(name: &str) -> Tab {
        match name {
            "list" => Tab::List,
            "big" => Tab::Big,
            "anim" => Tab::Anim,
            "system" => Tab::System,
            _ => Tab::Login,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Username,
    Password,
}

impl Field {
    fn parse(id: &str) -> Option<Field> {
        match id {
            "login-username" => Some(Field::Username),
            "login-password" => Some(Field::Password),
            _ => None,
        }
    }
}

/// One running fixture app.
#[derive(Clone, Debug, PartialEq)]
struct Fixture {
    tab: Tab,
    username: String,
    password: String,
    focus: Option<Field>,
    status: String,
    /// How far the list is scrolled, in points.
    offset: f64,
    detail: Option<u32>,
    animate: bool,
    alert: bool,
    prompt: bool,
    permission: &'static str,
}

impl Fixture {
    fn launch(tab: Tab) -> Fixture {
        Fixture {
            tab,
            username: String::new(),
            password: String::new(),
            focus: None,
            status: "Signed out".to_owned(),
            offset: 0.0,
            detail: None,
            animate: true,
            alert: false,
            prompt: false,
            permission: "unknown",
        }
    }

    fn field(&mut self, field: Field) -> &mut String {
        match field {
            Field::Username => &mut self.username,
            Field::Password => &mut self.password,
        }
    }

    fn submit(&mut self) {
        self.status = if self.password == "hunter2" {
            format!("Welcome, {}", self.username)
        } else {
            "Invalid credentials".to_owned()
        };
        self.focus = None;
    }

    fn modal(&self) -> bool {
        self.alert || self.prompt
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Foreground {
    Launcher,
    Fixture(String),
    Settings,
}

/// The fake device: which app is in front and every running app's state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Device {
    pub kit: Kit,
    foreground: Foreground,
    fixtures: HashMap<String, Fixture>,
    wifi: bool,
}

impl Device {
    pub fn new(platform: Platform) -> Device {
        Device {
            kit: Kit { platform },
            foreground: Foreground::Launcher,
            fixtures: HashMap::new(),
            wifi: true,
        }
    }

    fn settings_id(&self) -> &'static str {
        match self.kit.platform {
            Platform::Android => "com.android.settings",
            Platform::Ios => "com.apple.Preferences",
        }
    }

    /// The apps the device has installed.
    pub fn installed(&self) -> Vec<&'static str> {
        let mut apps = vec![FIXTURE];
        apps.push(match self.kit.platform {
            Platform::Android => "dev.mobileagent.fixture.compose",
            Platform::Ios => "dev.mobileagent.fixture.swiftui",
        });
        apps.push(self.settings_id());
        apps
    }

    fn not_installed(&self, app: &str) -> DriverError {
        DriverError::new(
            code::APP_NOT_FOUND,
            format!(
                "{app} is not installed on this target; installed apps: {}",
                self.installed().join(", ")
            ),
        )
    }

    pub fn launch(&mut self, app: &str, launch: &Launch) -> Result<(), DriverError> {
        if app == self.settings_id() {
            self.foreground = Foreground::Settings;
            return Ok(());
        }
        if !self.installed().contains(&app) {
            return Err(self.not_installed(app));
        }
        if launch.relaunch || !self.fixtures.contains_key(app) {
            let tab = launch
                .env
                .iter()
                .find(|(key, _)| key == "FIXTURE_INITIAL_TAB")
                .map_or(Tab::Login, |(_, value)| Tab::parse(value));
            self.fixtures.insert(app.to_owned(), Fixture::launch(tab));
        }
        self.foreground = Foreground::Fixture(app.to_owned());
        Ok(())
    }

    pub fn terminate(&mut self, app: &str) -> Result<(), DriverError> {
        if !self.installed().contains(&app) {
            return Err(self.not_installed(app));
        }
        self.fixtures.remove(app);
        let front = match &self.foreground {
            Foreground::Fixture(front) => front == app,
            Foreground::Settings => app == self.settings_id(),
            Foreground::Launcher => false,
        };
        if front {
            self.foreground = Foreground::Launcher;
        }
        Ok(())
    }

    fn front_fixture(&mut self) -> Option<&mut Fixture> {
        match &self.foreground {
            Foreground::Fixture(app) => self.fixtures.get_mut(app),
            _ => None,
        }
    }

    /// Whether the Anim tab's spinner turns, so frames keep changing.
    pub fn animating(&self) -> bool {
        match &self.foreground {
            Foreground::Fixture(app) => self
                .fixtures
                .get(app)
                .is_some_and(|f| f.tab == Tab::Anim && f.animate && !f.modal()),
            _ => false,
        }
    }

    /// A tap that landed on `hit`; whether the screen changed.
    pub fn tap(&mut self, hit: Option<&str>) -> bool {
        let before = self.clone();
        match self.foreground.clone() {
            Foreground::Launcher => match hit {
                Some("launcher-fixture") => {
                    let _ = self.launch(FIXTURE, &Launch::default());
                }
                Some("launcher-settings") => self.foreground = Foreground::Settings,
                _ => {}
            },
            Foreground::Settings => {
                if hit == Some("settings-wifi") {
                    self.wifi = !self.wifi;
                }
            }
            Foreground::Fixture(_) => {
                if let Some(fixture) = self.front_fixture() {
                    tap_fixture(fixture, hit);
                }
            }
        }
        *self != before
    }

    /// Text for the field `target` names, or the focused one: whether the
    /// screen changed, or `None` when no editable field takes the text.
    pub fn text(&mut self, target: Option<&str>, text: &str, method: TextMethod) -> Option<bool> {
        let before = self.clone();
        let fixture = self.front_fixture()?;
        if fixture.tab != Tab::Login || fixture.modal() {
            return None;
        }
        let field = match target {
            Some(id) => Field::parse(id)?,
            None => fixture.focus?,
        };
        fixture.focus = Some(field);
        let value = fixture.field(field);
        if method != TextMethod::KEYS {
            value.clear();
        }
        value.push_str(text);
        Some(*self != before)
    }

    /// A key press; whether the screen changed.
    pub fn key(&mut self, code: u32, meta: Option<u32>) -> bool {
        let before = self.clone();
        let platform = self.kit.platform;
        let key = Key::decode(platform, code, meta.unwrap_or(0));
        match key {
            Key::Home => self.foreground = Foreground::Launcher,
            Key::Back => self.back(),
            key => {
                if let Some(fixture) = self.front_fixture() {
                    key_fixture(fixture, key);
                }
            }
        }
        *self != before
    }

    fn back(&mut self) {
        match self.foreground.clone() {
            Foreground::Launcher => {}
            Foreground::Settings => self.foreground = Foreground::Launcher,
            Foreground::Fixture(_) => {
                let Some(fixture) = self.front_fixture() else {
                    return;
                };
                if fixture.prompt {
                    fixture.prompt = false;
                    fixture.permission = "denied";
                } else if fixture.alert {
                    fixture.alert = false;
                } else if fixture.focus.is_some() {
                    fixture.focus = None;
                } else if fixture.tab == Tab::List && fixture.detail.is_some() {
                    fixture.detail = None;
                } else {
                    self.foreground = Foreground::Launcher;
                }
            }
        }
    }

    /// A swipe from `from` to `to` in points; whether the screen changed.
    pub fn swipe(&mut self, from: (f64, f64), to: (f64, f64)) -> bool {
        let before = self.clone();
        let (top, bottom) = self.kit.content();
        if let Some(fixture) = self.front_fixture()
            && fixture.tab == Tab::List
            && fixture.detail.is_none()
            && !fixture.modal()
        {
            let max = f64::from(ROWS) * ROW - (bottom - top);
            fixture.offset = (fixture.offset + from.1 - to.1).clamp(0.0, max);
        }
        *self != before
    }

    /// The windows the device shows, top-most first.
    pub fn windows(&self) -> Vec<Win> {
        let kit = self.kit;
        let mut windows = Vec::new();
        match &self.foreground {
            Foreground::Launcher => {
                let package = match kit.platform {
                    Platform::Android => "com.android.launcher3",
                    Platform::Ios => "com.apple.springboard",
                };
                windows.push(kit.app_window(package, "Home", launcher(kit)));
            }
            Foreground::Settings => {
                let content = settings(kit, self.wifi);
                windows.push(kit.app_window(self.settings_id(), "Settings", content));
            }
            Foreground::Fixture(app) => {
                fixture_windows(kit, app, &self.fixtures[app], &mut windows);
            }
        }
        windows.extend(kit.status_bar());
        windows.sort_by_key(|w| std::cmp::Reverse(w.layer));
        windows
    }
}

fn launcher(kit: Kit) -> Vec<El> {
    vec![
        kit.heading("launcher-title", "Home", kit.line(16.0, 32.0)),
        kit.button("launcher-fixture", "Fixture", kit.line(72.0, 48.0)),
        kit.button("launcher-settings", "Settings", kit.line(136.0, 48.0)),
    ]
}

fn settings(kit: Kit, wifi: bool) -> Vec<El> {
    vec![
        kit.heading("settings-title", "Settings", kit.line(16.0, 32.0)),
        kit.switch("settings-wifi", "Wi-Fi", wifi, kit.line(72.0, 48.0)),
    ]
}

fn tap_fixture(fixture: &mut Fixture, hit: Option<&str>) {
    if fixture.prompt {
        match hit {
            Some("permission-allow") => {
                fixture.prompt = false;
                fixture.permission = "granted";
            }
            Some("permission-deny") => {
                fixture.prompt = false;
                fixture.permission = "denied";
            }
            _ => {}
        }
        return;
    }
    if fixture.alert {
        if hit == Some("inapp-alert-ok") {
            fixture.alert = false;
        }
        return;
    }
    let Some(hit) = hit else {
        fixture.focus = None;
        return;
    };
    if let Some((tab, _, _)) = TABS.iter().find(|(_, id, _)| *id == hit) {
        fixture.tab = *tab;
        fixture.focus = None;
        return;
    }
    if let Some(field) = Field::parse(hit) {
        fixture.focus = Some(field);
        return;
    }
    fixture.focus = None;
    match hit {
        "login-submit" => fixture.submit(),
        "detail-back" => fixture.detail = None,
        "anim-toggle" => fixture.animate = !fixture.animate,
        "system-alert" => fixture.alert = true,
        "system-permission" if fixture.permission == "unknown" => fixture.prompt = true,
        row => {
            if let Some(n) = row.strip_prefix("row-").and_then(|n| n.parse().ok()) {
                fixture.detail = Some(n);
            }
        }
    }
}

/// A key, decoded from either platform's codes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Key {
    Home,
    Back,
    Enter,
    Tab,
    Escape,
    Delete,
    Char(char),
    Other,
}

impl Key {
    fn decode(platform: Platform, code: u32, meta: u32) -> Key {
        match platform {
            Platform::Android => {
                let shift = meta & android::META_SHIFT_ON != 0;
                match code {
                    android::HOME => Key::Home,
                    android::BACK => Key::Back,
                    android::ENTER => Key::Enter,
                    android::TAB => Key::Tab,
                    android::ESCAPE => Key::Escape,
                    android::DEL => Key::Delete,
                    android::SPACE => Key::Char(' '),
                    c if (android::DIGIT_0..android::DIGIT_0 + 10).contains(&c) => {
                        Key::Char(char::from(b'0' + (c - android::DIGIT_0) as u8))
                    }
                    c if (android::A..android::A + 26).contains(&c) => {
                        letter((c - android::A) as u8, shift)
                    }
                    _ => Key::Other,
                }
            }
            Platform::Ios => {
                let shift = meta & ios::LEFT_SHIFT != 0;
                match code {
                    ios::HOME => Key::Home,
                    ios::ENTER => Key::Enter,
                    ios::TAB => Key::Tab,
                    ios::ESCAPE => Key::Escape,
                    ios::BACKSPACE => Key::Delete,
                    ios::SPACE => Key::Char(' '),
                    ios::DIGIT_0 => Key::Char('0'),
                    c if (ios::DIGIT_1..ios::DIGIT_1 + 9).contains(&c) => {
                        Key::Char(char::from(b'1' + (c - ios::DIGIT_1) as u8))
                    }
                    c if (ios::A..ios::A + 26).contains(&c) => letter((c - ios::A) as u8, shift),
                    _ => Key::Other,
                }
            }
        }
    }
}

fn letter(index: u8, shift: bool) -> Key {
    let c = char::from(b'a' + index);
    Key::Char(if shift { c.to_ascii_uppercase() } else { c })
}

fn key_fixture(fixture: &mut Fixture, key: Key) {
    match key {
        Key::Escape => {
            if fixture.alert {
                fixture.alert = false;
            } else {
                fixture.focus = None;
            }
        }
        Key::Enter => match fixture.focus {
            Some(Field::Username) => fixture.focus = Some(Field::Password),
            Some(Field::Password) => fixture.submit(),
            None => {}
        },
        Key::Tab => {
            if fixture.focus == Some(Field::Username) {
                fixture.focus = Some(Field::Password);
            }
        }
        Key::Delete => {
            if let Some(field) = fixture.focus {
                fixture.field(field).pop();
            }
        }
        Key::Char(c) => {
            if let Some(field) = fixture.focus {
                fixture.field(field).push(c);
            }
        }
        Key::Home | Key::Back | Key::Other => {}
    }
}

fn fixture_windows(kit: Kit, app: &str, fixture: &Fixture, windows: &mut Vec<Win>) {
    let title = match app {
        "dev.mobileagent.fixture.compose" => "FixtureCompose",
        "dev.mobileagent.fixture.swiftui" => "FixtureSwiftUI",
        _ => "FixtureExpo",
    };
    let (top, bottom) = kit.content();
    let area = [0.0, top, kit.width(), bottom];
    let mut content = vec![kit.group(area).children(tab_content(kit, fixture))];
    let [left, bar_top, right, bar_bottom] = kit.tab_bar();
    let width = (right - left) / TABS.len() as f64;
    let tabs = TABS.iter().enumerate().map(|(i, (tab, id, label))| {
        let x = left + width * i as f64;
        kit.tab(
            id,
            label,
            *tab == fixture.tab,
            [x, bar_top, x + width, bar_bottom],
        )
    });
    content.push(kit.group(kit.tab_bar()).children(tabs));
    let dialog = |message: &str, buttons: Vec<El>| -> Vec<El> {
        let mut children = vec![kit.label(None, message, [56.0, 420.0, kit.width() - 56.0, 460.0])];
        children.extend(buttons);
        children
    };
    match kit.platform {
        Platform::Ios if fixture.prompt => {
            // SpringBoard owns the foreground while its prompt shows, and the
            // reader reads its tree instead of the app's.
            let screen = [0.0, 0.0, kit.width(), kit.height()];
            let question = "Allow “FixtureSwiftUI” to use your location?";
            let prompt = kit.alert(
                question,
                [61.0, 300.0, 341.0, 620.0],
                vec![
                    kit.label(None, question, [77.0, 320.0, 325.0, 364.0]),
                    kit.button(
                        "permission-allow",
                        "Allow While Using App",
                        [61.0, 503.0, 341.0, 561.0],
                    ),
                    kit.button(
                        "permission-deny",
                        "Don’t Allow",
                        [61.0, 562.0, 341.0, 620.0],
                    ),
                ],
            );
            windows.push(Win {
                id: 900,
                kind: protocol::snapshot::WindowType::SYSTEM,
                layer: 10,
                package: Some("com.apple.springboard".to_owned()),
                title: Some("SpringBoard".to_owned()),
                rect: screen,
                flags: WindowFlags::ACTIVE | WindowFlags::FOCUSED,
                events: Some(false),
                owner: Owner::System("com.apple.springboard".to_owned()),
                modal: false,
                root: El::new("XCUIElementTypeApplication", screen)
                    .desc("SpringBoard")
                    .child(prompt),
            });
            return;
        }
        Platform::Ios if fixture.alert => {
            content.push(kit.alert(
                "In-app alert",
                [61.0, 360.0, 341.0, 514.0],
                vec![
                    kit.label(None, "In-app alert", [77.0, 380.0, 325.0, 402.0]),
                    kit.label(
                        None,
                        "This alert is presented by the app.",
                        [77.0, 406.0, 325.0, 446.0],
                    ),
                    kit.button("inapp-alert-ok", "OK", [61.0, 470.0, 341.0, 514.0]),
                ],
            ));
        }
        Platform::Android if fixture.prompt => {
            windows.push(kit.dialog(
                4,
                "com.google.android.permissioncontroller",
                Owner::System("com.google.android.permissioncontroller".to_owned()),
                "Permission",
                dialog(
                    "Allow FixtureExpo to access this device’s location?",
                    vec![
                        kit.button(
                            "permission-allow",
                            "While using the app",
                            [56.0, 480.0, kit.width() - 56.0, 520.0],
                        ),
                        kit.button(
                            "permission-deny",
                            "Don’t allow",
                            [56.0, 530.0, kit.width() - 56.0, 570.0],
                        ),
                    ],
                ),
            ));
        }
        Platform::Android if fixture.alert => {
            windows.push(kit.dialog(
                3,
                app,
                Owner::Target,
                "In-app alert",
                dialog(
                    "This alert is presented by the app.",
                    vec![kit.button(
                        "inapp-alert-ok",
                        "OK",
                        [56.0, 530.0, kit.width() - 56.0, 570.0],
                    )],
                ),
            ));
        }
        _ => {}
    }
    let mut window = kit.app_window(app, title, content);
    if windows.iter().any(|w| w.modal) {
        window.flags = WindowFlags::EMPTY;
    }
    if fixture.focus.is_some()
        && let Some(keyboard) = kit.keyboard()
    {
        windows.insert(0, keyboard);
    }
    windows.push(window);
}

fn tab_content(kit: Kit, fixture: &Fixture) -> Vec<El> {
    let (top, bottom) = kit.content();
    let width = kit.width();
    match fixture.tab {
        Tab::Login => vec![
            kit.heading("login-title", "Login", kit.line(16.0, 32.0)),
            kit.field(
                "login-username",
                "Username",
                &fixture.username,
                false,
                fixture.focus == Some(Field::Username),
                kit.line(72.0, 48.0),
            ),
            kit.field(
                "login-password",
                "Password",
                &fixture.password,
                true,
                fixture.focus == Some(Field::Password),
                kit.line(136.0, 48.0),
            ),
            kit.pressable("login-submit", "Sign in", kit.line(208.0, 48.0)),
            kit.label(Some("login-status"), &fixture.status, kit.line(280.0, 24.0)),
        ],
        Tab::List => match fixture.detail {
            Some(n) => vec![
                kit.label(
                    Some(&format!("detail-{n}")),
                    &format!("Detail {n}"),
                    kit.line(16.0, 32.0),
                ),
                kit.button("detail-back", "Back", kit.line(72.0, 48.0)),
            ],
            None => {
                let rows = (1..=ROWS).map(|n| {
                    let row_top = top + f64::from(n - 1) * ROW - fixture.offset;
                    let rect = [0.0, row_top, width, row_top + ROW];
                    let visible = rect[3] > top && rect[1] < bottom;
                    let el = kit.pressable(&format!("row-{n}"), &format!("Row {n}"), rect);
                    hide_unless(el, visible)
                });
                vec![kit.list("list", [0.0, top, width, bottom]).children(rows)]
            }
        },
        Tab::Big => {
            let grid_top = top + 40.0;
            let cell_width = width / f64::from(COLUMNS);
            let cell_height = (bottom - grid_top) / f64::from(CELLS / COLUMNS);
            let cells = (1..=CELLS).map(|n| {
                let (col, row) = (f64::from((n - 1) % COLUMNS), f64::from((n - 1) / COLUMNS));
                let (x, y) = (col * cell_width, grid_top + row * cell_height);
                kit.cell(
                    &format!("cell-{n}"),
                    &format!("Cell {n}"),
                    [x, y, x + cell_width, y + cell_height],
                )
            });
            vec![
                kit.label(Some("big-count"), "500 elements", kit.line(8.0, 24.0)),
                kit.grid("big-grid", [0.0, grid_top, width, bottom])
                    .children(cells),
            ]
        }
        Tab::Anim => vec![
            kit.switch(
                "anim-toggle",
                "Animate",
                fixture.animate,
                kit.line(16.0, 48.0),
            ),
            kit.spinner(
                "anim-spinner",
                [
                    width / 2.0 - 24.0,
                    top + 96.0,
                    width / 2.0 + 24.0,
                    top + 144.0,
                ],
            ),
        ],
        Tab::System => vec![
            kit.button(
                "system-permission",
                "Request location permission",
                kit.line(16.0, 48.0),
            ),
            kit.label(
                Some("system-permission-status"),
                &format!("Permission: {}", fixture.permission),
                kit.line(80.0, 24.0),
            ),
            kit.button("system-alert", "Show in-app alert", kit.line(128.0, 48.0)),
        ],
    }
}

/// `el` and its subtree marked invisible unless `visible`.
fn hide_unless(el: El, visible: bool) -> El {
    if visible {
        return el;
    }
    let mut el = el.set(protocol::snapshot::NodeFlags::VISIBLE, false);
    el.children = el
        .children
        .into_iter()
        .map(|child| hide_unless(child, false))
        .collect();
    el
}
