//! The capability set an agent advertises in its hello.

wire_flags! {
    /// What an agent implements, advertised in its hello as a `u32` bitmask.
    /// The host refuses a request whose method needs a capability the agent
    /// did not advertise ([`crate::message::Method::required_capability`]).
    pub struct Capabilities;
    /// Tree reads: `snapshot`.
    TREE = 0, "tree";
    /// Input injection: `tap`, `swipe`, `key`.
    INPUT = 1, "input";
    /// Screenshots: `screenshot`.
    SCREENSHOT = 2, "screenshot";
    /// Frame streaming: `frames_start`, `frames_stop`.
    FRAMES = 3, "frames";
    /// Change events on the tree, window and lifecycle streams, and `resync`.
    EVENTS = 4, "events";
    /// Direct text setting: `text_set`.
    TEXT_SET = 5, "text_set";
}

impl Capabilities {
    /// The empty set.
    pub const NONE: Capabilities = Capabilities::EMPTY;

    /// A human description of the capabilities in `self`, such as
    /// `frame streaming`, for error messages.
    pub fn label(self) -> String {
        const LABELS: [(Capabilities, &str); 6] = [
            (Capabilities::TREE, "tree reads"),
            (Capabilities::INPUT, "input injection"),
            (Capabilities::SCREENSHOT, "screenshots"),
            (Capabilities::FRAMES, "frame streaming"),
            (Capabilities::EVENTS, "change events"),
            (Capabilities::TEXT_SET, "direct text setting"),
        ];
        let labels: Vec<&str> = LABELS
            .iter()
            .filter(|(cap, _)| self.contains(*cap))
            .map(|(_, label)| *label)
            .collect();
        if labels.is_empty() {
            self.to_string()
        } else {
            labels.join(" and ")
        }
    }
}
