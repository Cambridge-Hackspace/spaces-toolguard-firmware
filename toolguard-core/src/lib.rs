//! A ToolGuard badge reader's decisions and its display, with no hardware in it.
//!
//! Two halves, both pure:
//!
//! - [`reader`] is the scan state machine. Monotonic milliseconds in, a
//!   `Vec<Action>` out; no sockets, no card, no panel.
//! - [`screen`] is what the LCD shows, as a function of state. The panel is a
//!   1602 and sixteen columns is the entire budget, so "does this outcome fit"
//!   is a question answered here and tested on a laptop rather than learned
//!   from a board.
//!
//! The protocol itself is not redefined: the topics, `ToolRequest` and
//! `ToolOnReply` come from `spaces-device`, so this crate and the power modules
//! speak the same wire by construction rather than by agreement.

pub mod reader;
pub mod screen;

pub use reader::{Action, Reader};
pub use screen::{Health, Screen};
