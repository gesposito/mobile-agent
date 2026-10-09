//! The fake device's apps as state that input changes: a launcher with a
//! heading and two buttons, the fixture app of `fixtures/CONTRACT.md` (tabs
//! Login, List, Big, Anim and System), a settings app and the placeholder
//! app. Each state renders to the windows a driver would read.

use std::collections::HashMap;
use std::time::Duration;

use mirror::backend::Owner;
use protocol::snapshot::{
    Scroll, ScrollAxis, ScrollFlags, ScrollPosition, ScrollRange, ScrollUnit, WindowFlags,
};

use super::tree::{self, El, Kit, Win};
use super::{Slide, Typing};
use crate::driver::{DriverError, Launch, Platform};
use crate::wire::code;

/// The word the keyboard's glide typing takes a stroke across it for, as
/// Gboard took each stroke of a `scroll` that started on its keys on a Nexus
/// 5X (`reports/10.3-physical-android.md`, finding 4).
const GLIDE_WORD: &str = "by";

/// How text reaches a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// Key by key: in place of what the field holds when `replace` is set,
    /// as `fill` types, else after it, as `type` does.
    Keys { replace: bool },
    /// In one piece, in place of what the field holds: set directly or
    /// pasted.
    Whole,
}

/// The first key, counting from 0, that a field which takes keys out of
/// order ([`Typing::Reorder`]) lands before its last character.
const REORDER_FROM: usize = 3;

/// Enters `text` into a field that holds `value`, as `entry` says, the
/// field taking it as `typing` says.
fn take(value: &mut String, text: &str, entry: Entry, typing: Typing) {
    match entry {
        Entry::Whole => {
            value.clear();
            value.push_str(text);
        }
        Entry::Keys { replace } => {
            if replace {
                value.clear();
            }
            for (index, key) in text.chars().enumerate() {
                let behind = typing == Typing::Reorder && index >= REORDER_FROM;
                match value.char_indices().last() {
                    Some((last, _)) if behind => value.insert(last, key),
                    _ => value.push(key),
                }
            }
        }
    }
    if let Typing::Max(max) = typing
        && let Some((cut, _)) = value.char_indices().nth(max)
    {
        value.truncate(cut);
    }
}

/// The fixture app the launcher's Fixture button opens.
pub(crate) const FIXTURE: &str = "dev.mobileagent.fixture.expo";

/// The placeholder app: one screen with the shapes a React Native app
/// showed in the smoke test of 2026-10-09, under placeholder names (see
/// [`placeholder_content`]).
pub(crate) const PLACEHOLDER: &str = "dev.mobileagent.placeholder";

/// How many boxes the placeholder app's code field shows, one character
/// each.
const CODE_BOXES: usize = 6;

/// What a tap on the keyboard enters into the placeholder app's field in
/// focus: a key of its number pad.
const PAD_KEY: char = '5';

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

/// A field of the placeholder app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormField {
    /// The code field: a labelled node without traits holding a box per
    /// character, as a React Native code input built of boxes reads.
    Code,
    /// A field the tree shows only as a labelled node without traits, with
    /// nothing below it and no value, as iOS reads a React Native field
    /// inside a view marked accessible: it takes text and never shows it.
    Secret,
}

impl FormField {
    fn parse(id: &str) -> Option<FormField> {
        match id {
            "placeholder-code" => Some(FormField::Code),
            "placeholder-secret" => Some(FormField::Secret),
            _ => None,
        }
    }
}

/// The running placeholder app.
#[derive(Clone, Debug, PartialEq)]
struct Forms {
    code: String,
    secret: String,
    focus: Option<FormField>,
    status: String,
    /// Whether the layout has moved the submit button above the keyboard,
    /// which it does some time after the keyboard came up.
    lifted: bool,
}

impl Forms {
    fn launch() -> Forms {
        Forms {
            code: String::new(),
            secret: String::new(),
            focus: None,
            status: "Not submitted".to_owned(),
            lifted: false,
        }
    }

    fn field(&mut self, field: FormField) -> &mut String {
        match field {
            FormField::Code => &mut self.code,
            FormField::Secret => &mut self.secret,
        }
    }

    /// A tap that landed on `hit`, or on the keyboard when `on_keyboard`
    /// says so: a key of the number pad, entered into the field in focus.
    fn tap(&mut self, hit: Option<&str>, on_keyboard: bool) {
        if on_keyboard {
            if let Some(field) = self.focus {
                self.field(field).push(PAD_KEY);
            }
            return;
        }
        if let Some(field) = hit.and_then(FormField::parse) {
            self.focus = Some(field);
            return;
        }
        match hit {
            Some("placeholder-submit") => {
                self.status = if self.code.is_empty() {
                    "Submitted".to_owned()
                } else {
                    format!("Submitted {}", self.code)
                };
                self.focus = None;
            }
            Some("placeholder-help") => self.status = "Help shown".to_owned(),
            Some("placeholder-hint") => self.status = "Hint shown".to_owned(),
            _ => self.focus = None,
        }
        if self.focus.is_none() {
            self.lifted = false;
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Foreground {
    Launcher,
    Fixture(String),
    Settings,
    Placeholder,
}

/// Which screen is in front: the app, and in the fixture its tab, open
/// detail, alert and prompt. Moving from one screen to another animates;
/// changes within a screen do not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Screen {
    Launcher,
    Settings,
    Placeholder,
    Fixture {
        app: String,
        tab: Tab,
        detail: Option<u32>,
        alert: bool,
        prompt: bool,
    },
}

/// Where the keyboard is in coming up for the field in focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Keys {
    /// Down: no field has the focus, or its keyboard never comes.
    Down,
    /// Coming up, as [`Slide::After`] has it.
    Coming,
    /// Up.
    Up,
}

/// The fake device: which app is in front and every running app's state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Device {
    pub kit: Kit,
    foreground: Foreground,
    fixtures: HashMap<String, Fixture>,
    /// The placeholder app, while it runs.
    placeholder: Option<Forms>,
    wifi: bool,
    /// Whether the keyboard stays up over every screen of the fixture apps,
    /// whether or not a field has focus.
    keyboard_up: bool,
    /// How the keyboard comes up once a field takes the focus.
    slide: Slide,
    /// Where it is in coming up.
    keys: Keys,
}

impl Device {
    pub fn new(platform: Platform) -> Device {
        Device {
            kit: Kit { platform },
            foreground: Foreground::Launcher,
            fixtures: HashMap::new(),
            placeholder: None,
            wifi: true,
            keyboard_up: false,
            slide: Slide::AtOnce,
            keys: Keys::Down,
        }
    }

    /// This device, its keyboard up over every screen of the fixture apps
    /// when `up` is set.
    pub fn with_keyboard(mut self, up: bool) -> Device {
        self.keyboard_up = up;
        self
    }

    /// This device, its keyboard coming up as `slide` says once a field
    /// takes the focus.
    pub fn with_slide(mut self, slide: Slide) -> Device {
        self.slide = slide;
        self
    }

    /// Where the keyboard is, in points, while it shows: over an app with a
    /// field in focus, once it came up, or over any fixture screen when it
    /// stays up.
    fn keyboard(&self) -> Option<[f64; 4]> {
        let app = match &self.foreground {
            Foreground::Fixture(app) if self.keyboard_up || self.keys == Keys::Up => app.as_str(),
            Foreground::Placeholder if self.keys == Keys::Up => PLACEHOLDER,
            _ => return None,
        };
        Some(self.kit.keyboard(app).rect)
    }

    /// Whether a field of the app in front has the focus.
    fn focused(&self) -> bool {
        match &self.foreground {
            Foreground::Fixture(app) => self
                .fixtures
                .get(app)
                .is_some_and(|fixture| fixture.focus.is_some()),
            Foreground::Placeholder => self
                .placeholder
                .as_ref()
                .is_some_and(|forms| forms.focus.is_some()),
            Foreground::Launcher | Foreground::Settings => false,
        }
    }

    /// Brings the keyboard up or down after a change, before which a field
    /// had the focus when `was` says so: a field taking the focus brings it
    /// up as the slide says, and no field in focus sends it down.
    fn update_keyboard(&mut self, was: bool) {
        if !self.focused() {
            self.keys = Keys::Down;
            if let Some(forms) = self.placeholder.as_mut() {
                forms.lifted = false;
            }
        } else if !was || self.keys == Keys::Down {
            self.keys = match self.slide {
                Slide::AtOnce => Keys::Up,
                Slide::After(_) => Keys::Coming,
                Slide::Never => Keys::Down,
            };
        }
    }

    /// Whether the keyboard is coming up, and how long after the focus it
    /// comes.
    pub fn keyboard_coming(&self) -> Option<Duration> {
        match (self.keys, self.slide) {
            (Keys::Coming, Slide::After(after)) => Some(after),
            _ => None,
        }
    }

    /// Brings up the keyboard that was coming; whether it came.
    pub fn raise_keyboard(&mut self) -> bool {
        if self.keys != Keys::Coming || !self.focused() {
            return false;
        }
        self.keys = Keys::Up;
        true
    }

    /// Whether text for the field in focus is lost: on iOS, while its
    /// keyboard comes up.
    fn loses_text(&self) -> bool {
        self.kit.platform == Platform::Ios && self.keys == Keys::Coming
    }

    /// Whether the placeholder app's layout has yet to move its submit
    /// button above the keyboard that came up for its field in focus.
    pub fn lift_pending(&self) -> bool {
        self.foreground == Foreground::Placeholder
            && self.keyboard().is_some()
            && self.placeholder.as_ref().is_some_and(|forms| !forms.lifted)
    }

    /// Moves the placeholder app's submit button above the keyboard, as its
    /// layout does some time after the keyboard came up; whether it moved.
    pub fn lift(&mut self) -> bool {
        if !self.lift_pending() {
            return false;
        }
        if let Some(forms) = self.placeholder.as_mut() {
            forms.lifted = true;
        }
        true
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
        apps.push(PLACEHOLDER);
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
        let was = self.focused();
        let launched = self.launch_app(app, launch);
        self.update_keyboard(was);
        launched
    }

    fn launch_app(&mut self, app: &str, launch: &Launch) -> Result<(), DriverError> {
        if app == self.settings_id() {
            self.foreground = Foreground::Settings;
            return Ok(());
        }
        if app == PLACEHOLDER {
            if launch.relaunch || self.placeholder.is_none() {
                self.placeholder = Some(Forms::launch());
            }
            self.foreground = Foreground::Placeholder;
            return Ok(());
        }
        if !self.installed().contains(&app) {
            return Err(self.not_installed(app));
        }
        if launch.relaunch || !self.fixtures.contains_key(app) {
            // The fixtures' launch option (fixtures/CONTRACT.md): the
            // environment variable on iOS, the launch intent's string extra,
            // which is what an `--env` entry becomes, on Android.
            let option = match self.kit.platform {
                Platform::Ios => "FIXTURE_INITIAL_TAB",
                Platform::Android => "initial_tab",
            };
            let tab = launch
                .env
                .iter()
                .find(|(key, _)| key == option)
                .map_or(Tab::Login, |(_, value)| Tab::parse(value));
            self.fixtures.insert(app.to_owned(), Fixture::launch(tab));
        }
        self.foreground = Foreground::Fixture(app.to_owned());
        Ok(())
    }

    /// Whether `app` runs.
    pub fn runs(&self, app: &str) -> bool {
        self.fixtures.contains_key(app) || (app == PLACEHOLDER && self.placeholder.is_some())
    }

    /// Whether `app` is one of the fixture apps.
    pub fn is_fixture(&self, app: &str) -> bool {
        app != self.settings_id() && app != PLACEHOLDER && self.installed().contains(&app)
    }

    /// The windows the device shows while `app` shows its launch splash:
    /// the app's window holding one image and nothing else, as Expo's
    /// static launch splash reads (`image [id=SplashScreenLegacy]`).
    pub fn splash_windows(&self, app: &str) -> Vec<Win> {
        let kit = self.kit;
        let screen = [0.0, 0.0, kit.width(), kit.height()];
        let splash = kit.image("SplashScreenLegacy", screen);
        let mut windows = vec![kit.app_window(app, fixture_title(app), vec![splash])];
        windows.extend(kit.status_bar());
        windows.sort_by_key(|w| std::cmp::Reverse(w.layer));
        windows
    }

    pub fn terminate(&mut self, app: &str) -> Result<(), DriverError> {
        let was = self.focused();
        let stopped = self.stop_app(app);
        self.update_keyboard(was);
        stopped
    }

    fn stop_app(&mut self, app: &str) -> Result<(), DriverError> {
        if !self.installed().contains(&app) {
            return Err(self.not_installed(app));
        }
        self.fixtures.remove(app);
        if app == PLACEHOLDER {
            self.placeholder = None;
        }
        let front = match &self.foreground {
            Foreground::Fixture(front) => front == app,
            Foreground::Settings => app == self.settings_id(),
            Foreground::Placeholder => app == PLACEHOLDER,
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

    /// The screen in front.
    pub fn screen(&self) -> Screen {
        match &self.foreground {
            Foreground::Launcher => Screen::Launcher,
            Foreground::Settings => Screen::Settings,
            Foreground::Placeholder => Screen::Placeholder,
            Foreground::Fixture(app) => {
                let fixture = &self.fixtures[app];
                Screen::Fixture {
                    app: app.clone(),
                    tab: fixture.tab,
                    detail: fixture.detail,
                    alert: fixture.alert,
                    prompt: fixture.prompt,
                }
            }
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

    /// A tap at `at`, in points, that landed on `hit`; whether the screen
    /// changed.
    pub fn tap(&mut self, hit: Option<&str>, at: (f64, f64)) -> bool {
        let before = self.clone();
        let was = self.focused();
        let on_keyboard = self
            .keyboard()
            .is_some_and(|keyboard| tree::contains(keyboard, at.0, at.1));
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
            Foreground::Placeholder => {
                if let Some(forms) = self.placeholder.as_mut() {
                    forms.tap(hit, on_keyboard);
                }
            }
            Foreground::Fixture(_) => {
                if let Some(fixture) = self.front_fixture() {
                    tap_fixture(fixture, hit);
                }
            }
        }
        self.update_keyboard(was);
        *self != before
    }

    /// Text for the field `target` names, or the focused one, entered as
    /// `entry` says into fields that take text as `typing` says: whether
    /// the screen changed, or `None` when no editable field takes the text.
    /// The field takes the focus; on iOS it loses the text while its
    /// keyboard comes up.
    pub fn text(
        &mut self,
        target: Option<&str>,
        text: &str,
        entry: Entry,
        typing: Typing,
    ) -> Option<bool> {
        let before = self.clone();
        let was = self.focused();
        if self.foreground == Foreground::Placeholder {
            let forms = self.placeholder.as_mut()?;
            let field = match target {
                Some(id) => FormField::parse(id)?,
                None => forms.focus?,
            };
            forms.focus = Some(field);
            self.update_keyboard(was);
            if !self.loses_text()
                && let Some(forms) = self.placeholder.as_mut()
            {
                take(forms.field(field), text, entry, typing);
            }
            return Some(*self != before);
        }
        let fixture = self.front_fixture()?;
        if fixture.tab != Tab::Login || fixture.modal() {
            return None;
        }
        let field = match target {
            Some(id) => Field::parse(id)?,
            None => fixture.focus?,
        };
        fixture.focus = Some(field);
        self.update_keyboard(was);
        if !self.loses_text()
            && let Some(fixture) = self.front_fixture()
        {
            take(fixture.field(field), text, entry, typing);
        }
        Some(*self != before)
    }

    /// A key press; whether the screen changed.
    pub fn key(&mut self, code: u32, meta: Option<u32>) -> bool {
        let before = self.clone();
        let was = self.focused();
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
        self.update_keyboard(was);
        *self != before
    }

    fn back(&mut self) {
        match self.foreground.clone() {
            Foreground::Launcher => {}
            Foreground::Settings => self.foreground = Foreground::Launcher,
            Foreground::Placeholder => match self.placeholder.as_mut() {
                Some(forms) if forms.focus.is_some() => {
                    forms.focus = None;
                    forms.lifted = false;
                }
                _ => self.foreground = Foreground::Launcher,
            },
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

    /// A swipe from `from` to `to` in points, after which the list glides
    /// on `glide` points the way the finger went, as far as it scrolls;
    /// whether the screen changed. A swipe that comes down on the keyboard
    /// goes to the keyboard, whose glide typing enters [`GLIDE_WORD`] into
    /// the field in focus, if any, and moves nothing.
    pub fn swipe(&mut self, from: (f64, f64), to: (f64, f64), glide: f64) -> bool {
        let before = self.clone();
        let max = max_offset(self.kit);
        let on_keyboard = self
            .keyboard()
            .is_some_and(|keyboard| tree::contains(keyboard, from.0, from.1));
        let Some(fixture) = self.front_fixture() else {
            return false;
        };
        if on_keyboard {
            if let Some(field) = fixture.focus {
                let value = fixture.field(field);
                if !value.is_empty() {
                    value.push(' ');
                }
                value.push_str(GLIDE_WORD);
            }
        } else if fixture.tab == Tab::List && fixture.detail.is_none() && !fixture.modal() {
            let moved = from.1 - to.1;
            let way = if moved == 0.0 { 0.0 } else { moved.signum() };
            fixture.offset = (fixture.offset + moved + glide * way).clamp(0.0, max);
        }
        *self != before
    }

    /// The windows the device shows, top-most first.
    pub fn windows(&self) -> Vec<Win> {
        let kit = self.kit;
        let mut windows = Vec::new();
        match &self.foreground {
            Foreground::Launcher => windows.push(match kit.platform {
                Platform::Android => kit.app_window("com.android.launcher3", "Home", launcher(kit)),
                // The home screen is SpringBoard's, which the reader reports
                // as system UI named by its process.
                Platform::Ios => kit.springboard(launcher(kit)),
            }),
            Foreground::Settings => {
                let content = settings(kit, self.wifi);
                windows.push(kit.app_window(self.settings_id(), "Settings", content));
            }
            Foreground::Fixture(app) => {
                let keyboard = self.keyboard().is_some();
                fixture_windows(kit, app, &self.fixtures[app], keyboard, &mut windows);
            }
            Foreground::Placeholder => {
                if let Some(forms) = &self.placeholder {
                    let content = placeholder_content(kit, forms);
                    windows.push(kit.app_window(PLACEHOLDER, "Placeholder", content));
                    if self.keyboard().is_some() {
                        windows.push(kit.keyboard(PLACEHOLDER));
                    }
                }
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

/// The title a fixture app's window reports.
fn fixture_title(app: &str) -> &'static str {
    match app {
        "dev.mobileagent.fixture.compose" => "FixtureCompose",
        "dev.mobileagent.fixture.swiftui" => "FixtureSwiftUI",
        _ => "FixtureExpo",
    }
}

/// The windows `fixture`, running as `app`, shows, the keyboard among them
/// when `keyboard` is set.
fn fixture_windows(kit: Kit, app: &str, fixture: &Fixture, keyboard: bool, windows: &mut Vec<Win>) {
    let title = fixture_title(app);
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
            windows.push(kit.springboard(vec![prompt]));
            return;
        }
        Platform::Ios if fixture.alert => {
            // The reader moves an application's alert, with the view that
            // presents it, to a modal window of its own above the app's,
            // which keeps what the alert covers.
            let alert = kit.alert(
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
            );
            let mut app_window = kit.app_window(app, title, content);
            app_window.flags = WindowFlags::EMPTY;
            windows.push(kit.alert_window(app, &app_window, alert));
            windows.push(app_window);
            return;
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
    if keyboard {
        windows.insert(0, kit.keyboard(app));
    }
    windows.push(window);
}

/// The placeholder app's screen, from the top: a heading; the code field, a
/// labelled node without traits holding a box per character, as a React
/// Native code input built of boxes reads; the secret field, a labelled
/// node without traits and nothing below it, as iOS reads a React Native
/// field inside a view marked accessible; a status line; a hint button
/// that the keyboard, once up, covers all but its top 14 points of; the
/// submit button low on the screen, which the layout moves above the
/// keyboard some time after the keyboard came up, as React Native moves a
/// view out of the keyboard's way; and a help button at the bottom, which
/// the keyboard covers.
fn placeholder_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let (width, height) = (kit.width(), kit.height());
    let keyboard = kit.keyboard_top();
    let row = |top: f64| [16.0, top, width - 16.0, top + 48.0];
    let code = kit.line(56.0, 48.0);
    let side = (code[2] - code[0]) / CODE_BOXES as f64;
    let mut digits = forms.code.chars().map(String::from);
    let boxes = (0..CODE_BOXES).map(|k| {
        let left = code[0] + side * k as f64;
        kit.text(
            digits.next().as_deref(),
            [left, code[1], left + side, code[3]],
        )
    });
    let submit = if forms.lifted {
        keyboard - 64.0
    } else {
        height - 174.0
    };
    vec![
        kit.heading(
            "placeholder-title",
            "Placeholder form",
            kit.line(16.0, 32.0),
        ),
        kit.generic("placeholder-code", "Placeholder code", code)
            .children(boxes),
        kit.generic(
            "placeholder-secret",
            "Placeholder secret",
            kit.line(120.0, 48.0),
        ),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            kit.line(184.0, 24.0),
        ),
        kit.button("placeholder-hint", "Placeholder hint", row(keyboard - 14.0)),
        kit.button("placeholder-submit", "Placeholder submit", row(submit)),
        kit.button("placeholder-help", "Placeholder help", row(height - 114.0)),
    ]
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
                let area = [0.0, top, width, bottom];
                let mut shown = Vec::new();
                let rows: Vec<El> = (1..=ROWS)
                    .map(|n| {
                        let row_top = top + f64::from(n - 1) * ROW - fixture.offset;
                        let rect = [0.0, row_top, width, row_top + ROW];
                        let visible = rect[3] > top && rect[1] < bottom;
                        let el = kit.pressable(&format!("row-{n}"), &format!("Row {n}"), rect);
                        if visible {
                            shown.push(n - 1);
                            kit.inside(el, area)
                        } else {
                            hide_unless(el, false)
                        }
                    })
                    .collect();
                let list = kit.list("list", area);
                let list = match kit.platform {
                    Platform::Android => list.scroll(list_scroll(kit, fixture.offset, &shown)),
                    Platform::Ios => list,
                };
                vec![list.children(rows)]
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

/// The slowest finger, in points a second, after which an Android list
/// flings (`ViewConfiguration`'s minimum fling velocity, 50 dp a second).
const MIN_FLING: f64 = 50.0;

/// The fastest fling, in points a second (`ViewConfiguration`'s maximum,
/// 8000 dp a second); UIKit's lists are held to it too.
const MAX_FLING: f64 = 8000.0;

/// How far `platform`'s lists glide on after a finger lifts moving at
/// `speed` points a second, as the platforms' scroll views fling them:
/// Android's `OverScroller` spline with `ViewConfiguration`'s scroll
/// friction, in points, which are its density-independent pixels; UIKit's
/// normal deceleration rate, the speed kept 0.998 of each millisecond.
pub(crate) fn glide(platform: Platform, speed: f64) -> f64 {
    let speed = speed.min(MAX_FLING);
    match platform {
        Platform::Android => {
            if speed < MIN_FLING {
                return 0.0;
            }
            // SplineOverScroller.getSplineFlingDistance: gravity times inches
            // a metre times dots an inch at 160 dpi times 0.84.
            const FRICTION: f64 = 0.015;
            const INFLEXION: f64 = 0.35;
            let physical = 9.80665 * 39.37 * 160.0 * 0.84;
            let deceleration = 0.78f64.ln() / 0.9f64.ln();
            let spline = (INFLEXION * speed / (FRICTION * physical)).ln();
            FRICTION * physical * (deceleration / (deceleration - 1.0) * spline).exp()
        }
        Platform::Ios => {
            const RATE: f64 = 0.998;
            speed * RATE / (1.0 - RATE) / 1000.0
        }
    }
}

/// How far the fixture's list scrolls, in points.
fn max_offset(kit: Kit) -> f64 {
    let (top, bottom) = kit.content();
    f64::from(ROWS) * ROW - (bottom - top)
}

/// The list's scroll semantics as the Android agent reads a RecyclerView's:
/// vertical, back and forward as far as the list scrolls from `offset`, and
/// its rows as items, `shown` (zero-based) inside the viewport.
fn list_scroll(kit: Kit, offset: f64, shown: &[u32]) -> Scroll {
    let mut flags = ScrollFlags::EMPTY;
    if offset > 0.0 {
        flags |= ScrollFlags::BACK;
    }
    if offset < max_offset(kit) {
        flags |= ScrollFlags::FORWARD;
    }
    let index = |n: u32| i32::try_from(n).unwrap_or(i32::MAX);
    Scroll {
        axis: ScrollAxis::VERTICAL,
        flags,
        range: Some(ScrollRange {
            unit: ScrollUnit::ITEMS,
            min: 0,
            max: index(ROWS - 1),
        }),
        position: match (shown.first(), shown.last()) {
            (Some(&first), Some(&last)) => Some(ScrollPosition {
                first: index(first),
                last: index(last),
            }),
            _ => None,
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 10.16: a stroke that lifts moving carries a platform's list on
    /// as far as its fling does: Android's spline, which took the Expo list
    /// about 650 points further than a 600-point stroke over 300 ms on the
    /// emulator, from 50 points a second on; UIKit's deceleration, about
    /// half a second at the finger's speed.
    #[test]
    fn a_list_glides_on_as_far_as_its_platform_flings_it() {
        let close = |actual: f64, expected: f64| (actual - expected).abs() < 1.0;
        assert!(close(glide(Platform::Android, 2000.0), 647.4));
        assert!(close(glide(Platform::Android, 1000.0), 194.3));
        assert_eq!(glide(Platform::Android, 49.0), 0.0);
        assert!(close(glide(Platform::Ios, 2000.0), 998.0));
        assert!(close(glide(Platform::Ios, 400.0), 199.6));
        assert_eq!(
            glide(Platform::Android, f64::INFINITY),
            glide(Platform::Android, 8000.0),
            "the platform's fastest fling"
        );
    }

    /// Task 10.23: a field that takes keys out of order lands each from the
    /// fourth on before its last character, as Bluesky's search field took
    /// "nobel" as "noelb"; it takes whole text as it is. A field with a
    /// maximum length keeps its first characters however text reaches it,
    /// and keys without `replace` add to what a field holds.
    #[test]
    fn fields_take_text_as_their_typing_mode_says() {
        let mut device = Device::new(Platform::Ios);
        device.launch(FIXTURE, &Launch::default()).unwrap();
        /// Enters `text` into the username field and says what it holds.
        fn enter(device: &mut Device, text: &str, entry: Entry, typing: Typing) -> String {
            assert!(
                device
                    .text(Some("login-username"), text, entry, typing)
                    .is_some()
            );
            device.fixtures[FIXTURE].username.clone()
        }
        let fill = Entry::Keys { replace: true };
        assert_eq!(enter(&mut device, "nobel", fill, Typing::Reorder), "noelb");
        assert_eq!(
            enter(&mut device, "nobel", Entry::Whole, Typing::Reorder),
            "nobel"
        );
        let keys = Entry::Keys { replace: false };
        assert_eq!(enter(&mut device, "ab", keys, Typing::Keys), "nobelab");
        assert_eq!(enter(&mut device, "agent", fill, Typing::Keys), "agent");
        assert_eq!(
            enter(&mut device, "nobel", Entry::Whole, Typing::Max(4)),
            "nobe"
        );
        assert_eq!(enter(&mut device, "abcdef", fill, Typing::Max(4)), "abcd");
        assert_eq!(
            enter(&mut device, "abc", Entry::Whole, Typing::Max(4)),
            "abc"
        );
    }

    /// Task 10.16: a swipe moves the list by the finger's travel and then
    /// by the glide, the way the finger went, within the list's range.
    #[test]
    fn a_swipe_moves_the_list_by_the_finger_and_the_glide() {
        let mut device = Device::new(Platform::Android);
        let launch = Launch {
            relaunch: false,
            env: vec![("initial_tab".into(), "list".into())],
        };
        device.launch(FIXTURE, &launch).unwrap();
        let offset = |device: &Device| device.fixtures[FIXTURE].offset;
        assert!(device.swipe((204.0, 700.0), (204.0, 100.0), 650.0));
        assert_eq!(offset(&device), 1250.0);
        assert!(device.swipe((204.0, 100.0), (204.0, 400.0), 100.0));
        assert_eq!(offset(&device), 850.0);
        assert!(device.swipe((204.0, 100.0), (204.0, 700.0), 5000.0));
        assert_eq!(offset(&device), 0.0);
    }

    /// Task 10.32: a stroke that comes down on the keyboard goes to it, as
    /// the strokes of a `scroll` went to Gboard on a Nexus 5X: its glide
    /// typing enters a word into the field in focus, and the list under it
    /// does not move. A keyboard that stays up shows over the List tab too,
    /// and a stroke above it moves the list.
    #[test]
    fn a_stroke_that_comes_down_on_the_keyboard_types_and_moves_nothing() {
        let mut device = Device::new(Platform::Android);
        device.launch(FIXTURE, &Launch::default()).unwrap();
        assert!(device.tap(Some("login-username"), (204.0, 120.0)));
        assert!(device.swipe((204.0, 700.0), (204.0, 300.0), 0.0));
        assert!(device.swipe((204.0, 650.0), (204.0, 250.0), 0.0));
        assert_eq!(device.fixtures[FIXTURE].username, "by by");
        assert!(
            !device.swipe((204.0, 500.0), (204.0, 100.0), 0.0),
            "above the keyboard the Login tab has nothing to scroll"
        );

        let list = Launch {
            relaunch: false,
            env: vec![("initial_tab".into(), "list".into())],
        };
        let mut up = Device::new(Platform::Android).with_keyboard(true);
        up.launch(FIXTURE, &list).unwrap();
        let keyboard = |device: &Device| {
            device
                .windows()
                .iter()
                .any(|w| w.kind == protocol::snapshot::WindowType::INPUT_METHOD)
        };
        assert!(keyboard(&up), "over the List tab, no field in focus");
        assert!(!up.swipe((204.0, 700.0), (204.0, 300.0), 0.0));
        assert!(up.swipe((204.0, 500.0), (204.0, 100.0), 0.0));
        assert_eq!(up.fixtures[FIXTURE].offset, 400.0);

        let mut down = Device::new(Platform::Android);
        down.launch(FIXTURE, &list).unwrap();
        assert!(!keyboard(&down));
        assert!(down.swipe((204.0, 700.0), (204.0, 300.0), 0.0));
        assert_eq!(down.fixtures[FIXTURE].offset, 400.0);
    }
}
