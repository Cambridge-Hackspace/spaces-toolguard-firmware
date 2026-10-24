//! The scan state machine, with no hardware and no clock inside it.
//!
//! Same convention as `spaces_device::module`: every entry point takes the
//! current monotonic milliseconds, and every effect leaves as an [`Action`] in
//! a `Vec`. Nothing here opens a socket, reads a card or writes a panel, which
//! is why a lease expiring, a reply arriving late, and a neighbour's verdict
//! landing on this device are all testable without a board.
//!
//! **Why not `spaces_device::Module`.** That type is a class-1 *power* module:
//! it is lease-gated, drives a relay, meters a session and heartbeats a power
//! report. A reader with no relay is never bound in the `power` role, so it is
//! never leased (`spaces-firmware` docs/FINDINGS.md finding 7 and the
//! `note_device_seen` ingest), and building on `Module` would land every valid
//! badge in `EndReason::NoFirstLease` five seconds later. The protocol pieces
//! are shared instead: the topics, [`ToolRequest`] and [`ToolOnReply`] all come
//! from `spaces-device`.
//!
//! **What a reader is for.** It asks, and it tells the person what the answer
//! was. Energizing is somebody else's binding — a `power` module on the same
//! tool — and this device deliberately has no `Output`.

use serde_json::json;
use spaces_device::wire::{topic, ToolOnReply, ToolRequest};

use crate::screen::{self, Health, Screen};

/// An effect for the shell to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Publish `payload` on `topic`.
    Publish {
        topic: &'static str,
        payload: String,
    },
    /// Put this on the panel until something replaces it.
    Show(Screen),
    /// For the log, and for a human reading it later. Never for the panel: the
    /// panel has sixteen columns and these do not fit.
    Note(String),
}

/// How long to wait for the edge before calling it a no.
///
/// `FIRMWARE.md`: *"If nothing answers, fail closed; a timeout is a no."*
pub const REPLY_TIMEOUT_MS: u64 = 5_000;

/// How long an outcome stays on the panel before it returns to idle.
pub const SHOW_MS: u64 = 4_000;

/// Ignore the same card presented again inside this window.
///
/// The RC522 reports a card for as long as it is in the field, so a person
/// holding theirs against the reader is one tap and not thirty requests. The
/// driver halts the card as well; this is the second guard, because a tap that
/// bounces re-enters the field and is a genuinely new detection.
pub const SAME_CARD_COOLDOWN_MS: u64 = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Waiting for a card.
    Idle,
    /// Asked the edge, waiting for its answer.
    Asking { card: String, since: u64 },
    /// Showing an outcome, which replaces itself with idle at `until`.
    Showing { until: u64 },
}

/// A badge reader.
pub struct Reader {
    tool_id: String,
    phase: Phase,
    health: Health,
    last_card: Option<(String, u64)>,
    /// The panel's current content, so the shell is only told to redraw when it
    /// actually changes. A 1602 over a PCF8574 is slow — three I2C writes per
    /// nibble — and redrawing an unchanged screen every poll would be the whole
    /// I2C budget spent saying nothing.
    shown: Option<Screen>,
}

impl Reader {
    pub fn new(tool_id: impl Into<String>) -> Self {
        Reader {
            tool_id: tool_id.into(),
            phase: Phase::Idle,
            health: Health::default(),
            last_card: None,
            shown: None,
        }
    }

    pub fn tool_id(&self) -> &str {
        &self.tool_id
    }

    /// True when a card would be acted on rather than refused locally.
    pub fn ready(&self) -> bool {
        self.health.ready()
    }

    /// What is on the panel right now, if anything has been put there.
    ///
    /// Needed because [`Action::Show`] is emitted only on CHANGE, so the action
    /// list says what the panel just became and not what it currently is. A
    /// test asserting "the member is looking at the diagnostic screen" has to
    /// read state; so does a shell that wants to repaint after an I2C glitch
    /// without waiting for the content to happen to change.
    pub fn screen(&self) -> Option<&Screen> {
        self.shown.as_ref()
    }

    /// Tell the reader what the shell knows about the network.
    pub fn health(&mut self, health: Health, now: u64) -> Vec<Action> {
        let mut out = Vec::new();
        self.health = health;
        self.redraw(now, &mut out);
        out
    }

    /// A card was read.
    pub fn card(&mut self, uid: &str, now: u64) -> Vec<Action> {
        let mut out = Vec::new();
        self.advance(now, &mut out);

        // A card held against the reader, or a tap that bounced.
        if let Some((last, at)) = &self.last_card {
            if last == uid && now.saturating_sub(*at) < SAME_CARD_COOLDOWN_MS {
                self.last_card = Some((uid.to_string(), now));
                return out;
            }
        }
        self.last_card = Some((uid.to_string(), now));

        // NOT READY IS ANSWERED HERE AND NOT SENT. Publishing into a broker the
        // device is not connected to would be silently dropped and the member
        // would watch a reader do nothing at all; the diagnostic screen says
        // which half is missing instead.
        if !self.health.ready() {
            out.push(Action::Note(format!(
                "card {uid} not sent: wifi={} broker={}",
                self.health.wifi, self.health.broker
            )));
            self.phase = Phase::Showing {
                until: now + SHOW_MS,
            };
            self.show(screen::diagnostics(&self.health), &mut out);
            return out;
        }

        match &self.phase {
            // A second card while one answer is outstanding. Dropped rather
            // than queued: the reply carries no card, so two in flight cannot
            // be told apart, and the second would take the first's answer.
            Phase::Asking { card, .. } => {
                out.push(Action::Note(format!(
                    "card {uid} ignored: still waiting on an answer about {card}"
                )));
            }
            Phase::Idle | Phase::Showing { .. } => {
                out.push(Action::Note(format!("card {uid} presented")));
                out.push(Action::Publish {
                    topic: topic::TOOL_ON_REQUEST,
                    payload: json!(ToolRequest {
                        card: uid.to_string(),
                        tool_id: self.tool_id.clone(),
                    })
                    .to_string(),
                });
                self.phase = Phase::Asking {
                    card: uid.to_string(),
                    since: now,
                };
                self.show(screen::asking(), &mut out);
            }
        }
        out
    }

    /// A message arrived on a subscribed topic.
    pub fn message(&mut self, topic_name: &str, payload: &[u8], now: u64) -> Vec<Action> {
        let mut out = Vec::new();
        self.advance(now, &mut out);
        if topic_name == topic::TOOL_ON_RESPONSE {
            self.on_reply(payload, now, &mut out);
        }
        // Every other subscribed topic is somebody else's: a lease is for the
        // power module, and the acks on tool-off and tool-log answer requests
        // this device never sends. Silence is correct, not an omission.
        out
    }

    /// Time passed.
    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        let mut out = Vec::new();
        self.advance(now, &mut out);
        out
    }

    fn on_reply(&mut self, payload: &[u8], now: u64, out: &mut Vec<Action>) {
        let reply: Option<ToolOnReply> = serde_json::from_slice(payload).ok();

        // IS IT MINE BEFORE IS IT YES (CS:S #166). Every module on this broker
        // receives every answer, so a reply about another tool is dropped
        // without touching the phase: ours may still be in flight, and treating
        // a neighbour's reply as our refusal would deny this member for
        // somebody else's swipe. If ours never comes, the timeout is the no.
        if let Some(r) = &reply {
            if !r.is_for(&self.tool_id) {
                out.push(Action::Note(format!(
                    "ignoring an answer about {} -- this reader asked about {}",
                    r.tool_id.as_deref().unwrap_or("?"),
                    self.tool_id
                )));
                return;
            }
        }

        let Phase::Asking { card, .. } = &self.phase else {
            // An answer with nothing outstanding: a late reply to a request
            // this reader already timed out, or one meant for a neighbour on an
            // edge too old to echo a tool. Logged, never shown -- the member
            // standing here did not ask.
            out.push(Action::Note(
                "an answer arrived with nothing outstanding; ignored".to_string(),
            ));
            return;
        };
        let card = card.clone();

        match reply {
            Some(r) if r.is_yes() => {
                out.push(Action::Note(format!("card {card} authorized")));
                self.phase = Phase::Showing {
                    until: now + SHOW_MS,
                };
                self.show(screen::granted(), out);
            }
            Some(r) => {
                let why = r.explanation().to_string();
                out.push(Action::Note(format!("card {card} refused: {why}")));
                self.phase = Phase::Showing {
                    until: now + SHOW_MS,
                };
                self.show(screen::denied(&why), out);
            }
            None => {
                // Unparseable is a refusal, not an absence: something answered,
                // and absence of a yes is never permission.
                out.push(Action::Note(format!(
                    "card {card} refused: the answer did not parse"
                )));
                self.phase = Phase::Showing {
                    until: now + SHOW_MS,
                };
                self.show(screen::denied("bad reply"), out);
            }
        }
    }

    /// Move the clock forward: time out a request, retire a shown outcome.
    fn advance(&mut self, now: u64, out: &mut Vec<Action>) {
        match &self.phase {
            Phase::Asking { card, since } if now.saturating_sub(*since) >= REPLY_TIMEOUT_MS => {
                out.push(Action::Note(format!(
                    "card {card}: nothing answered in {REPLY_TIMEOUT_MS} ms, and a timeout is a no"
                )));
                self.phase = Phase::Showing {
                    until: now + SHOW_MS,
                };
                self.show(screen::no_answer(), out);
            }
            Phase::Showing { until } if now >= *until => {
                self.phase = Phase::Idle;
                self.redraw(now, out);
            }
            _ => {}
        }
    }

    /// Put the resting screen up: idle when ready, diagnostics when not.
    fn redraw(&mut self, now: u64, out: &mut Vec<Action>) {
        if let Phase::Showing { .. } | Phase::Asking { .. } = self.phase {
            // An outcome or a question is on the panel and has not expired;
            // health news does not get to overwrite it mid-read.
            let _ = now;
            return;
        }
        let screen = if self.health.ready() {
            screen::idle(&self.health)
        } else {
            screen::diagnostics(&self.health)
        };
        self.show(screen, out);
    }

    /// Emit a redraw only when the content actually changes.
    fn show(&mut self, screen: Screen, out: &mut Vec<Action>) {
        if self.shown.as_ref() == Some(&screen) {
            return;
        }
        self.shown = Some(screen.clone());
        out.push(Action::Show(screen));
    }
}
