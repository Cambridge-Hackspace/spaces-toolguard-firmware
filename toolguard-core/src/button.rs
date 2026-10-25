//! A stop button: anyone may press it, and it ends the session.
//!
//! The protocol half already exists -- [`spaces_device::Module::tool_off`], "the
//! person pressed stop" -- and CS:S's `tool-off` is open to any device bound to
//! the tool, so a stop needs no card: anyone may cut the power, by design. What
//! a physical button adds is contact bounce and a finger that stays down, and
//! this is the piece that turns those into exactly one stop per press.
//!
//! Pure, like [`spaces_device::RunFilter`]: fed a reading and a clock, it says
//! whether a press just completed. The shell samples the pin every pass.

/// Turns a bouncy, held contact into one event per press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopButton {
    debounce_ms: u64,
    /// When the contact was first seen closed, if it is closed now.
    closed_since: Option<u64>,
    /// Whether this press has already fired, so a held button is one stop.
    fired: bool,
}

impl StopButton {
    /// `debounce_ms` is how long the contact must stay closed before a press
    /// counts. Tens of milliseconds covers a mechanical switch; a longer value
    /// makes it a deliberate hold.
    pub fn new(debounce_ms: u64) -> Self {
        StopButton {
            debounce_ms,
            closed_since: None,
            fired: false,
        }
    }

    /// Feed the current reading. Returns `true` exactly once per press: on the
    /// first sample at which the contact has been closed for `debounce_ms`.
    /// Releasing re-arms it; a glitch shorter than the debounce never fires.
    pub fn sample(&mut self, closed: bool, now: u64) -> bool {
        if !closed {
            self.closed_since = None;
            self.fired = false;
            return false;
        }
        let since = *self.closed_since.get_or_insert(now);
        if self.fired || now.saturating_sub(since) < self.debounce_ms {
            return false;
        }
        self.fired = true;
        true
    }
}
