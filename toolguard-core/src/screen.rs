//! What the LCD shows, as a pure function of state.
//!
//! The panel is a 1602: **two lines of sixteen columns**, and that is the whole
//! budget. Sixteen columns is less than it sounds — `Tool not authorized for
//! this user` is 38 — so every outcome has to be cut to fit, and cutting is a
//! decision about what a person standing at a machine needs to read.
//!
//! Keeping it here, with no `I2cDriver` in sight, is what makes that decision
//! testable: whether "ACCESS DENIED" fits, whether a 38-character refusal is
//! truncated at a word or mid-word, and whether a diagnostic screen still shows
//! the broker state when the Wi-Fi line is long, are all assertions that run on
//! a laptop. The alternative is learning it from a board, one reflash at a time,
//! and there is no board yet.
//!
//! **Truncation is deliberate and lossy.** The full `reason` goes to the log,
//! which is readable over the network at `/log`; the panel gets as much as fits.
//! A reader who needs the exact string has somewhere to get it.

/// The panel's shape. Not configurable: the hardware is a 1602.
pub const COLS: usize = 16;
pub const ROWS: usize = 2;

/// Two lines, each already cut to [`COLS`].
///
/// Built only through [`Screen::new`], so a `Screen` that exists is a `Screen`
/// that fits. Every construction path in this module goes through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    lines: [String; ROWS],
}

impl Screen {
    /// Cut both lines to the panel.
    ///
    /// Truncation is by CHARACTER and not by byte: `str::truncate` panics on a
    /// multi-byte boundary, and a reason arriving from the edge is arbitrary
    /// UTF-8 the firmware does not control. An HD44780 cannot render most of it
    /// anyway — that is the shell's problem, and it gets a string of the right
    /// LENGTH here rather than a string of the right byte count.
    pub fn new(top: &str, bottom: &str) -> Self {
        Screen {
            lines: [fit(top), fit(bottom)],
        }
    }

    pub fn top(&self) -> &str {
        &self.lines[0]
    }

    pub fn bottom(&self) -> &str {
        &self.lines[1]
    }

    /// Both lines, for a driver that writes the panel row by row.
    pub fn rows(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(|s| s.as_str())
    }
}

/// One line, cut to the panel's width in CHARACTERS.
fn fit(s: &str) -> String {
    s.chars().take(COLS).collect()
}

/// How the device is doing, for the diagnostic screen.
///
/// This is the screen the request asked for by name: *"diagnostic information
/// if the server connection is down"*. It is a first-class view rather than an
/// error case, because a reader that cannot reach its edge is the situation a
/// person is most likely to be standing in front of, wondering why their card
/// did nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Health {
    /// Joined a network.
    pub wifi: bool,
    /// Connected to the local broker.
    pub broker: bool,
    /// This device's address, when it has one.
    pub address: Option<String>,
    /// Wall clock as `HH:MM`, when SNTP has answered.
    ///
    /// `None` until it does, and **never** consulted by any decision: the
    /// platform's tool path is measured on a monotonic timer by design, and a
    /// clock that is only sometimes right must not reach an authorization.
    pub clock: Option<String>,
}

impl Health {
    /// True when the device can actually ask about a card.
    pub fn ready(&self) -> bool {
        self.wifi && self.broker
    }
}

/// The screen for a device that is ready and waiting for a card.
pub fn idle(health: &Health) -> Screen {
    match &health.clock {
        Some(hhmm) => Screen::new("Present card", hhmm),
        // No clock yet is not an error worth showing a member, so the line is
        // spent on the instruction instead of on an empty field.
        None => Screen::new("Present card", ""),
    }
}

/// The screen while the edge is being asked about a card.
pub fn asking() -> Screen {
    Screen::new("Checking card", "please wait")
}

/// The screen for a yes.
pub fn granted() -> Screen {
    Screen::new("ACCESS GRANTED", "")
}

/// The screen for a no, carrying as much of the edge's reason as fits.
///
/// The verdict and the reason are on separate lines on purpose: the top line is
/// the same sixteen characters every time, so it is recognisable from across a
/// room, and the bottom line is the detail. Branch on the boolean, display the
/// string — the reason text is not part of the contract and will change.
pub fn denied(reason: &str) -> Screen {
    Screen::new("ACCESS DENIED", reason)
}

/// The screen when nothing answered.
///
/// A timeout is a no (`FIRMWARE.md`), and it is a *different* no from a refusal:
/// the member did nothing wrong and retrying may work, so it does not say
/// DENIED.
pub fn no_answer() -> Screen {
    Screen::new("No answer", "try again")
}

/// The diagnostic screen: what is up, what is down, and where this device is.
///
/// Shown when the device is not ready, and reachable deliberately at any time —
/// a reader that looks dead and a reader that is fine but whose edge is gone
/// are the same thing from the outside, and this is what tells them apart.
pub fn diagnostics(health: &Health) -> Screen {
    let top = match (health.wifi, health.broker) {
        (false, _) => "No Wi-Fi",
        (true, false) => "No edge",
        (true, true) => "Online",
    };
    // The address earns the second line over anything else: it is what someone
    // holding a laptop needs in order to reach `/log`, where the full story is.
    let bottom = health.address.as_deref().unwrap_or("no address");
    Screen::new(top, bottom)
}
