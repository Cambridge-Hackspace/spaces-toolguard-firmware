//! A ToolGuard panel's display and the glue over the protocol core, with no
//! hardware in it.
//!
//! Two halves, both pure:
//!
//! - [`reader`] wraps `spaces_device::Module` with the things a member
//!   interacts with: the panel, the card cooldown, and the one decision this
//!   device makes locally, which is that it is not connected to anything so
//!   there is no point asking. Monotonic milliseconds in, a `Vec<Action>` out;
//!   no sockets, no card, no panel.
//! - [`screen`] is what the LCD shows, as a function of state. The panel is a
//!   1602 and sixteen columns is the entire budget, so "does this outcome fit"
//!   is a question answered here and tested on a laptop rather than learned
//!   from a board.
//!
//! **The protocol is not reimplemented here.** It was, once, because
//! `spaces_device::Module` assumed a device with a relay and ended any session
//! that got no lease — which a reader never gets. Since `spaces-device` v0.2.0
//! a module serves any role combination, so the rules that decide whether a
//! machine energizes live in exactly one place and this crate renders them.
//!
//! What the box *is* comes from `spaces_device::Config`: whether it reads
//! cards, switches power, senses the machine, or any combination. This crate
//! adds the display to whichever of those is configured.

pub mod button;
pub mod reader;
pub mod screen;

pub use button::StopButton;
pub use reader::{Action, Reader};
pub use screen::{Health, Screen};

// Re-exported so a firmware configures its device without naming two crates:
// the roles are the configuration, and `Config` is where they live.
pub use spaces_device::{Activity, Config, EndReason, Outcome, Output};
