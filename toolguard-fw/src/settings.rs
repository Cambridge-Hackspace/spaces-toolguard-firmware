//! What this box is, as an administrator sets it.
//!
//! The roles are independent — read cards, switch power, sense the machine — and
//! a ToolGuard may fill any combination. None of that is a build-time choice:
//! one image is flashed and the same board becomes a reader beside a laser, a
//! reader that also switches it, or a plug with no reader at all.
//!
//! **Every field here is `required: false`, and that is load-bearing.**
//! `Store::missing` puts a device into the setup portal and keeps it there
//! until every required field is filled, so adding a required field to this
//! firmware would drop every already-deployed board into its captive portal on
//! the next boot — a fleet of readers waiting for somebody with a phone. A new
//! setting must therefore have a sane default in code, which is also the right
//! answer for an upgrade: a box that was working keeps working.
//!
//! **Booleans are typed as words** because the setup page has only text and
//! password inputs. The spellings a person actually uses are accepted rather
//! than one blessed form, and anything unrecognised reads as the default rather
//! than as `false` — a typo must not silently disable a safety-relevant output.
//!
//! The timings are seconds where a person would say seconds, and milliseconds
//! only for the glitch filter, where a person would not.

use spaces_device::Config;
use spaces_device_espidf::store::{Field, Store};

/// NVS keys. **At most fifteen characters each**, which is the NVS limit and
/// the reason some of these are abbreviated.
pub mod key {
    pub const READS_CARDS: &str = "reads_cards";
    pub const GATES_POWER: &str = "gates_power";
    pub const SENSES_RUN: &str = "senses_run";
    pub const SENSE_LOW: &str = "sense_low";
    pub const USE_HOLD_S: &str = "use_hold_s";
    pub const USE_ASSERT: &str = "use_asrt_ms";
    pub const STOP_2TAP: &str = "stop_2tap";
    pub const IDLE_END_S: &str = "idle_end_s";
    pub const MAX_SESS_S: &str = "max_sess_s";
    pub const USAGE_INT_S: &str = "usage_int_s";
}

/// The settings this firmware adds to the shared set, in the order they are
/// rendered.
pub const FIELDS: &[Field] = &[
    Field {
        key: key::READS_CARDS,
        label: "Has a card reader",
        hint: "yes (default) if an RC522 is fitted. no makes this a plug: it switches the \
               tool on the edge's lease and never reads a badge",
        secret: false,
        required: false,
    },
    Field {
        key: key::GATES_POWER,
        label: "Switches the tool",
        hint: "no (default) if this box only reads badges and shows answers. yes if it \
               drives a relay -- it must then be bound to the tool in the power role, or \
               the edge will never lease it",
        secret: false,
        required: false,
    },
    Field {
        key: key::SENSES_RUN,
        label: "Senses the machine",
        hint: "no (default). yes if an input is wired to something that says the machine \
               is working. Leave it no when nothing is: a device that reports zero working \
               time is believed, and on a tool billed by working time that bills nothing",
        secret: false,
        required: false,
    },
    Field {
        key: key::SENSE_LOW,
        label: "Sense is active low",
        hint: "yes (default) -- a Ruida laser's L-ON1 pulls low to enable, and most dry \
               contacts close to ground. Check the machine: on a Ruida this is the \
               'Laser Output Signal' vendor setting and it can be either way round",
        secret: false,
        required: false,
    },
    Field {
        key: key::USE_HOLD_S,
        label: "Bridge gaps (seconds)",
        hint: "10 (default). How long the machine may pause and still count as working. A \
               laser's enable line drops on every scan-line return, so 0 here meters one \
               engrave as hundreds of fragments. 0 measures beam-on time instead",
        secret: false,
        required: false,
    },
    Field {
        key: key::USE_ASSERT,
        label: "Ignore blips under (ms)",
        hint: "100 (default). The signal must hold this long before it counts. Rejects \
               noise, which matters next to a laser's HV supply",
        secret: false,
        required: false,
    },
    Field {
        key: key::STOP_2TAP,
        label: "Tap again to finish",
        hint: "yes (default) when this box does not switch the tool. The member's own card \
               ends their session -- on a box with no relay and no sensor it is the only \
               stop there is, and without it the tool stays in use after they leave",
        secret: false,
        required: false,
    },
    Field {
        key: key::IDLE_END_S,
        label: "Finish after idle (s)",
        hint: "0 (default, never). End the session when the machine has not worked for this \
               long. Needs a sensor; ignored without one",
        secret: false,
        required: false,
    },
    Field {
        key: key::MAX_SESS_S,
        label: "Longest session (s)",
        hint: "0 (default, none). A backstop for a member who walks away without tapping \
               off. Worth setting on any box that does not switch the tool",
        secret: false,
        required: false,
    },
    Field {
        key: key::USAGE_INT_S,
        label: "Report usage every (s)",
        hint: "30 (default). Usage is sent in increments, so a reboot mid-session costs one \
               interval rather than the whole session. 0 reports once at the end",
        secret: false,
        required: false,
    },
];

/// How this box is configured, read once at boot.
pub struct Settings {
    pub config: Config,
    /// Whether the sensed input reads the machine as working when LOW.
    ///
    /// Applied here, in the shell, and nowhere else. `Module::running` takes a
    /// *semantic* bool, so exactly one inversion happens in the whole firmware
    /// and it is this one.
    pub sense_active_low: bool,
}

impl Settings {
    /// Read the settings, falling back to a plain badge reader.
    ///
    /// The ids are left empty for the caller to fill: [`Self::roles`] is needed
    /// *before* registration, because it is what the device registers as, and
    /// `device_id` does not exist until registration has happened. Reading the
    /// store twice to break that circle would mean two places that decide what
    /// this box is.
    pub fn read(store: &Store) -> Self {
        let reads_cards = flag(store, key::READS_CARDS, true);
        let gates_power = flag(store, key::GATES_POWER, false);
        let senses_running = flag(store, key::SENSES_RUN, false);

        let mut config = Config::new("", "");
        config.reads_cards = reads_cards;
        config.gates_power = gates_power;
        config.senses_running = senses_running;
        config.use_hold_ms = secs_ms(store, key::USE_HOLD_S, 10) as u32;
        config.use_assert_ms = number(store, key::USE_ASSERT, 100) as u32;
        config.idle_end_ms = secs_ms(store, key::IDLE_END_S, 0);
        config.max_session_ms = secs_ms(store, key::MAX_SESS_S, 0);
        config.usage_interval_ms = secs_ms(store, key::USAGE_INT_S, 30);
        // Defaults to on for a box that cannot stop a session any other way,
        // and off for one that can: where a lease lapses or a relay opens, a
        // re-tap meaning "done" would surprise somebody who simply swiped
        // twice.
        config.stop_on_second_tap = flag(store, key::STOP_2TAP, !gates_power);

        Settings {
            config,
            sense_active_low: flag(store, key::SENSE_LOW, true),
        }
    }

    /// The roles to register with, derived from the hardware rather than
    /// typed in twice.
    ///
    /// A device declares what it can do and an administrator binds it to a tool
    /// in a role it declared. Claiming `power` here when nothing is wired to a
    /// relay is how a reader with no relay becomes the thing a tool's safety
    /// rests on, and the mistake surfaces at the far end of the system as a
    /// machine that never comes on.
    pub fn roles(&self) -> Vec<&'static str> {
        let mut roles = Vec::new();
        if self.config.reads_cards {
            roles.push("reader");
        }
        if self.config.gates_power {
            roles.push("power");
        }
        if self.config.senses_running {
            roles.push("sensor");
        }
        roles
    }

    /// One line for the log, so a boot record says what this box thinks it is.
    ///
    /// Worth the line: almost every way this firmware can be misconfigured
    /// shows up as a tool that quietly never works, and the first question is
    /// always what the box believes about itself.
    pub fn describe(&self) -> String {
        format!(
            "reads_cards={} gates_power={} senses_running={} (active {}) \
             hold={}ms assert={}ms stop_2tap={} idle_end={}ms max_session={}ms usage_every={}ms",
            self.config.reads_cards,
            self.config.gates_power,
            self.config.senses_running,
            if self.sense_active_low { "low" } else { "high" },
            self.config.use_hold_ms,
            self.config.use_assert_ms,
            self.config.stop_on_second_tap,
            self.config.idle_end_ms,
            self.config.max_session_ms,
            self.config.usage_interval_ms,
        )
    }
}

/// A yes/no setting. Anything unrecognised, including empty, is `default`.
fn flag(store: &Store, key: &str, default: bool) -> bool {
    match store.get(key) {
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "yes" | "y" | "true" | "t" | "1" | "on" => true,
            "no" | "n" | "false" | "f" | "0" | "off" => false,
            // Not `false`: a typo must not quietly turn an output off, or on.
            other => {
                if !other.is_empty() {
                    log::warn!("{key}: {other:?} is not yes or no; using {default}");
                }
                default
            }
        },
        None => default,
    }
}

/// A whole number, or `default` if it is absent or not one.
fn number(store: &Store, key: &str, default: u64) -> u64 {
    match store.get(key) {
        Some(v) if !v.trim().is_empty() => match v.trim().parse::<u64>() {
            Ok(n) => n,
            Err(_) => {
                log::warn!("{key}: {:?} is not a number; using {default}", v.trim());
                default
            }
        },
        _ => default,
    }
}

/// A number of seconds, as milliseconds.
fn secs_ms(store: &Store, key: &str, default_secs: u64) -> u64 {
    number(store, key, default_secs).saturating_mul(1_000)
}
