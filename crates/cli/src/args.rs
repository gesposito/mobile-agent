//! The command grammar: agent-browser's where devices have the concept
//! (design D9). Global flags work before or after the command, and each has
//! an environment variable twin. Every command's help ends with examples.

use std::path::PathBuf;

use clap::builder::BoolishValueParser;
use clap::{ArgAction, Args, Parser, Subcommand};

const EXAMPLES: &str = "\
Run 'mobile-agent help agent' for a compact guide to put in an agent's prompt.

Examples:
  mobile-agent devices
  mobile-agent open dev.mobileagent.fixture.expo
  mobile-agent snapshot -i
  mobile-agent click @e3
  mobile-agent wait --settle
  mobile-agent --json snapshot -i";

#[derive(Parser, Debug)]
#[command(
    name = "mobile-agent",
    version,
    about = "Drive Android devices, emulators and iOS Simulators through a live UI mirror, with agent-browser's grammar.",
    disable_help_subcommand = true,
    after_help = EXAMPLES
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

/// Flags every command takes.
#[derive(Args, Debug, Clone, Default)]
pub struct Global {
    /// Print one JSON object: {success, data, error}, plus code and warning when set
    #[arg(long, global = true, help_heading = "Global Options", env = "MOBILE_AGENT_JSON", action = ArgAction::SetTrue, value_parser = BoolishValueParser::new())]
    pub json: bool,

    /// Session whose target claims and refs the command uses [default: default]
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "MOBILE_AGENT_SESSION",
        value_name = "NAME"
    )]
    pub session: Option<String>,

    /// The target's platform: ios or android
    #[arg(long, global = true, help_heading = "Global Options", env = "MOBILE_AGENT_PLATFORM", value_name = "PLATFORM", value_parser = ["ios", "android"], ignore_case = true)]
    pub platform: Option<String>,

    /// The target: a Simulator UDID or adb serial as devices lists it, or a target's name
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "MOBILE_AGENT_DEVICE",
        value_name = "ID"
    )]
    pub device: Option<String>,

    /// Take the target over from the session that holds it
    #[arg(long, global = true, help_heading = "Global Options", env = "MOBILE_AGENT_FORCE", action = ArgAction::SetTrue, value_parser = BoolishValueParser::new())]
    pub force: bool,

    /// Timeout of waiting commands that name none and whose session set none, in ms [default: 10000]
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "MOBILE_AGENT_DEFAULT_TIMEOUT",
        value_name = "MS"
    )]
    pub default_timeout: Option<u64>,

    /// Read settings from this file alone, instead of ~/.mobile-agent/config.json overlaid by ./mobile-agent.json
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "MOBILE_AGENT_CONFIG",
        value_name = "PATH"
    )]
    pub config: Option<PathBuf>,

    /// Stop a daemon this command starts after this long without requests: ms, or a number with s, m or h; 0 never [default: 1h]
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "MOBILE_AGENT_IDLE_TIMEOUT_MS",
        value_name = "TIME"
    )]
    pub idle_timeout: Option<String>,

    /// Whether a daemon this command starts sends taps, swipes and frames on Android emulators through the emulator's gRPC controller when it answers (auto) or through the agent, as on a device (off) [default: auto]
    #[arg(long, global = true, help_heading = "Global Options", env = "MOBILE_AGENT_EMULATOR_CONTROLLER", value_name = "MODE", value_parser = ["auto", "off"], ignore_case = true)]
    pub emulator_controller: Option<String>,

    /// Directory of the daemon's socket [default: $XDG_RUNTIME_DIR/mobile-agent, else ~/.mobile-agent]
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "MOBILE_AGENT_SOCKET_DIR",
        value_name = "DIR"
    )]
    pub socket_dir: Option<PathBuf>,

    /// The daemon executable [default: mobile-agent-daemon beside this executable]
    #[arg(
        long,
        global = true,
        help_heading = "Global Options",
        env = "MOBILE_AGENT_DAEMON_BIN",
        value_name = "PATH",
        hide = true
    )]
    pub daemon_bin: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List connected devices, emulators and Simulators
    #[command(after_help = "\
Examples:
  mobile-agent devices
  mobile-agent devices --platform ios
  mobile-agent --json devices")]
    Devices,

    /// Launch an app and claim its target for this session
    #[command(after_help = "\
Waits for the launched app's first tree, up to the session's default timeout
(10000 ms), for the launch's first effect within a second of the launch, then
up to 3000 ms for its screen to settle; --timeout bounds them all. Once the
app's screen showed and settled, an app another holds the front from, as the
home screen, is launched again; one still not in front fails with
not_in_front, naming what is in front.

Examples:
  mobile-agent open dev.mobileagent.fixture.expo
  mobile-agent open com.example.app --relaunch
  mobile-agent open com.example.app --env FIXTURE_INITIAL_TAB=list
  mobile-agent --device emulator-5554 open com.example.app")]
    Open(OpenArgs),

    /// Stop the app this session opened and release the target
    #[command(after_help = "\
With --keep the target is released and the app keeps running, so the next
session finds it as this one left it.

Examples:
  mobile-agent close
  mobile-agent close --keep
  mobile-agent --session alice close")]
    Close(CloseArgs),

    /// End this session's command in flight, such as a wait another shell left running
    #[command(after_help = "\
The command ends at its next wakeup, within about 100 ms, and fails with
cancelled; a tap or tree read already under way finishes first, and a click
whose tap went in completes. cancel replies once the command has ended,
naming it, and says when nothing was in flight.

Examples:
  mobile-agent cancel
  mobile-agent --session alice cancel")]
    Cancel,

    /// List the screen's elements, one per line with role, name and ref
    #[command(after_help = "\
Lines read - role \"name\" [attributes, ref=eN]: value. Act on the refs, then
take a new snapshot: a ref stays valid while its element stays on screen.

Examples:
  mobile-agent snapshot -i
  mobile-agent snapshot
  mobile-agent snapshot -d 2
  mobile-agent snapshot -i --delta
  mobile-agent --json snapshot -i")]
    Snapshot(SnapshotArgs),

    /// Tap an element (a ref such as @e3, or a selector) or a point in device points
    #[command(
        visible_alias = "tap",
        after_help = "\
Targets: @e3, ref=e3 or e3 from the last snapshot; #id; text=Sign in (contains);
text==Sign in (the whole name or value); role=switch; or x y in device points.

The reply comes once the tap's first frame change and first tree delta have
arrived, or after 200 ms without a change, at most 1 s, and reports them from
the dispatch: dispatchMs, frameChangeMs and treeDeltaMs, the last two absent
when they did not come.

Examples:
  mobile-agent click @e3
  mobile-agent tap @e3
  mobile-agent click '#login-submit'
  mobile-agent click 'text=Sign in'
  mobile-agent click 204 120
  mobile-agent click @e7 --hold 800"
    )]
    Click(ClickArgs),

    /// Clear a text field and enter text into it
    #[command(after_help = "\
fill reads the field back. Text it typed that the field took otherwise is
entered again in one piece, pasted on iOS and set directly on Android, with
a warning naming what the keys left; a secure field is read back by its
length. A field that still holds other text fails with text_mismatch,
naming what it holds.

Examples:
  mobile-agent fill @e2 agent
  mobile-agent fill '#login-password' hunter2
  mobile-agent fill @e2 ''")]
    Fill(FillArgs),

    /// Type text as key events into an element, or into the focused field
    #[command(after_help = "\
Unlike fill, type keeps the field's text and adds to it.

Examples:
  mobile-agent type @e2 ' smith'
  mobile-agent type 'hello world'")]
    Type(TypeArgs),

    /// Press a key such as Enter, Back, Home or Control+a
    #[command(
        visible_alias = "key",
        after_help = "\
Keys: Enter, Tab, Escape, Backspace, Delete, Space, ArrowUp, ArrowDown, ArrowLeft,
ArrowRight, Home, End, PageUp, PageDown, Back, AppSwitch and Menu (Android),
VolumeUp, VolumeDown, Power, F1 to F12, a letter or digit, modifiers joined with +
(Shift, Control, Alt, Meta), or a platform key code of two digits or more.

Examples:
  mobile-agent press Enter
  mobile-agent press Back
  mobile-agent key Home
  mobile-agent press Control+a"
    )]
    Press(PressArgs),

    /// Swipe the finger in a direction from the screen's center, or between two points
    #[command(after_help = "\
The direction is the finger's: swipe up moves the content up and shows what is below.

Examples:
  mobile-agent swipe up
  mobile-agent swipe left 200
  mobile-agent swipe 200 700 200 200 --duration 500")]
    Swipe(SwipeArgs),

    /// Scroll the content up, down, left or right, or until an element is in view
    #[command(after_help = "\
The direction is the content's: scroll down shows what is below. A stroke must
be longer than a tap, more than 8 points on Android and 10 on iOS, or the
scroll fails with stroke_too_short. Without --until each stroke lifts moving,
as a swipe does, so the content glides on after the last, and the scroll
answers once the screen has come to rest, at most 3000 ms after the last
stroke: settled says whether it did, and movedOnMs how long the content moved
on after the last stroke, since a list still gliding takes a tap as a stop.
With --until, each stroke comes to rest before the finger lifts, so the list
moves only as far as the finger, and is AMOUNT long or shorter: at most three
quarters of what the list shows, plus the tap threshold, so the element
cannot pass through the list between two looks. The reply's amount is that
length. The screen settles after each stroke, and the scroll stops once the
element is mostly in view: its centre and at least half of it inside what its
list shows, less the bars over it. Once the element is on screen, the next
stroke brings it in, whichever way that takes, rather than going on by
AMOUNT. The reply's visible is the share of the element's bounds in view,
from 0 to 1 (Android reports an element its list cuts off as the part that
shows). The scroll fails with element_not_found once the content stops moving
or no stroke along its axis brings more of the element in, and waits at most
--timeout.

Examples:
  mobile-agent scroll
  mobile-agent scroll down 600
  mobile-agent scroll up 300 -s @e7
  mobile-agent scroll down 600 --until '#row-150'")]
    Scroll(ScrollArgs),

    /// Save the latest frame as a JPEG
    #[command(after_help = "\
Examples:
  mobile-agent screenshot
  mobile-agent screenshot login.jpg
  mobile-agent screenshot --max-dimension 600 --quality 70")]
    Screenshot(ScreenshotArgs),

    /// Wait for an element, for text, for some milliseconds, for the screen to settle or to change
    #[command(after_help = "\
Element, text and change waits end at --timeout: the session's default
(set timeout), else MOBILE_AGENT_DEFAULT_TIMEOUT or the config's
defaultTimeout, else 10000 ms. wait --settle ends once neither the tree nor
the frames change for the quiet window, or at --timeout, default 3000 ms;
within a second of this session's last click, fill, type, press, swipe or
scroll it first waits for that action's first effect, until a second after
the action, and input names the action. On a Simulator, once quiet, it reads
the tree the frames changed and settles again when that read finds it
changed, so a menu whose items come after a pause in its frames is listed
whole right after it. endedBy says which signal changed last (tree, frame,
both or none, after an action nothing within 1 s of it) or timeout, and
stillChanging what kept changing. wait --change ends once the
listing in the options of this session's last snapshot changed from what
that snapshot showed, at once for a change that came before it, and prints
what moved (endedBy: tree, frame or both) and the delta snapshot --delta
prints; a change the listing does not show leaves it waiting. With --frames
a frame change ends it too, for a canvas, a map or a screen the tree does
not describe.

Examples:
  mobile-agent wait --settle
  mobile-agent wait --change
  mobile-agent wait --change --frames
  mobile-agent wait @e5
  mobile-agent wait '#spinner' --state hidden
  mobile-agent wait --text 'Welcome' --timeout 5000
  mobile-agent wait 500")]
    Wait(WaitArgs),

    /// Read an element's text, value or bounds
    #[command(after_help = "\
Examples:
  mobile-agent get text @e5
  mobile-agent get value '#login-username'
  mobile-agent get box @e3")]
    Get(GetArgs),

    /// Compare the screen with this session's last snapshot
    #[command(after_help = "\
Examples:
  mobile-agent snapshot -i
  mobile-agent click @e3
  mobile-agent diff snapshot")]
    Diff(DiffArgs),

    /// Run several commands in order, from quoted arguments or a JSON array on stdin
    #[command(after_help = "\
Examples:
  mobile-agent batch 'snapshot -i' 'click @e3' 'wait --settle'
  mobile-agent batch --bail 'open com.example.app' 'fill #user agent' 'press Enter'
  echo '[[\"snapshot\", \"-i\"], [\"click\", \"@e2\"]]' | mobile-agent --json batch")]
    Batch(BatchArgs),

    /// Report the daemon, its targets and its sessions
    #[command(after_help = "\
Examples:
  mobile-agent status
  mobile-agent --json status")]
    Status,

    /// Change a setting of this session
    #[command(after_help = "\
Examples:
  mobile-agent set timeout 5000
  mobile-agent --session alice set timeout 20000")]
    Set(SetArgs),

    /// Check the toolchains, the adb server, emulators and the daemon
    #[command(after_help = "\
Prints a line per check (ok, warn, fail or skip) with a hint where something
can be done, then the Xcode compatibility table the Simulator driver supplies.
The checks of a device driver that MOBILE_AGENT_DRIVERS leaves out ask the host
nothing and are skipped as not consulted: with simulator alone, no adb runs.
Exits with status 1 when a check fails; warnings do not fail it.

Examples:
  mobile-agent doctor
  mobile-agent --json doctor")]
    Doctor,

    /// Stop the daemon; the next command starts a new one
    #[command(after_help = "\
Examples:
  mobile-agent shutdown")]
    Shutdown,

    /// Print help for a command, or 'help agent' for the guide for agents
    #[command(after_help = "\
Examples:
  mobile-agent help agent
  mobile-agent help snapshot
  mobile-agent help get text")]
    Help(HelpArgs),
}

#[derive(Args, Debug)]
pub struct OpenArgs {
    /// The app: an Android package or an iOS bundle id
    pub app: String,
    /// Restart the app if it is running
    #[arg(long)]
    pub relaunch: bool,
    /// A launch environment variable, repeatable; an intent string extra on Android
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,
    /// Longest wait for the app's first tree and its settling, in ms [default: the session's timeout for the tree, then 3000 to settle]
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct CloseArgs {
    /// Release the target and leave the app running, for the next session to attach to
    #[arg(long)]
    pub keep: bool,
}

#[derive(Args, Debug)]
pub struct SnapshotArgs {
    /// Only elements with refs: actionable elements and named headings, cells and list items
    #[arg(short, long)]
    pub interactive: bool,
    /// Deepest nesting level to list
    #[arg(short, long, value_name = "N")]
    pub depth: Option<u64>,
    /// Answer with agent-browser's delta against this session's last delta snapshot
    #[arg(long)]
    pub delta: bool,
    /// With --delta, send the whole listing
    #[arg(long, requires = "delta")]
    pub full: bool,
}

/// The help of a selector's timeout.
const SELECTOR_TIMEOUT: &str = "Longest wait for a selector's element, in ms [default: the session's, else MOBILE_AGENT_DEFAULT_TIMEOUT, else 10000]";

#[derive(Args, Debug)]
pub struct ClickArgs {
    /// A ref (@e3, ref=e3, e3), a selector (#id, text=..., text==..., role=...) or x and y in device points
    #[arg(required = true, num_args = 1..=2, value_name = "TARGET")]
    pub target: Vec<String>,
    /// Hold that long, in ms: a long press
    #[arg(long, value_name = "MS")]
    pub hold: Option<u64>,
    #[arg(long, value_name = "MS", help = SELECTOR_TIMEOUT)]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct FillArgs {
    /// The field: a ref or a selector
    pub target: String,
    /// The text that replaces the field's content
    #[arg(allow_hyphen_values = true)]
    pub text: String,
    #[arg(long, value_name = "MS", help = SELECTOR_TIMEOUT)]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct TypeArgs {
    /// [TARGET] TEXT: the field as a ref or selector, then the text; without a field, the focused one
    #[arg(required = true, num_args = 1..=2, allow_hyphen_values = true, value_name = "ARGS")]
    pub args: Vec<String>,
    #[arg(long, value_name = "MS", help = SELECTOR_TIMEOUT)]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct PressArgs {
    /// A key name, a letter or digit, modifiers joined with +, or a platform key code
    pub key: String,
}

#[derive(Args, Debug)]
pub struct SwipeArgs {
    /// DIRECTION [DISTANCE] (up, down, left or right, then points, default 300), or X1 Y1 X2 Y2
    #[arg(required = true, num_args = 1..=4, value_name = "ARGS")]
    pub args: Vec<String>,
    /// How long the stroke takes, in ms [default: 300]
    #[arg(long, value_name = "MS")]
    pub duration: Option<u64>,
}

#[derive(Args, Debug)]
pub struct ScrollArgs {
    /// up, down, left or right [default: down]
    pub direction: Option<String>,
    /// How far the content moves, in points [default: 300]
    pub amount: Option<f64>,
    /// Scroll inside this element (a ref or a selector) instead of the screen
    #[arg(short, long, value_name = "TARGET")]
    pub selector: Option<String>,
    /// Keep scrolling until this element (a ref or a selector) is mostly in view
    #[arg(long, value_name = "TARGET")]
    pub until: Option<String>,
    #[arg(long, value_name = "MS", help = SELECTOR_TIMEOUT)]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct ScreenshotArgs {
    /// Where to save the JPEG [default: a new file in the temporary directory]
    pub path: Option<PathBuf>,
    /// JPEG quality, 1 to 100 [default: 80]
    #[arg(long, value_name = "N")]
    pub quality: Option<u8>,
    /// Longest edge in pixels [default: 1000]
    #[arg(long, value_name = "PX")]
    pub max_dimension: Option<u32>,
}

#[derive(Args, Debug)]
pub struct WaitArgs {
    /// An element (a ref or a selector) or a number of milliseconds
    pub target: Option<String>,
    /// Wait for an element whose name or value contains this text
    #[arg(long, conflicts_with = "target")]
    pub text: Option<String>,
    /// What to wait for: visible (the default), hidden or enabled on screen, attached or detached in the tree, in view or not
    #[arg(long, value_name = "STATE")]
    pub state: Option<String>,
    /// Wait until neither the tree nor the frames change for the quiet window
    #[arg(long, conflicts_with_all = ["target", "text"])]
    pub settle: bool,
    /// The quiet window of --settle, in ms [default: 200]
    #[arg(long, value_name = "MS", requires = "settle")]
    pub quiet: Option<u64>,
    /// Settle on tree changes alone, ignoring frames (for video)
    #[arg(long, requires = "settle")]
    pub tree_only: bool,
    /// Wait until the listing in the options of this session's last snapshot changes, then print the delta
    #[arg(long, conflicts_with_all = ["target", "text", "settle"])]
    pub change: bool,
    /// With --change, end the wait at a frame change too, for a canvas, a map or a screen the tree does not describe
    #[arg(long, requires = "change")]
    pub frames: bool,
    /// Longest wait, in ms [default: the session's, else MOBILE_AGENT_DEFAULT_TIMEOUT, else 10000; with --settle 3000]
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct GetArgs {
    #[command(subcommand)]
    pub what: GetWhat,
}

#[derive(Subcommand, Debug)]
pub enum GetWhat {
    /// An element's value, else its name
    #[command(after_help = "\
Examples:
  mobile-agent get text @e5
  mobile-agent get text '#login-status'")]
    Text(ElementArg),
    /// A field's value
    #[command(after_help = "\
Examples:
  mobile-agent get value @e2")]
    Value(ElementArg),
    /// An element's bounds in device points: x, y, width and height
    #[command(after_help = "\
Examples:
  mobile-agent get box @e3")]
    Box(ElementArg),
}

#[derive(Args, Debug)]
pub struct ElementArg {
    /// A ref or a selector
    pub target: String,
    #[arg(long, value_name = "MS", help = SELECTOR_TIMEOUT)]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct DiffArgs {
    #[command(subcommand)]
    pub what: DiffWhat,
}

#[derive(Subcommand, Debug)]
pub enum DiffWhat {
    /// Compare the screen with this session's last snapshot, element by element
    #[command(after_help = "\
Examples:
  mobile-agent diff snapshot
  mobile-agent --json diff snapshot")]
    Snapshot(DiffSnapshotArgs),
}

#[derive(Args, Debug)]
pub struct DiffSnapshotArgs {
    /// Without a last snapshot, list only elements with refs
    #[arg(short, long)]
    pub interactive: bool,
    /// Without a last snapshot, the deepest nesting level to list
    #[arg(short, long, value_name = "N")]
    pub depth: Option<u64>,
}

#[derive(Args, Debug)]
pub struct BatchArgs {
    /// Stop at the first command that fails
    #[arg(long)]
    pub bail: bool,
    /// Commands as quoted strings, such as "snapshot -i"; without any, a JSON array of argv arrays on stdin
    #[arg(value_name = "COMMAND")]
    pub commands: Vec<String>,
}

#[derive(Args, Debug)]
pub struct SetArgs {
    #[command(subcommand)]
    pub what: SetWhat,
}

#[derive(Subcommand, Debug)]
pub enum SetWhat {
    /// The default timeout of this session's waiting commands, in ms, at most 600000
    #[command(after_help = "\
Examples:
  mobile-agent set timeout 5000")]
    Timeout {
        /// Milliseconds
        ms: u64,
    },
}

#[derive(Args, Debug)]
pub struct HelpArgs {
    /// agent for the guide, or a command such as snapshot or get text
    #[arg(value_name = "TOPIC")]
    pub topic: Vec<String>,
}
