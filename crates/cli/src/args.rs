//! The command grammar: agent-browser's where devices have the concept
//! (design D9). Global flags work before or after the command, and each has
//! an environment variable twin.

use std::path::PathBuf;

use clap::builder::BoolishValueParser;
use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "mobile-agent",
    version,
    about = "Drive Android devices, emulators and iOS Simulators through a live UI mirror, with agent-browser's grammar."
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
    #[arg(long, global = true, env = "MOBILE_AGENT_JSON", action = ArgAction::SetTrue, value_parser = BoolishValueParser::new())]
    pub json: bool,

    /// Session whose target claims and refs the command uses [default: default]
    #[arg(long, global = true, env = "MOBILE_AGENT_SESSION", value_name = "NAME")]
    pub session: Option<String>,

    /// Take the target over from the session that holds it
    #[arg(long, global = true, env = "MOBILE_AGENT_FORCE", action = ArgAction::SetTrue, value_parser = BoolishValueParser::new())]
    pub force: bool,

    /// Read settings from this file alone, instead of ~/.mobile-agent/config.json overlaid by ./mobile-agent.json
    #[arg(long, global = true, env = "MOBILE_AGENT_CONFIG", value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Directory of the daemon's socket [default: $XDG_RUNTIME_DIR/mobile-agent, else ~/.mobile-agent]
    #[arg(
        long,
        global = true,
        env = "MOBILE_AGENT_SOCKET_DIR",
        value_name = "DIR"
    )]
    pub socket_dir: Option<PathBuf>,

    /// The daemon executable [default: mobile-agent-daemon beside this executable]
    #[arg(
        long,
        global = true,
        env = "MOBILE_AGENT_DAEMON_BIN",
        value_name = "PATH",
        hide = true
    )]
    pub daemon_bin: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List connected devices, emulators and Simulators
    Devices,
    /// Launch an app and claim its target for this session
    Open(OpenArgs),
    /// Stop the app this session opened and release the target
    Close,
    /// List the screen's elements, one per line with role, name and ref
    Snapshot(SnapshotArgs),
    /// Tap an element (a ref such as @e3, or a selector) or a point in device points
    #[command(visible_alias = "tap")]
    Click(ClickArgs),
    /// Clear a text field and enter text into it
    Fill(FillArgs),
    /// Type text as key events into an element, or into the focused field
    Type(TypeArgs),
    /// Press a key such as Enter, Back, Home or Control+a
    #[command(visible_alias = "key")]
    Press(PressArgs),
    /// Swipe the finger in a direction from the screen's center, or between two points
    Swipe(SwipeArgs),
    /// Scroll the content up, down, left or right
    Scroll(ScrollArgs),
    /// Save the latest frame as a JPEG
    Screenshot(ScreenshotArgs),
    /// Wait for an element, for text, for some milliseconds or for the screen to settle
    Wait(WaitArgs),
    /// Read an element's text, value or bounds
    Get(GetArgs),
    /// Compare the screen with this session's last snapshot
    Diff(DiffArgs),
    /// Report the daemon, its targets and its sessions
    Status,
    /// Stop the daemon; the next command starts a new one
    Shutdown,
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
    /// Longest wait for the app's screen to settle, in ms
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
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

#[derive(Args, Debug)]
pub struct ClickArgs {
    /// A ref (@e3, ref=e3, e3), a selector (#id, text=..., role=...) or x and y in device points
    #[arg(required = true, num_args = 1..=2, value_name = "TARGET")]
    pub target: Vec<String>,
    /// Hold that long, in ms: a long press
    #[arg(long, value_name = "MS")]
    pub hold: Option<u64>,
    /// Longest wait for a selector's element, in ms
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct FillArgs {
    /// The field: a ref or a selector
    pub target: String,
    /// The text that replaces the field's content
    #[arg(allow_hyphen_values = true)]
    pub text: String,
    /// Longest wait for a selector's element, in ms
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct TypeArgs {
    /// [TARGET] TEXT: the field as a ref or selector, then the text; without a field, the focused one
    #[arg(required = true, num_args = 1..=2, allow_hyphen_values = true, value_name = "ARGS")]
    pub args: Vec<String>,
    /// Longest wait for a selector's element, in ms
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
}

#[derive(Args, Debug)]
pub struct PressArgs {
    /// A key name, a letter or digit, modifiers joined with +, or a platform key code
    pub key: String,
}

#[derive(Args, Debug)]
pub struct SwipeArgs {
    /// DIRECTION [DISTANCE] (up, down, left or right, then points), or X1 Y1 X2 Y2
    #[arg(required = true, num_args = 1..=4, value_name = "ARGS")]
    pub args: Vec<String>,
    /// How long the stroke takes, in ms
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
    /// Longest wait for a selector's element, in ms
    #[arg(long, value_name = "MS")]
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
    /// What to wait for: visible (the default), hidden, detached or enabled
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
    /// Longest wait, in ms
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
    Text(ElementArg),
    /// A field's value
    Value(ElementArg),
    /// An element's bounds in device points: x, y, width and height
    Box(ElementArg),
}

#[derive(Args, Debug)]
pub struct ElementArg {
    /// A ref or a selector
    pub target: String,
    /// Longest wait for a selector's element, in ms
    #[arg(long, value_name = "MS")]
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
