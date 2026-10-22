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

/// The character a key of a US keyboard types without Shift for the
/// character `c`: a letter in lower case, a digit for the symbol above it,
/// and so on along the rows; any other as it is.
fn unshifted(c: char) -> char {
    const SHIFTED: &str = "~!@#$%^&*()_+{}|:\"<>?";
    const PLAIN: &str = "`1234567890-=[]\\;',./";
    if c.is_ascii_uppercase() {
        return c.to_ascii_lowercase();
    }
    SHIFTED
        .chars()
        .position(|shifted| shifted == c)
        .and_then(|index| PLAIN.chars().nth(index))
        .unwrap_or(c)
}

/// Enters `text` into a field that holds `value`, as `entry` says, the
/// field taking it as `typing` says.
fn take(value: &mut String, text: &str, entry: Entry, typing: Typing) {
    match entry {
        Entry::Whole if typing == Typing::Unshifted => {}
        Entry::Whole => {
            value.clear();
            value.push_str(text);
        }
        Entry::Keys { replace } => {
            if replace {
                value.clear();
            }
            for (index, key) in text.chars().enumerate() {
                let key = if typing == Typing::Unshifted {
                    unshifted(key)
                } else {
                    key
                };
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
/// showed in the production-app smoke test of 2026-10-09, under placeholder names (see
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
    /// Whether iOS's edit menu is up over the field in focus.
    menu: bool,
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
            menu: false,
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
    /// The list's field, a text field that brings the keyboard up.
    Query,
    /// The code step's field: on the first visit a labelled node without
    /// traits whose name spells the digits it holds, on the second a
    /// field without an area inside such a node.
    Pin,
    /// The paste sheet's title field, which has no identifier and whose
    /// name changes once it holds text.
    Title,
    /// The dialog board's name field, under the dialog.
    Name,
    /// One of the rebuilt board's two fields, by its place.
    Rebuilt(usize),
    /// The accessory board's field, drawn above the keyboard.
    Accessory,
    /// The raised board's field, whose focus raises its card.
    Raised,
}

impl FormField {
    /// The field `id` names on `board`.
    fn on(board: Board, id: &str) -> Option<FormField> {
        match (board, id) {
            (Board::Form, "placeholder-code") => Some(FormField::Code),
            (Board::Form, "placeholder-secret") => Some(FormField::Secret),
            (Board::List | Board::ListLow, LIST_FIELD) => Some(FormField::Query),
            (
                Board::Code
                | Board::CodeAgain
                | Board::CodeNamed
                | Board::CodeMasked
                | Board::CodeBare
                | Board::CodeValued,
                CODE_FIELD | CODE_INPUT,
            ) => Some(FormField::Pin),
            (Board::Sheet, TITLE_FIELD) => Some(FormField::Title),
            (Board::Dialog, NAME_FIELD) => Some(FormField::Name),
            (Board::Accessory, ACCESSORY_FIELD) => Some(FormField::Accessory),
            (Board::Raised, RAISED_FIELD) => Some(FormField::Raised),
            (Board::Rebuilt, id) => REBUILT_FIELDS
                .iter()
                .position(|field| *field == id)
                .map(FormField::Rebuilt),
            _ => None,
        }
    }
}

/// The launch option that names the placeholder app's screen, as an
/// environment variable on iOS and an intent extra on Android.
const BOARD_OPTION: &str = "PLACEHOLDER_SCREEN";

/// Which screen the placeholder app shows, as [`BOARD_OPTION`] names it:
/// the form of the production-app smoke test of 2026-10-09, or one of the screens the
/// production-app run of 2026-10-12 met (rn-app-fixes, task 2.2), in the
/// structure of the masked listings' sections, laid out in an iPhone's
/// points on either platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Board {
    /// The form (see [`form_content`]).
    Form,
    /// The consent screen (section 1a): a heading half under the secondary
    /// footer button, a 12-point text under the primary one.
    Consent,
    /// The creation tab (section 1b, task 3.7): a list scrolling under the
    /// header listed before it, its disabled primary button drawn after it,
    /// over most of the paste control, and a footer drawn after that, over
    /// the rest.
    Creation,
    /// The creation tab without the footer: nothing is drawn over the list
    /// below the disabled button.
    CreationOpen,
    /// The creation tab as React Native's accessibility lists it (task 5.1;
    /// task 4.7, N6): the footer's view holds nothing, its tabs listed after
    /// it as its siblings.
    CreationRn,
    /// The same tab as the fifth production-app run met it (task 5.6, N6 and
    /// N12): the footer's view drawn from the primary button's top, so that
    /// it holds the paste control and the button beside it whole, as the
    /// run's listings left both out.
    CreationHeld,
    /// The feed (section 2): a card partly above its list's top, where the
    /// header listed before the list is drawn.
    Feed,
    /// The list (section 3): a header listed before the list, down to 122
    /// points, and the list's field, which brings the keyboard up.
    List,
    /// The list under a header down to 430 points, which leaves 109 points
    /// of the list above the keyboard.
    ListLow,
    /// The code step on its first visit (section 4): the code field is a
    /// labelled node without traits whose name spells the digits it holds.
    Code,
    /// The code step on its second visit (section 4): the code field's node
    /// holds a field without an area, and so gets no ref.
    CodeAgain,
    /// The code step as the third production-app run met it (task 3.7): the
    /// code field is a labelled node without traits named by its six boxes,
    /// each the digit it holds or a dash, then by its label.
    CodeNamed,
    /// The same step with the field's boxes masked: a bullet for each digit.
    CodeMasked,
    /// The same step on its second visit: the field's node holds no field,
    /// only an empty view out of sight, the app's input not in the tree; a
    /// tap on the node gives that input the focus and brings the keyboard up.
    CodeBare,
    /// The second visit as the sixth production-app run met it with the
    /// keyboard up (task 6.5, N14): the node also shows a value, the last
    /// digit its boxes hold or, with none, an empty box's dash, as a single
    /// box's placeholder or the last digit an emptying left, which the
    /// read-back took for what the field holds.
    CodeValued,
    /// The paste sheet (section 7): the title field, without an identifier,
    /// in one of four unidentified views of one class, which gains a counter
    /// when the field takes the focus; its name changes once it holds text.
    Sheet,
    /// A dialog the app marks modal over a field (task 5.6, N12): the name
    /// field under the dialog, which asks to stay or leave.
    Dialog,
    /// The two shapes of a switch row the Simulator's reader gave Settings'
    /// Display & Text Size (real-app-fixes, task 3.3): a table cell around a
    /// labelled switch that holds its label and its control, the cell, the
    /// switch and the label sharing one identifier, as Bold Text's row; and
    /// a named cell around its label and its control, the three sharing
    /// one, as On/Off Labels' row. A tap flips a row's switch only on its
    /// control, as UIKit's does.
    Switches,
    /// Two fields the app lays out again once they hold text (real-app-fixes,
    /// task 5.5): the first with an identifier, the second without one,
    /// each in a view of one class while it is empty and of another once
    /// it holds text, so that the tree holds a new node for the field as
    /// its text goes in, at the same place, as the Simulator's reader gave
    /// a sheet's title field during a fill.
    Rebuilt,
    /// A field drawn above the keyboard, in the place the keyboard's host
    /// keeps for the field's input accessory view, as the Simulator's
    /// reader gave a browser's address bar in focus (real-app-fixes, task
    /// 5.6): the field and a button beside it from 510 to 558 points, above
    /// what the keyboard draws from 566, a button above them and one under
    /// the keys; the keyboard's window, once the field has the focus,
    /// reaches from 498 points down.
    Accessory,
    /// A banner system UI brings up over the app (action-verdicts, task
    /// 2.1): a heading, a button high on the screen, and a button lower down
    /// whose tap brings up, in a window of system UI's above the app's, a
    /// banner holding a button, over the first button's centre and inset 8
    /// points from the screen's sides, as a notification comes down over an
    /// app between a snapshot and a tap.
    Banner,
    /// A field in a card low on the screen, which the tap that gives the
    /// field the focus raises to the screen's top (action-verdicts, task
    /// 2.2): the field takes a new node there, in a view of another class,
    /// as a map app's search field did on the second demo pass between the
    /// tap that focused it and the text `fill` entered.
    Raised,
}

impl Board {
    fn parse(name: &str) -> Board {
        match name {
            "consent" => Board::Consent,
            "creation" => Board::Creation,
            "creation-open" => Board::CreationOpen,
            "creation-rn" => Board::CreationRn,
            "creation-held" => Board::CreationHeld,
            "feed" => Board::Feed,
            "list" => Board::List,
            "list-low" => Board::ListLow,
            "code" => Board::Code,
            "code-again" => Board::CodeAgain,
            "code-named" => Board::CodeNamed,
            "code-masked" => Board::CodeMasked,
            "code-bare" => Board::CodeBare,
            "code-valued" => Board::CodeValued,
            "sheet" => Board::Sheet,
            "dialog" => Board::Dialog,
            "switches" => Board::Switches,
            "rebuilt" => Board::Rebuilt,
            "accessory" => Board::Accessory,
            "banner" => Board::Banner,
            "raised" => Board::Raised,
            _ => Board::Form,
        }
    }
}

/// The list's field.
const LIST_FIELD: &str = "placeholder-id-2";

/// The code step's field: the labelled node with the identifier.
const CODE_FIELD: &str = "placeholder-id-3";

/// The field inside the code step's node on its second visit, which the app
/// gives no identifier.
const CODE_INPUT: &str = "placeholder-code-input";

/// The paste sheet's title field, which the app gives no identifier.
const TITLE_FIELD: &str = "placeholder-title";

/// The dialog board's name field.
const NAME_FIELD: &str = "placeholder-name";

/// The identifiers the rows of the switches board share with what they
/// hold.
const SWITCH_ROWS: [&str; 2] = ["placeholder-id-1", "placeholder-id-2"];

/// The accessory board's field.
const ACCESSORY_FIELD: &str = "placeholder-id-1";

/// The rebuilt board's fields: the first by its identifier, the second by
/// the key the fake finds it by, which the app gives no identifier.
const REBUILT_FIELDS: [&str; 2] = ["placeholder-id-1", "placeholder-field-2"];

/// The banner board's button whose tap brings the banner up.
const BANNER_CALL: &str = "placeholder-id-2";

/// The raised board's field.
const RAISED_FIELD: &str = "placeholder-id-1";

/// Where the switches board's rows lie, in points, as their cells.
const SWITCH_CELLS: [[f64; 4]; 2] = [[20.0, 134.0, 382.0, 187.0], [20.0, 293.0, 382.0, 345.0]];

/// Where the switches board's controls lie, in points: the 63x28 switch
/// UIKit draws at a row's right end.
const SWITCH_CONTROLS: [[f64; 4]; 2] = [[305.0, 146.0, 368.0, 174.0], [299.0, 305.0, 362.0, 333.0]];

/// The creation tab's list, in points: down to the bottom of the screen,
/// under the footer.
const CREATION_LIST: [f64; 4] = [0.0, 122.0, 402.0, 874.0];

/// How far the creation tab's list scrolls, in points.
const CREATION_MAX: f64 = 400.0;

/// The creation tab's disabled primary button, which takes a tap and does
/// nothing.
const CREATION_DISABLED: &str = "placeholder-id-9";

/// Where the creation tab's primary button lies, in points.
const CREATION_PRIMARY: [f64; 4] = [20.0, 708.0, 382.0, 764.0];

/// Where the creation tab's footer lies, in points: from the primary
/// button's bottom to the bottom of the screen.
const CREATION_FOOTER: [f64; 4] = [0.0, 764.0, 402.0, 874.0];

/// Where the footer's view lies on the creation tab as the fifth production-app
/// run met it, in points: from the primary button's top to the bottom of
/// the screen.
const CREATION_HELD_FOOTER: [f64; 4] = [0.0, 708.0, 402.0, 874.0];

/// The running placeholder app.
#[derive(Clone, Debug, PartialEq)]
struct Forms {
    board: Board,
    code: String,
    secret: String,
    /// What the list's field holds.
    query: String,
    /// What the code step's field holds.
    pin: String,
    /// What the paste sheet's title field holds.
    title: String,
    /// What the dialog board's name field holds.
    name: String,
    focus: Option<FormField>,
    status: String,
    /// Whether the layout has moved the submit button above the keyboard,
    /// which it does some time after the keyboard came up.
    lifted: bool,
    /// Whether the footer shows, which the hint brings up.
    footer: bool,
    /// How far the creation tab's list is scrolled, in points.
    offset: f64,
    /// Whether each of the switches board's switches is on.
    switches: [bool; 2],
    /// What each of the rebuilt board's fields holds.
    rebuilt: [String; 2],
    /// What the accessory board's field holds.
    accessory: String,
    /// Whether the banner board's banner shows.
    banner: bool,
    /// What the raised board's field holds.
    raised: String,
}

impl Forms {
    fn launch(board: Board) -> Forms {
        Forms {
            board,
            code: String::new(),
            secret: String::new(),
            query: String::new(),
            pin: String::new(),
            title: String::new(),
            name: String::new(),
            focus: None,
            status: match board {
                Board::Form => "Not submitted",
                _ => "Not tapped",
            }
            .to_owned(),
            lifted: false,
            footer: false,
            offset: 0.0,
            switches: [false; 2],
            rebuilt: Default::default(),
            accessory: String::new(),
            banner: false,
            raised: String::new(),
        }
    }

    /// A stroke from `from` to `to`, gliding on `glide` points the way the
    /// finger went: the creation tab's list moves by them, as far as it
    /// scrolls, when the stroke comes down on it, and on the board in React
    /// Native's shape not on a view drawn over it, the primary button or the
    /// footer, which takes the touch; whether it moved.
    fn swipe(&mut self, from: (f64, f64), to: (f64, f64), glide: f64) -> bool {
        let drawn_over = matches!(self.board, Board::CreationRn | Board::CreationHeld)
            && (tree::contains(CREATION_PRIMARY, from.0, from.1)
                || tree::contains(creation_footer(self.board), from.0, from.1));
        if !matches!(
            self.board,
            Board::Creation | Board::CreationOpen | Board::CreationRn | Board::CreationHeld
        ) || !tree::contains(CREATION_LIST, from.0, from.1)
            || drawn_over
        {
            return false;
        }
        let moved = from.1 - to.1;
        let way = if moved == 0.0 { 0.0 } else { moved.signum() };
        let offset = (self.offset + moved + glide * way).clamp(0.0, CREATION_MAX);
        let changed = offset != self.offset;
        self.offset = offset;
        changed
    }

    fn field(&mut self, field: FormField) -> &mut String {
        match field {
            FormField::Code => &mut self.code,
            FormField::Secret => &mut self.secret,
            FormField::Query => &mut self.query,
            FormField::Pin => &mut self.pin,
            FormField::Title => &mut self.title,
            FormField::Name => &mut self.name,
            FormField::Rebuilt(k) => &mut self.rebuilt[k],
            FormField::Accessory => &mut self.accessory,
            FormField::Raised => &mut self.raised,
        }
    }

    /// A tap at `at`, in points, that landed on `hit`, or on the keyboard
    /// when `on_keyboard` says so: a key of the number pad, entered into the
    /// field in focus. A field takes the focus. On the screens of the
    /// production-app run the status names what else the tap landed on, but
    /// for the creation tab's disabled button, which does nothing; on the
    /// switches board a tap on a control flips its switch.
    fn tap(&mut self, hit: Option<&str>, at: (f64, f64), on_keyboard: bool) {
        if self.board == Board::Switches {
            for (on, control) in self.switches.iter_mut().zip(SWITCH_CONTROLS) {
                if tree::contains(control, at.0, at.1) {
                    *on = !*on;
                }
            }
        }
        if on_keyboard {
            if let Some(field) = self.focus {
                self.field(field).push(PAD_KEY);
            }
            return;
        }
        if let Some(field) = hit.and_then(|id| FormField::on(self.board, id)) {
            self.focus = Some(field);
            return;
        }
        if self.board == Board::Banner && hit == Some(BANNER_CALL) {
            self.banner = true;
        }
        if self.board != Board::Form {
            if hit != Some(CREATION_DISABLED) {
                self.status = format!("Tapped {}", hit.unwrap_or("nothing"));
                self.focus = None;
            }
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
            Some("placeholder-hint") => {
                self.status = "Hint shown".to_owned();
                self.footer = true;
            }
            Some("placeholder-accept") => self.status = "Accepted".to_owned(),
            Some("placeholder-terms") => self.status = "Terms shown".to_owned(),
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
    /// Up with its keys hidden, drawing nothing, as [`Slide::Hidden`] has
    /// it: a hardware keyboard serves the field in focus.
    Hidden,
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
        Some(if self.accessory() {
            self.kit.drawn_under_accessory()
        } else {
            self.kit.keyboard(app).rect
        })
    }

    /// Whether the field in focus has an input accessory view on iOS: the
    /// placeholder app's accessory board's.
    fn accessory(&self) -> bool {
        self.kit.platform == Platform::Ios
            && self.foreground == Foreground::Placeholder
            && self
                .placeholder
                .as_ref()
                .is_some_and(|forms| forms.board == Board::Accessory)
    }

    /// The keyboard's window over `app`, the app in front: the keyboard
    /// while it shows, its host reaching above it for the input accessory
    /// view of a field that has one, or, on iOS, a window that draws nothing
    /// while its keys are hidden.
    fn keyboard_window(&self, app: &str) -> Option<Win> {
        if self.keyboard().is_some() && self.accessory() {
            Some(self.kit.accessory_keyboard(app))
        } else if self.keyboard().is_some() {
            Some(self.kit.keyboard(app))
        } else if self.keys == Keys::Hidden && self.kit.platform == Platform::Ios {
            Some(self.kit.hidden_keyboard(app))
        } else {
            None
        }
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
        } else if self.slide == Slide::HiddenPad {
            self.keys = if self.pad_focused() {
                Keys::Down
            } else {
                Keys::Hidden
            };
        } else if !was || self.keys == Keys::Down {
            self.keys = match self.slide {
                Slide::AtOnce => Keys::Up,
                Slide::After(_) => Keys::Coming,
                Slide::Never => Keys::Down,
                Slide::Hidden | Slide::HiddenPad => Keys::Hidden,
            };
        }
    }

    /// Whether the field in focus takes a number pad: one of the placeholder
    /// app's code fields.
    fn pad_focused(&self) -> bool {
        self.foreground == Foreground::Placeholder
            && self
                .placeholder
                .as_ref()
                .is_some_and(|forms| matches!(forms.focus, Some(FormField::Code | FormField::Pin)))
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
                let board = launch
                    .env
                    .iter()
                    .find(|(key, _)| key == BOARD_OPTION)
                    .map_or(Board::Form, |(_, value)| Board::parse(value));
                self.placeholder = Some(Forms::launch(board));
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
                    forms.tap(hit, at, on_keyboard);
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
                Some(id) => FormField::on(forms.board, id)?,
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
        let ios = self.kit.platform == Platform::Ios;
        let fixture = self.front_fixture()?;
        if fixture.tab != Tab::Login || fixture.modal() {
            return None;
        }
        let field = match target {
            Some(id) => Field::parse(id)?,
            None => fixture.focus?,
        };
        // Text for a field the input names goes after a tap on it, which on
        // iOS opens the edit menu over the field when it has the focus.
        let tapped_in_focus = target.is_some() && fixture.focus == Some(field);
        fixture.focus = Some(field);
        self.update_keyboard(was);
        if !self.loses_text()
            && let Some(fixture) = self.front_fixture()
        {
            take(fixture.field(field), text, entry, typing);
            // The text a paste brings closes the menu; one that lands
            // nothing leaves it up.
            let lost = typing == Typing::Unshifted && entry == Entry::Whole;
            fixture.menu = ios && tapped_in_focus && lost;
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
        if self.foreground == Foreground::Placeholder {
            return !on_keyboard
                && self
                    .placeholder
                    .as_mut()
                    .is_some_and(|forms| forms.swipe(from, to, glide));
        }
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
                let keyboard = self.keyboard_window(app);
                fixture_windows(kit, app, &self.fixtures[app], keyboard, &mut windows);
            }
            Foreground::Placeholder => {
                if let Some(forms) = &self.placeholder {
                    let content = placeholder_content(kit, forms);
                    windows.push(kit.app_window(PLACEHOLDER, "Placeholder", content));
                    windows.extend(self.keyboard_window(PLACEHOLDER));
                    if forms.banner {
                        windows.push(banner(kit));
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
    // A tap closes iOS's edit menu.
    fixture.menu = false;
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

/// The windows `fixture`, running as `app`, shows, with `keyboard`, the
/// keyboard's window, when one shows.
fn fixture_windows(
    kit: Kit,
    app: &str,
    fixture: &Fixture,
    keyboard: Option<Win>,
    windows: &mut Vec<Win>,
) {
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
    if let Some(field) = fixture.focus.filter(|_| fixture.menu) {
        // Below the field in focus, as iOS puts it.
        let below = match field {
            Field::Username => kit.line(72.0, 48.0),
            Field::Password => kit.line(136.0, 48.0),
        }[3];
        let menu = [16.0, below + 9.0, 341.0, below + 53.0];
        window.root.children.push(kit.edit_menu(menu));
    }
    if let Some(keyboard) = keyboard {
        windows.insert(0, keyboard);
    }
    windows.push(window);
}

/// The placeholder app's screen, as its board says.
fn placeholder_content(kit: Kit, forms: &Forms) -> Vec<El> {
    match forms.board {
        Board::Form => form_content(kit, forms),
        Board::Consent => consent_content(kit, forms),
        Board::Creation | Board::CreationOpen => creation_content(kit, forms),
        Board::CreationRn | Board::CreationHeld => creation_rn_content(kit, forms),
        Board::Feed => feed_content(kit, forms),
        Board::List => list_content(kit, forms, 122.0),
        Board::ListLow => list_content(kit, forms, 430.0),
        Board::Code
        | Board::CodeAgain
        | Board::CodeNamed
        | Board::CodeMasked
        | Board::CodeBare
        | Board::CodeValued => code_content(kit, forms),
        Board::Sheet => sheet_content(kit, forms),
        Board::Dialog => dialog_content(kit, forms),
        Board::Switches => switches_content(kit, forms),
        Board::Rebuilt => rebuilt_content(kit, forms),
        Board::Accessory => accessory_content(kit, forms),
        Board::Banner => banner_content(kit, forms),
        Board::Raised => raised_content(kit, forms),
    }
}

/// The code field's name on the code step of the third production-app run
/// (task 3.7): its boxes, each the digit it holds, a bullet in the digit's
/// place when `masked`, or a dash when empty, then its label.
fn boxes_name(pin: &str, masked: bool) -> String {
    let mut parts: Vec<String> = pin
        .chars()
        .take(CODE_BOXES)
        .map(|digit| {
            if masked {
                "\u{2022}".to_owned()
            } else {
                digit.to_string()
            }
        })
        .collect();
    parts.resize(CODE_BOXES, "\u{2013}".to_owned());
    parts.extend(["Verification code".to_owned(), "required".to_owned()]);
    parts.join(", ")
}

/// The code step (section 4 of the masked listings): the step's 12-point
/// heading, the code field, a labelled node without traits with the
/// identifier, whose name spells the digits it holds after its label, a
/// resend button, the submit button and the status line. On the second
/// visit the code field's node also holds a field without an area, which
/// the tree reports as off the screen, so the node gets no ref; and two
/// more labelled nodes with identifiers hold no field and two of them. On
/// the step as the third production-app run met it, the field's name spells its
/// boxes before its label ([`boxes_name`]), and on its second visit the
/// field's node holds an empty view out of sight and no field, so it gets
/// no ref either.
fn code_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let digits: Vec<String> = forms.pin.chars().map(String::from).collect();
    let name = match forms.board {
        Board::CodeNamed | Board::CodeBare | Board::CodeValued => boxes_name(&forms.pin, false),
        Board::CodeMasked => boxes_name(&forms.pin, true),
        _ if digits.is_empty() => "placeholder-generic-2".to_owned(),
        _ => format!("placeholder-generic-2, {}", digits.join(" ")),
    };
    let hidden = |key: &str| {
        kit.field(
            key,
            "",
            &forms.pin,
            false,
            forms.focus == Some(FormField::Pin),
            [20.0, 518.67, 20.0, 518.67],
        )
        .key(key)
        .set(protocol::snapshot::NodeFlags::VISIBLE, false)
    };
    let mut field = kit.generic(CODE_FIELD, &name, [20.0, 518.67, 382.0, 574.67]);
    let mut content = vec![
        kit.heading(
            "placeholder-heading-2",
            "placeholder-heading-2",
            [32.0, 494.67, 144.67, 506.67],
        )
        .key("placeholder-heading-2"),
    ];
    if forms.board == Board::CodeAgain {
        field = field.child(hidden(CODE_INPUT));
        content.push(
            kit.generic(
                "placeholder-id-4",
                "placeholder-generic-3",
                [20.0, 420.0, 382.0, 460.0],
            )
            .child(kit.image("placeholder-image-1", [28.0, 424.0, 60.0, 456.0])),
        );
        content.push(
            kit.generic(
                "placeholder-id-5",
                "placeholder-generic-4",
                [20.0, 464.0, 382.0, 488.0],
            )
            .children([hidden("placeholder-pair-1"), hidden("placeholder-pair-2")]),
        );
    }
    if matches!(forms.board, Board::CodeBare | Board::CodeValued) {
        field = field.child(
            kit.group([20.0, 518.67, 20.0, 518.67])
                .set(protocol::snapshot::NodeFlags::VISIBLE, false),
        );
    }
    if forms.board == Board::CodeValued {
        let last = forms.pin.chars().take(CODE_BOXES).last();
        field = field.text(last.map_or("\u{2013}".to_owned(), String::from));
    }
    content.extend([
        field,
        kit.button(
            "placeholder-button-4",
            "placeholder-button-4",
            [8.0, 586.67, 107.33, 618.67],
        )
        .key("placeholder-button-4"),
        kit.button(
            "placeholder-id-2",
            "placeholder-button-3",
            [20.0, 638.67, 382.0, 686.67],
        ),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [20.0, 700.0, 382.0, 720.0],
        ),
    ]);
    content
}

/// The paste sheet (section 7 of the masked listings): a heading, a close
/// button, a label, the title field without an identifier in an
/// unidentified view, which also holds a counter while the field has the
/// focus, a second label, a field with an identifier in another such view,
/// the disabled primary button and a second close button identical to the
/// first. The title field's name changes once it holds text.
fn sheet_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let focused = forms.focus == Some(FormField::Title);
    let name = if forms.title.is_empty() {
        "placeholder-textbox-1"
    } else {
        "placeholder-textbox-3"
    };
    let mut view = kit.group([20.0, 212.0, 382.0, 268.0]).child(
        kit.field(
            TITLE_FIELD,
            name,
            &forms.title,
            false,
            focused,
            [28.0, 216.0, 374.0, 260.0],
        )
        .key(TITLE_FIELD),
    );
    if focused {
        view = view.child(kit.label(None, "placeholder-text-1", [300.0, 262.0, 374.0, 268.0]));
    }
    let label = |text: &str, rect: [f64; 4]| kit.group(rect).desc(text);
    let close = |key: &str, rect: [f64; 4]| kit.button(key, "placeholder-button-1", rect).key(key);
    vec![
        kit.group([0.0, 120.0, kit.width(), kit.height()])
            .children([
                kit.heading(
                    "placeholder-heading-1",
                    "placeholder-heading-1",
                    [20.0, 140.0, 330.0, 172.0],
                )
                .key("placeholder-heading-1"),
                close("placeholder-close-1", [338.0, 136.0, 382.0, 176.0]),
                label("placeholder-generic-1", [20.0, 188.0, 382.0, 208.0]),
                view,
                label("placeholder-generic-2", [20.0, 284.0, 382.0, 304.0]),
                kit.group([20.0, 308.0, 382.0, 364.0]).child(kit.field(
                    "placeholder-id-1",
                    "placeholder-textbox-2",
                    "",
                    false,
                    false,
                    [28.0, 312.0, 374.0, 356.0],
                )),
                kit.button(
                    "placeholder-id-2",
                    "placeholder-button-2",
                    [20.0, 380.0, 382.0, 428.0],
                )
                .set(protocol::snapshot::NodeFlags::ENABLED, false),
                close("placeholder-close-2", [338.0, 440.0, 382.0, 480.0]),
                kit.label(
                    Some("placeholder-status"),
                    &forms.status,
                    [20.0, 500.0, 382.0, 520.0],
                ),
            ]),
    ]
}

/// The list with the keyboard up (section 3 of the masked listings): a
/// header listed before the list, down to `header` points, holding a
/// button, the screen's heading and the status line; and the list, from
/// there down to the bottom of the screen, holding the row scrolled up
/// behind the header, the field, which brings the keyboard up, and a
/// button the keyboard's top crosses.
fn list_content(kit: Kit, forms: &Forms, header: f64) -> Vec<El> {
    let width = kit.width();
    vec![
        kit.group([0.0, 0.0, width, header]).children([
            kit.button(
                "placeholder-button-1",
                "placeholder-button-1",
                [20.0, 70.0, 64.0, 114.0],
            )
            .key("placeholder-button-1"),
            kit.heading(
                "placeholder-heading-1",
                "placeholder-heading-1",
                [76.0, 76.0, 306.67, 98.0],
            )
            .key("placeholder-heading-1"),
            kit.label(
                Some("placeholder-status"),
                &forms.status,
                [76.0, 100.0, 306.0, 114.0],
            ),
        ]),
        kit.scroll_view([0.0, header, width, kit.height()])
            .children([
                kit.button(
                    "placeholder-id-1",
                    "placeholder-button-2",
                    [38.0, 2.0, 364.0, 46.0],
                ),
                kit.field(
                    LIST_FIELD,
                    "placeholder-textbox-1",
                    &forms.query,
                    false,
                    forms.focus == Some(FormField::Query),
                    [38.0, 452.0, 364.0, 496.0],
                ),
                kit.button(
                    "placeholder-id-3",
                    "placeholder-button-5",
                    [38.0, 508.0, 364.0, 552.0],
                ),
            ]),
    ]
}

/// The consent screen (section 1a of the masked listings): a scroll view
/// holding the status line, the eighth and ninth headings, a 12-point text
/// wholly under the secondary footer button, as the fifth production-app run's
/// text at 737 points (task 5.6, N12), the text that straddles the gap
/// between the footer buttons and the 12-point text, and the secondary and
/// primary footer buttons listed after it, drawn over what is scrolled
/// under them, the ninth heading's lower 11 points under the secondary one,
/// the 12-point text's upper 9 under the primary one. Nothing but the
/// status line carries an identifier the tree reports.
fn consent_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let height = kit.height();
    let scrolled = kit.scroll_view([0.0, 62.0, kit.width(), height]).children([
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [33.0, 62.0, 369.0, 76.0],
        ),
        kit.heading(
            "placeholder-heading-8",
            "placeholder-heading-8",
            [33.0, 596.0, 369.0, 618.0],
        )
        .key("placeholder-heading-8"),
        kit.heading(
            "placeholder-heading-9",
            "placeholder-heading-9",
            [33.0, 709.0, 90.33, 731.0],
        )
        .key("placeholder-heading-9"),
        kit.label(None, "placeholder-text-7", [33.0, 737.0, 369.0, 749.0]),
        kit.label(None, "placeholder-text-6", [33.0, 755.0, 369.0, 771.0]),
        kit.label(None, "placeholder-text-8", [33.0, 819.0, 369.0, 831.0])
            .key("placeholder-text-8"),
    ]);
    vec![
        scrolled,
        kit.button(
            "placeholder-button-1",
            "placeholder-button-1",
            [20.0, 720.0, 382.0, 764.0],
        )
        .key("placeholder-button-1"),
        kit.button(
            "placeholder-button-2",
            "placeholder-button-2",
            [20.0, 772.0, 382.0, 828.0],
        )
        .key("placeholder-button-2"),
    ]
}

/// `el` and what it holds moved `offset` points up, as a list scrolled that
/// far moves them, and hidden off the screen, `height` points tall.
fn scrolled_by(el: El, offset: f64, height: f64) -> El {
    let mut el = el;
    el.rect[1] -= offset;
    el.rect[3] -= offset;
    el.children = el
        .children
        .into_iter()
        .map(|child| scrolled_by(child, offset, height))
        .collect();
    let shown = el.rect[3] > 0.0 && el.rect[1] < height;
    hide_unless(el, shown)
}

/// The creation tab (section 1b of the masked listings, task 3.7): the
/// header, listed before the tab's list, holding the app's button and the
/// status line; the list, scrolled `forms.offset` points, down to the
/// bottom of the screen, holding a heading, the button beside the paste
/// control and the paste control, in a row 38 points tall that their lower
/// 10 points overflow, 38 of their 48 points under the primary button at
/// the list's start, and more content below; the primary button, disabled,
/// listed after the list; and, but on the open board, a footer listed after
/// it, from the button's bottom down, holding the tab bar and drawn over the
/// list's lower part. The list says which ways it scrolls, as the
/// Simulator's reader reads a scroll view's.
fn creation_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let width = kit.width();
    let scrolled = |el: El| scrolled_by(el, forms.offset, kit.height());
    let content = [
        kit.heading(
            "placeholder-heading-18",
            "placeholder-heading-18",
            [20.0, 130.0, 382.0, 162.0],
        )
        .key("placeholder-heading-18"),
        kit.heading(
            "placeholder-heading-24",
            "placeholder-heading-24",
            [20.0, 648.0, 382.0, 670.0],
        )
        .key("placeholder-heading-24"),
        kit.group([0.0, 726.0, width, 764.0]).children([
            kit.button(
                "placeholder-button-19",
                "placeholder-button-19",
                [20.0, 726.0, 197.0, 774.0],
            )
            .key("placeholder-button-19"),
            kit.button(
                "placeholder-id-8",
                "placeholder-button-20",
                [205.0, 726.0, 382.0, 774.0],
            ),
        ]),
        kit.heading(
            "placeholder-heading-25",
            "placeholder-heading-25",
            [20.0, 806.0, 382.0, 828.0],
        )
        .key("placeholder-heading-25"),
        kit.button(
            "placeholder-button-22",
            "placeholder-button-22",
            [20.0, 1100.0, 382.0, 1148.0],
        )
        .key("placeholder-button-22"),
    ]
    .map(scrolled);
    let mut flags = ScrollFlags::EMPTY;
    if forms.offset > 0.0 {
        flags |= ScrollFlags::BACK;
    }
    if forms.offset < CREATION_MAX {
        flags |= ScrollFlags::FORWARD;
    }
    let scroll = Scroll {
        axis: ScrollAxis::VERTICAL,
        flags,
        range: None,
        position: None,
    };
    let mut screen = vec![
        kit.group([0.0, 62.0, width, 122.0]).children([
            kit.button(
                "placeholder-id-7",
                "placeholder-button-9",
                [20.0, 70.0, 64.0, 114.0],
            ),
            kit.label(
                Some("placeholder-status"),
                &forms.status,
                [76.0, 76.0, 306.0, 108.0],
            ),
        ]),
        kit.scroll_view(CREATION_LIST)
            .scroll(scroll)
            .children(content),
        kit.button(
            CREATION_DISABLED,
            "placeholder-button-21",
            [20.0, 708.0, 382.0, 764.0],
        )
        .set(protocol::snapshot::NodeFlags::ENABLED, false),
    ];
    if forms.board == Board::CreationOpen {
        return screen;
    }
    screen.push(
        kit.group([0.0, 764.0, width, kit.height()]).children(
            [
                ("placeholder-tab-1", "placeholder-generic-3", false),
                ("placeholder-tab-2", "placeholder-heading-18", true),
                ("placeholder-tab-3", "placeholder-generic-4", false),
            ]
            .into_iter()
            .enumerate()
            .map(|(k, (id, label, selected))| {
                let left = width / 3.0 * k as f64;
                kit.tab(
                    id,
                    label,
                    selected,
                    [left, 791.0, left + width / 3.0, kit.height()],
                )
            }),
        ),
    );
    screen
}

/// Where the footer's view lies on the creation tab in React Native's shape
/// that `board` shows.
fn creation_footer(board: Board) -> [f64; 4] {
    if board == Board::CreationHeld {
        CREATION_HELD_FOOTER
    } else {
        CREATION_FOOTER
    }
}

/// The creation tab as React Native's accessibility lists it on the
/// Simulator (task 5.1; task 4.7, N6; the mirror's `create-rn-ios`): a view
/// that draws something is a node holding nothing, what it lays out listed
/// after it as its siblings. The header's view, then its button and the
/// status line; the list, in a view wrapping it, scrolled `forms.offset`
/// points, down to the bottom of the screen, holding a heading, the button
/// beside the paste control and the paste control, 38 of their 48 points
/// under the primary button, and a last button that the list's end brings
/// above the primary button; the primary button, disabled; the footer's
/// view, from the button's bottom down, drawn over the list's lower part,
/// or from its top down on the board of the fifth production-app run; and then
/// the footer's tabs.
fn creation_rn_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let width = kit.width();
    let scrolled = |el: El| scrolled_by(el, forms.offset, kit.height());
    let content = [
        kit.heading(
            "placeholder-heading-18",
            "placeholder-heading-18",
            [20.0, 130.0, 382.0, 162.0],
        )
        .key("placeholder-heading-18"),
        kit.heading(
            "placeholder-heading-24",
            "placeholder-heading-24",
            [20.0, 648.0, 382.0, 670.0],
        )
        .key("placeholder-heading-24"),
        kit.button(
            "placeholder-button-19",
            "placeholder-button-19",
            [20.0, 726.0, 197.0, 774.0],
        )
        .key("placeholder-button-19"),
        kit.button(
            "placeholder-id-8",
            "placeholder-button-20",
            [205.0, 726.0, 382.0, 774.0],
        ),
        kit.button(
            "placeholder-button-22",
            "placeholder-button-22",
            [20.0, 1000.0, 382.0, 1048.0],
        )
        .key("placeholder-button-22"),
    ]
    .map(scrolled);
    let mut flags = ScrollFlags::EMPTY;
    if forms.offset > 0.0 {
        flags |= ScrollFlags::BACK;
    }
    if forms.offset < CREATION_MAX {
        flags |= ScrollFlags::FORWARD;
    }
    let scroll = Scroll {
        axis: ScrollAxis::VERTICAL,
        flags,
        range: None,
        position: None,
    };
    let mut screen = vec![
        kit.group([0.0, 62.0, width, 122.0]),
        kit.button(
            "placeholder-id-7",
            "placeholder-button-9",
            [20.0, 70.0, 64.0, 114.0],
        ),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [76.0, 76.0, 306.0, 108.0],
        ),
        kit.group(CREATION_LIST).child(
            kit.scroll_view(CREATION_LIST)
                .scroll(scroll)
                .children(content),
        ),
        kit.button(CREATION_DISABLED, "placeholder-button-21", CREATION_PRIMARY)
            .set(protocol::snapshot::NodeFlags::ENABLED, false),
        kit.group(creation_footer(forms.board)),
    ];
    screen.extend(
        [
            ("placeholder-tab-1", "placeholder-generic-3", false),
            ("placeholder-tab-2", "placeholder-heading-18", true),
            ("placeholder-tab-3", "placeholder-generic-4", false),
        ]
        .into_iter()
        .enumerate()
        .map(|(k, (id, label, selected))| {
            let left = width / 3.0 * k as f64;
            kit.tab(
                id,
                label,
                selected,
                [left, 791.0, left + width / 3.0, kit.height()],
            )
        }),
    );
    screen
}

/// The dialog board (task 5.6, N12): a heading, the name field and a
/// button, the status line, and a dialog the app marks modal over the
/// screen, listed after them, holding its question and two buttons, as
/// React Native lays out a modal view.
fn dialog_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let width = kit.width();
    vec![
        kit.heading(
            "placeholder-heading-1",
            "placeholder-heading-1",
            [20.0, 70.0, 382.0, 108.0],
        )
        .key("placeholder-heading-1"),
        kit.field(
            NAME_FIELD,
            "placeholder-textbox-1",
            &forms.name,
            false,
            forms.focus == Some(FormField::Name),
            [20.0, 300.0, 382.0, 348.0],
        ),
        kit.button(
            "placeholder-id-1",
            "placeholder-button-1",
            [20.0, 364.0, 382.0, 412.0],
        ),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [20.0, 764.0, 382.0, 784.0],
        ),
        kit.group([0.0, 0.0, width, kit.height()])
            .with(protocol::snapshot::NodeFlags::MODAL)
            .child(kit.group([40.0, 330.0, width - 40.0, 520.0]).children([
                kit.label(
                    None,
                    "placeholder-text-1",
                    [56.0, 346.0, width - 56.0, 386.0],
                ),
                kit.button(
                    "placeholder-id-2",
                    "placeholder-button-2",
                    [56.0, 410.0, width - 56.0, 454.0],
                ),
                kit.button(
                    "placeholder-id-3",
                    "placeholder-button-3",
                    [56.0, 462.0, width - 56.0, 506.0],
                ),
            ])),
    ]
}

/// The feed (section 2 of the masked listings): the header, listed before
/// the feed's list, holding a button, the feed's heading and a button with
/// a count; the list, its first card at 41 points, 116 tall, 81 of them
/// above the list's top, where the header is; and the status line below
/// the list. The header carries no identifier the tree reports.
/// The switches board (real-app-fixes, task 3.3): Bold Text's row, a table
/// cell without a name around a labelled switch, which holds its label, a
/// button, and its control, an unnamed switch, the cell, the switch and the
/// label sharing an identifier; then On/Off Labels' row, a named cell whose
/// value is its switch's state around its label, a text, and its control, a
/// switch, the three sharing one; and the status line. Classes are the
/// Simulator reader's, and their Android counterparts there.
fn switches_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let ios = kit.platform == Platform::Ios;
    let class = |ios_class: &'static str, android_class: &'static str| {
        if ios { ios_class } else { android_class }
    };
    let switch = |k: usize, rect: [f64; 4]| {
        El::new(
            class("XCUIElementTypeSwitch", "android.widget.Switch"),
            rect,
        )
        .with(protocol::snapshot::NodeFlags::CHECKABLE)
        .set(protocol::snapshot::NodeFlags::CHECKED, forms.switches[k])
    };
    let cell = |k: usize| {
        El::new(
            class("XCUIElementTypeCell", "android.view.ViewGroup"),
            SWITCH_CELLS[k],
        )
        .id(SWITCH_ROWS[k])
    };
    let labelled = cell(0).child(
        switch(0, [36.0, 146.0, 366.0, 174.0])
            .id(SWITCH_ROWS[0])
            .desc("placeholder-switch-1")
            .children([
                El::new(
                    class("XCUIElementTypeButton", "android.widget.Button"),
                    [36.0, 150.0, 107.0, 170.0],
                )
                .id(SWITCH_ROWS[0])
                .desc("placeholder-switch-1"),
                switch(0, SWITCH_CONTROLS[0]),
            ]),
    );
    let named = cell(1)
        .desc("placeholder-switch-2")
        .text(if forms.switches[1] { "1" } else { "0" })
        .children([
            El::new(
                class("XCUIElementTypeStaticText", "android.widget.TextView"),
                [36.0, 309.0, 150.0, 329.0],
            )
            .id(SWITCH_ROWS[1])
            .desc("placeholder-switch-2"),
            switch(1, SWITCH_CONTROLS[1])
                .id(SWITCH_ROWS[1])
                .desc("placeholder-switch-2"),
        ]);
    vec![
        labelled,
        named,
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [20.0, 764.0, 382.0, 784.0],
        ),
    ]
}

/// The rebuilt board (real-app-fixes, task 5.5): a heading, then the two
/// fields, each in a view without a name of one class while the field is
/// empty and of another once it holds text, a plain view and a cell on iOS,
/// so that the field's node is a new one at the same place; and the status
/// line.
fn rebuilt_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let ios = kit.platform == Platform::Ios;
    let field = |k: usize, top: f64| {
        let rect = [20.0, top, 382.0, top + 48.0];
        let focused = forms.focus == Some(FormField::Rebuilt(k));
        let field = kit.field(
            REBUILT_FIELDS[k],
            &format!("placeholder-textbox-{}", k + 1),
            &forms.rebuilt[k],
            false,
            focused,
            rect,
        );
        let field = if k == 0 {
            field
        } else {
            field.key(REBUILT_FIELDS[k])
        };
        let view = match (forms.rebuilt[k].is_empty(), ios) {
            (true, true) => "XCUIElementTypeOther",
            (false, true) => "XCUIElementTypeCell",
            (true, false) => "android.view.ViewGroup",
            (false, false) => "android.widget.FrameLayout",
        };
        El::new(view, rect).child(field)
    };
    vec![
        kit.heading(
            "placeholder-heading-1",
            "placeholder-heading-1",
            [20.0, 70.0, 382.0, 108.0],
        ),
        field(0, 140.0),
        field(1, 220.0),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [20.0, 764.0, 382.0, 784.0],
        ),
    ]
}

/// The accessory board (real-app-fixes, task 5.6): a heading, a button, the
/// field with a button beside it from 510 to 558 points, a button from 700
/// points, under the keyboard's keys once they are up, and the status line.
fn accessory_content(kit: Kit, forms: &Forms) -> Vec<El> {
    vec![
        kit.heading(
            "placeholder-heading-1",
            "placeholder-heading-1",
            [20.0, 70.0, 382.0, 108.0],
        ),
        kit.button(
            "placeholder-id-4",
            "placeholder-button-4",
            [20.0, 400.0, 382.0, 448.0],
        ),
        kit.field(
            ACCESSORY_FIELD,
            "placeholder-textbox-1",
            &forms.accessory,
            false,
            forms.focus == Some(FormField::Accessory),
            [20.0, 510.0, 294.0, 558.0],
        ),
        kit.button(
            "placeholder-id-2",
            "placeholder-button-2",
            [346.0, 510.0, 394.0, 558.0],
        ),
        kit.button(
            "placeholder-id-3",
            "placeholder-button-3",
            [20.0, 700.0, 382.0, 748.0],
        ),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [20.0, 764.0, 382.0, 784.0],
        ),
    ]
}

/// The banner board (action-verdicts, task 2.1): a heading, a button from
/// 140 to 188 points, under the banner's place, the button whose tap brings
/// the banner up, and the status line.
fn banner_content(kit: Kit, forms: &Forms) -> Vec<El> {
    vec![
        kit.heading(
            "placeholder-heading-1",
            "placeholder-heading-1",
            [20.0, 70.0, 382.0, 108.0],
        ),
        kit.button(
            "placeholder-id-1",
            "placeholder-button-1",
            [20.0, 140.0, 382.0, 188.0],
        ),
        kit.button(
            BANNER_CALL,
            "placeholder-button-2",
            [20.0, 400.0, 382.0, 448.0],
        ),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [20.0, 764.0, 382.0, 784.0],
        ),
    ]
}

/// The raised board (action-verdicts, task 2.2): a heading, the field, from
/// 580 to 628 points in a plain view while it has no focus and, once it
/// has, from 120 to 168 in a cell on iOS and a frame layout on Android, a
/// new node at another place, and the status line.
fn raised_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let raised = forms.focus == Some(FormField::Raised);
    let top = if raised { 120.0 } else { 580.0 };
    let rect = [20.0, top, 382.0, top + 48.0];
    let field = kit.field(
        RAISED_FIELD,
        "placeholder-textbox-1",
        &forms.raised,
        false,
        raised,
        rect,
    );
    let view = match (raised, kit.platform) {
        (false, Platform::Ios) => "XCUIElementTypeOther",
        (true, Platform::Ios) => "XCUIElementTypeCell",
        (false, Platform::Android) => "android.view.ViewGroup",
        (true, Platform::Android) => "android.widget.FrameLayout",
    };
    vec![
        kit.heading(
            "placeholder-heading-1",
            "placeholder-heading-1",
            [20.0, 70.0, 382.0, 108.0],
        ),
        El::new(view, rect).child(field),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [20.0, 764.0, 382.0, 784.0],
        ),
    ]
}

/// The banner board's banner, in system UI's window over the app: from 128
/// to 200 points and inset 8 points from the screen's sides, so that it
/// spans neither side of the button under it, holding a button of its own
/// across it.
fn banner(kit: Kit) -> Win {
    let width = kit.width();
    let button = kit.button(
        "placeholder-id-5",
        "placeholder-button-5",
        [16.0, 136.0, width - 16.0, 192.0],
    );
    kit.banner([8.0, 128.0, width - 8.0, 200.0], vec![button])
}

fn feed_content(kit: Kit, forms: &Forms) -> Vec<El> {
    let width = kit.width();
    vec![
        kit.group([0.0, 62.0, width, 122.0])
            .key("placeholder-feed-header")
            .children([
                kit.button(
                    "placeholder-id-1",
                    "placeholder-button-1",
                    [20.0, 70.0, 64.0, 114.0],
                ),
                kit.heading(
                    "placeholder-heading-1",
                    "placeholder-heading-1",
                    [76.0, 76.0, 306.67, 108.0],
                )
                .key("placeholder-heading-1"),
                kit.button(
                    "placeholder-id-2",
                    "placeholder-button-2",
                    [338.0, 70.0, 382.0, 114.0],
                )
                .text("placeholder-value-1"),
            ]),
        kit.scroll_view([0.0, 122.0, width, 760.0]).children([
            kit.button(
                "placeholder-id-4",
                "placeholder-button-4",
                [37.0, 41.0, 365.0, 157.0],
            ),
            kit.label(None, "placeholder-text-2", [37.0, 165.0, 365.0, 185.0]),
            kit.button(
                "placeholder-id-5",
                "placeholder-button-5",
                [37.0, 201.0, 365.0, 317.0],
            ),
        ]),
        kit.label(
            Some("placeholder-status"),
            &forms.status,
            [16.0, 764.0, 386.0, 784.0],
        ),
    ]
}

/// The form, from the top: a heading; the code field, a labelled node
/// without traits holding a box per character, as a React Native code
/// input built of boxes reads; the secret field, a labelled node without
/// traits and nothing below it, as iOS reads a React Native field inside a
/// view marked accessible; a status line; a hint button that the keyboard,
/// once up, covers all but its top 14 points of; the submit button low on
/// the screen, which the layout moves above the keyboard some time after
/// the keyboard came up, as React Native moves a view out of the keyboard's
/// way; a help button, which the keyboard covers; a terms button at the
/// bottom; and, once the hint brings it up, a footer listed after the terms
/// button and drawn over it, a toolbar holding an accept button, as a fixed
/// footer is drawn over what scrolls under it.
fn form_content(kit: Kit, forms: &Forms) -> Vec<El> {
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
    let footer = forms.footer.then(|| {
        kit.toolbar("placeholder-footer", [0.0, height - 60.0, width, height])
            .child(kit.button(
                "placeholder-accept",
                "Placeholder accept",
                [16.0, height - 54.0, width - 16.0, height - 6.0],
            ))
    });
    let mut content = vec![
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
        kit.button(
            "placeholder-terms",
            "Placeholder terms",
            [16.0, height - 52.0, width - 16.0, height - 8.0],
        ),
    ];
    content.extend(footer);
    content
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
        assert_eq!(enter(&mut device, "A@b", fill, Typing::Unshifted), "a2b");
        assert!(!device.fixtures[FIXTURE].menu);
        assert_eq!(
            enter(&mut device, "A@b", Entry::Whole, Typing::Unshifted),
            "a2b",
            "a paste lands nothing"
        );
        assert!(
            device.fixtures[FIXTURE].menu,
            "the tap on the field in focus left the edit menu up"
        );
        assert!(
            device
                .windows()
                .iter()
                .flat_map(|window| &window.root.children)
                .any(|child| child
                    .children
                    .iter()
                    .any(|c| c.class == "_UIEditMenuContainerView"))
        );
        device.tap(Some("login-username"), (24.0, 158.0));
        assert!(!device.fixtures[FIXTURE].menu, "a tap closes it");
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

    /// Design D34: a keyboard that slides in comes up some time after the
    /// field took the focus; until then an iOS field loses the text that
    /// reaches it, the text that focused it included, while Android's text
    /// set types nothing and lands. A keyboard that never comes shows no
    /// window, and the field takes text.
    #[test]
    fn an_ios_field_loses_text_while_its_keyboard_slides_in() {
        let slide = Slide::After(Duration::from_millis(400));
        let username = |device: &Device| device.fixtures[FIXTURE].username.clone();
        let fill = Entry::Keys { replace: true };

        let mut ios = Device::new(Platform::Ios).with_slide(slide);
        ios.launch(FIXTURE, &Launch::default()).unwrap();
        assert!(ios.text(Some("login-username"), "agent", fill, Typing::Keys) == Some(true));
        assert_eq!(username(&ios), "", "lost while the keyboard comes up");
        assert_eq!(ios.keyboard_coming(), Some(Duration::from_millis(400)));
        assert!(ios.keyboard().is_none());
        assert!(ios.raise_keyboard());
        assert!(ios.keyboard().is_some());
        ios.text(Some("login-username"), "agent", fill, Typing::Keys);
        assert_eq!(username(&ios), "agent");

        let mut android = Device::new(Platform::Android).with_slide(slide);
        android.launch(FIXTURE, &Launch::default()).unwrap();
        android.text(Some("login-username"), "agent", Entry::Whole, Typing::Whole);
        assert_eq!(username(&android), "agent");
        assert!(android.keyboard().is_none(), "its keyboard still comes up");

        let mut never = Device::new(Platform::Ios).with_slide(Slide::Never);
        never.launch(FIXTURE, &Launch::default()).unwrap();
        never.text(Some("login-username"), "agent", fill, Typing::Keys);
        assert_eq!(username(&never), "agent");
        assert_eq!(never.keyboard_coming(), None);
        assert!(never.keyboard().is_none());
    }
}
