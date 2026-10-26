//! What a member sees, over a module that decides what happens.
//!
//! Same convention as `spaces_device::module`: every entry point takes the
//! current monotonic milliseconds, and every effect leaves as an [`Action`] in
//! a `Vec`. Nothing here opens a socket, reads a card or writes a panel, which
//! is why a lease expiring, a reply arriving late, and a neighbour's verdict
//! landing on this device are all testable without a board.
//!
//! **This used to reimplement the protocol, and the reason it did is gone.**
//! `spaces_device::Module` was a class-1 *power* module: lease-gated,
//! relay-driving, and it ended any session that got no lease within five
//! seconds. A reader is never bound in the `power` role, so it is never leased,
//! so building on `Module` landed every valid badge in
//! `EndReason::NoFirstLease` and the member watched ACCESS GRANTED turn into a
//! dead tool. The honest response was a second state machine; it was also two
//! copies of the rules that decide whether a machine energizes.
//!
//! `spaces-device` v0.2.0 separated a *session* from an *energized output*, so
//! the duplication is no longer the price of being right. `Module` now serves a
//! device in any role combination, and this type is what it always should have
//! been: the panel, the card cooldown, and the one decision a reader can make
//! locally — that it is not connected to anything, so there is no point asking.
//!
//! **What belongs here and what does not.** Anything a member reads is here;
//! anything about whether a machine may run is [`Module`]'s. Keeping the
//! outcome types (`spaces_device::Outcome`) between them is what stops a panel
//! from branching on log prose.

use spaces_device::{Action as DeviceAction, Activity, Config, Module, Outcome, Output};

use crate::screen::{self, Health, Screen};

/// An effect for the shell to carry out.
///
/// The union of what a module decides and what a panel needs. `Set` is here
/// because this firmware is configurable: the same box may be a reader with no
/// relay or a reader that also switches the tool, and the shell wires up only
/// the outputs its hardware has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Publish `payload` on `topic`.
    Publish {
        topic: &'static str,
        payload: String,
    },
    /// Drive an output. Only ever emitted for hardware the config claims.
    Set { output: Output, on: bool },
    /// Put this on the panel until something replaces it.
    Show(Screen),
    /// For the log, and for a human reading it later. Never for the panel: the
    /// panel has sixteen columns and these do not fit.
    Note(String),
}

/// How long an outcome stays on the panel before it returns to the resting
/// screen.
pub const SHOW_MS: u64 = 4_000;

/// Ignore the same card presented again inside this window.
///
/// The RC522 reports a card for as long as it is in the field, so a person
/// holding theirs against the reader is one tap and not thirty requests. The
/// driver halts the card as well; this is the second guard, because a tap that
/// bounces re-enters the field and is a genuinely new detection.
///
/// **It bounds a re-tap stop, too.** Where `stop_on_second_tap` is set, the
/// member's own card ends their session — so without a cooldown a single tap
/// held half a second would open a session and immediately close it.
pub const SAME_CARD_COOLDOWN_MS: u64 = 2_000;

/// A ToolGuard panel: a reader, a display, and whatever else the box has.
pub struct Reader {
    module: Module,
    health: Health,
    /// The panel's current content, so the shell is only told to redraw when it
    /// actually changes. A 1602 over a PCF8574 is slow — three I2C writes per
    /// nibble — and redrawing an unchanged screen every poll would be the whole
    /// I2C budget spent saying nothing.
    shown: Option<Screen>,
    /// While an outcome is on the panel, when it stops being.
    showing_until: Option<u64>,
    last_card: Option<(String, u64)>,
}

impl Reader {
    pub fn new(config: Config) -> Self {
        Reader {
            module: Module::new(config),
            health: Health::default(),
            shown: None,
            showing_until: None,
            last_card: None,
        }
    }

    pub fn tool_id(&self) -> &str {
        &self.module.config().tool_id
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

    /// What the module is doing, for a shell that drives a status light.
    pub fn activity(&self) -> Activity {
        self.module.activity()
    }

    /// Put the outputs in a known state and send the first heartbeat.
    pub fn start(&mut self, now: u64) -> Vec<Action> {
        let actions = self.module.start(now);
        let mut out = self.carry(actions, now);
        self.refresh(now, &mut out);
        out
    }

    /// Tell the reader what the shell knows about the network.
    pub fn health(&mut self, health: Health, now: u64) -> Vec<Action> {
        self.health = health;
        let mut out = Vec::new();
        self.refresh(now, &mut out);
        out
    }

    /// A card was read.
    pub fn card(&mut self, uid: &str, now: u64) -> Vec<Action> {
        // A card held against the reader, or a tap that bounced. Answered
        // before anything else so that neither a request nor a sign-off can be
        // produced by one finger.
        if let Some((last, at)) = &self.last_card {
            if last == uid && now.saturating_sub(*at) < SAME_CARD_COOLDOWN_MS {
                self.last_card = Some((uid.to_string(), now));
                return self.tick(now);
            }
        }
        self.last_card = Some((uid.to_string(), now));

        // NOT READY IS ANSWERED HERE AND NOT SENT. Publishing into a broker the
        // device is not connected to would be silently dropped and the member
        // would watch a reader do nothing at all; the diagnostic screen says
        // which half is missing instead. This is the only decision this type
        // makes about a card -- everything else is the module's.
        if !self.health.ready() {
            let mut out = Vec::new();
            out.push(Action::Note(format!(
                "card {uid} not sent: wifi={} broker={}",
                self.health.wifi, self.health.broker
            )));
            self.hold(screen::diagnostics(&self.health), now, &mut out);
            return out;
        }

        let actions = self.module.swipe(uid, now);
        let mut out = self.carry(actions, now);
        self.refresh(now, &mut out);
        out
    }

    /// The machine started or stopped working, as the hardware sees it.
    pub fn running(&mut self, on: bool, now: u64) -> Vec<Action> {
        let actions = self.module.running(on, now);
        let mut out = self.carry(actions, now);
        self.refresh(now, &mut out);
        out
    }

    /// A stop button was pressed.
    pub fn tool_off(&mut self, now: u64) -> Vec<Action> {
        let actions = self.module.tool_off(now);
        let mut out = self.carry(actions, now);
        self.refresh(now, &mut out);
        out
    }

    /// A message arrived on a subscribed topic.
    pub fn message(&mut self, topic_name: &str, payload: &[u8], now: u64) -> Vec<Action> {
        let actions = self.module.message(topic_name, payload, now);
        let mut out = self.carry(actions, now);
        self.refresh(now, &mut out);
        out
    }

    /// Time passed. Call often: a lease runs out only when something notices.
    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        let actions = self.module.tick(now);
        let mut out = self.carry(actions, now);
        self.refresh(now, &mut out);
        out
    }

    /// Translate the module's actions, turning its outcomes into screens.
    fn carry(&mut self, actions: Vec<DeviceAction>, now: u64) -> Vec<Action> {
        let mut out = Vec::new();
        for action in actions {
            match action {
                DeviceAction::Publish { topic, payload } => {
                    out.push(Action::Publish { topic, payload })
                }
                DeviceAction::Set { output, on } => out.push(Action::Set { output, on }),
                DeviceAction::Note(text) => out.push(Action::Note(text)),
                DeviceAction::Outcome(outcome) => {
                    let screen = match &outcome {
                        // A door box's yes is the latch releasing; a member
                        // told ACCESS GRANTED would wait to be let in (CS:S
                        // #182).
                        Outcome::Authorized if self.is_door_box() => screen::door_open(),
                        Outcome::Authorized => screen::granted(),
                        // The edge's own words, truncated to the panel rather
                        // than matched on. `FIRMWARE.md`: branch on the
                        // boolean, display the string.
                        Outcome::Refused(why) => screen::denied(why),
                        Outcome::NoAnswer => screen::no_answer(),
                        // The pulse ending is not a session ending that a
                        // member needs told about: the panel simply rests.
                        Outcome::Ended(spaces_device::EndReason::StrikePulsed) => {
                            out.push(Action::Note(format!("panel: {outcome:?}")));
                            self.showing_until = None;
                            continue;
                        }
                        Outcome::Ended(_) => screen::ended(),
                    };
                    out.push(Action::Note(format!("panel: {outcome:?}")));
                    self.hold(screen, now, &mut out);
                }
            }
        }
        out
    }

    /// A box whose session is the latch pulse: it drives a latch and switches
    /// no power (CS:S #182). A box that does both is a room in one box and
    /// keeps the tool's screens.
    fn is_door_box(&self) -> bool {
        let c = self.module.config();
        c.drives_latch && !c.gates_power
    }

    /// Show a screen and keep it there for [`SHOW_MS`], over whatever the
    /// resting screen would otherwise be.
    fn hold(&mut self, screen: Screen, now: u64, out: &mut Vec<Action>) {
        self.showing_until = Some(now.saturating_add(SHOW_MS));
        self.show(screen, out);
    }

    /// Retire an expired outcome, then put the resting screen up.
    ///
    /// The resting screen follows the module: waiting for a card, waiting for
    /// an answer, or in a session. Health only reaches the panel when nothing
    /// else is on it, so news about the network cannot wipe a verdict the
    /// member is still reading.
    fn refresh(&mut self, now: u64, out: &mut Vec<Action>) {
        if let Some(until) = self.showing_until {
            if now < until {
                return;
            }
            self.showing_until = None;
        }
        let screen = match self.module.activity() {
            Activity::Asking => screen::asking(),
            // While the latch is released there is nothing to sign into; the
            // panel keeps saying what is true (CS:S #182).
            Activity::Session { .. } if self.is_door_box() => screen::door_open(),
            Activity::Session { energized } => screen::in_session(
                energized,
                self.module.config().stop_on_second_tap,
                self.module.usage(now),
            ),
            Activity::Idle => {
                if self.health.ready() {
                    screen::idle(&self.health)
                } else {
                    screen::diagnostics(&self.health)
                }
            }
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
