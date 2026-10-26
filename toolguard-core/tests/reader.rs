//! What a member reads off the panel, and the one decision this device makes
//! on its own.
//!
//! Time is whatever these tests say it is: every call takes the current
//! monotonic milliseconds, which is what makes a reply timing out testable.
//!
//! There is no board yet, and that is the point rather than an excuse. Every
//! assertion here is about what a person reads off a 1602, and none of it needs
//! an RC522 to be true.
//!
//! **WHAT THIS FILE DELIBERATELY NO LONGER COVERS.** It used to assert the
//! protocol too — that an unparseable answer is a refusal, that a reply naming
//! another tool is dropped before its verdict is read, that a timeout is a no,
//! that `ok` without a yes is not a yes. Those rules now live in exactly one
//! implementation, `spaces_device::Module`, and are pinned by its own tests.
//! Asserting them here as well would be testing a re-export: the duplicate
//! would agree with the original no matter what either did, and the day they
//! disagreed the one that mattered would be the one these tests were not
//! reading. What is left is what this crate actually decides.

use spaces_device::wire::topic;
use spaces_device::{Output, Usage};
use toolguard_core::reader::{Action, Reader, SAME_CARD_COOLDOWN_MS, SHOW_MS};
use toolguard_core::screen::{Health, COLS};
use toolguard_core::Config;

const ME: &str = "dev-uuid-me";
const TOOL: &str = "laser-01";
const ALICE: &str = "04A1B2C3";
const BOB: &str = "04D4E5F6";

/// A reader with no relay, online and idle: the plainest badge reader.
fn reader() -> Reader {
    let mut r = Reader::new(Config::switches_nothing(ME, TOOL));
    r.start(0);
    r.health(online(), 0);
    r
}

/// A reader that also switches the tool, so it is leased.
fn gating_reader() -> Reader {
    let mut r = Reader::new(Config::new(ME, TOOL));
    r.start(0);
    r.health(online(), 0);
    r
}

fn online() -> Health {
    Health {
        wifi: true,
        broker: true,
        address: Some("192.168.1.50".into()),
        clock: None,
    }
}

fn published(actions: &[Action], name: &str) -> Vec<serde_json::Value> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Publish { topic, payload } if *topic == name => {
                Some(serde_json::from_str(payload).expect("published JSON"))
            }
            _ => None,
        })
        .collect()
}

fn publishes_to(actions: &[Action]) -> Vec<&'static str> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::Publish { topic, .. } => Some(*topic),
            _ => None,
        })
        // Both heartbeats are filtered out, so assertions about the protocol's
        // decisions are not drowned by the two messages every tick sends. The
        // heartbeats have their own arm below.
        .filter(|t| *t != topic::POWER_REPORT && *t != topic::ALIVE_REQUEST)
        .collect()
}

/// The last screen an action list puts up, as `(top, bottom)`.
fn screen(actions: &[Action]) -> Option<(String, String)> {
    actions.iter().rev().find_map(|a| match a {
        Action::Show(s) => Some((s.top().to_string(), s.bottom().to_string())),
        _ => None,
    })
}

fn here(r: &Reader) -> (String, String) {
    let s = r.screen().expect("the panel should have content");
    (s.top().to_string(), s.bottom().to_string())
}

fn yes() -> Vec<u8> {
    serde_json::json!({"resource": TOOL, "authorized": true, "reason": "authorized"})
        .to_string()
        .into_bytes()
}

fn no(reason: &str) -> Vec<u8> {
    serde_json::json!({"resource": TOOL, "authorized": false, "reason": reason})
        .to_string()
        .into_bytes()
}

fn lease(ttl_ms: u64) -> Vec<u8> {
    serde_json::json!({"resource": TOOL, "device_id": ME, "grant": true, "ttl_ms": ttl_ms})
        .to_string()
        .into_bytes()
}

// ------------------------------------------- the one local decision: not ready

#[test]
fn a_card_is_not_sent_when_there_is_nothing_to_send_it_to() {
    // Publishing into a broker this device is not connected to is silently
    // dropped, and the member watches a reader do nothing at all. The
    // diagnostic screen says which half is missing instead.
    let mut r = Reader::new(Config::switches_nothing(ME, TOOL));
    r.start(0);
    r.health(
        Health {
            wifi: true,
            broker: false,
            address: Some("192.168.1.50".into()),
            clock: None,
        },
        0,
    );
    let out = r.card(ALICE, 100);
    assert!(
        publishes_to(&out).is_empty(),
        "a card was sent with no broker: {out:?}"
    );
    // Asserted on panel STATE, not on the action list: `health` above already
    // put this screen up and `Show` is emitted only on change, so there is
    // nothing to redraw. The member is looking at the right thing either way,
    // which is the claim.
    assert_eq!(here(&r).0, "No edge");
}

#[test]
fn the_panel_names_which_half_is_missing() {
    let mut r = Reader::new(Config::switches_nothing(ME, TOOL));
    r.start(0);
    for (wifi, broker, top) in [
        (false, false, "No Wi-Fi"),
        (false, true, "No Wi-Fi"),
        (true, false, "No edge"),
    ] {
        r.health(
            Health {
                wifi,
                broker,
                address: Some("10.0.0.9".into()),
                clock: None,
            },
            0,
        );
        assert_eq!(here(&r).0, top, "wifi={wifi} broker={broker}");
    }
}

#[test]
fn the_diagnostic_screen_carries_the_address_to_reach_the_log_at() {
    let mut r = Reader::new(Config::switches_nothing(ME, TOOL));
    r.start(0);
    r.health(
        Health {
            wifi: false,
            broker: false,
            address: Some("192.168.1.50".into()),
            clock: None,
        },
        0,
    );
    assert_eq!(here(&r).1, "192.168.1.50");
}

// ------------------------------------------------------------- the cooldown

#[test]
fn the_same_card_held_against_the_reader_is_one_tap() {
    // The RC522 reports a card for as long as it is in the field.
    let mut r = reader();
    let first = r.card(ALICE, 100);
    assert_eq!(publishes_to(&first), vec![topic::SCAN_REQUEST]);
    for t in [150, 400, 900, 100 + SAME_CARD_COOLDOWN_MS - 1] {
        let again = r.card(ALICE, t);
        assert!(
            publishes_to(&again).is_empty(),
            "a held card asked twice at {t}: {again:?}"
        );
    }
}

/// And the cooldown is what stops one finger from both opening a session and
/// signing off from it, where a re-tap is the stop.
#[test]
fn a_held_card_does_not_open_a_session_and_immediately_close_it() {
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 150);
    let held = r.card(ALICE, 200);
    assert!(
        publishes_to(&held).is_empty(),
        "the same finger signed the member straight back out: {held:?}"
    );
}

#[test]
fn a_different_card_is_never_held_off_by_the_cooldown() {
    let mut r = reader();
    r.card(ALICE, 100);
    let bob = r.card(BOB, 150);
    assert!(
        bob.iter().any(|a| matches!(a, Action::Note(_))),
        "Bob's tap was swallowed by Alice's cooldown: {bob:?}"
    );
}

// ----------------------------------------------------------------- the panel

#[test]
fn the_panel_says_it_is_working_while_it_waits() {
    let mut r = reader();
    let out = r.card(ALICE, 100);
    assert_eq!(
        screen(&out),
        Some(("Checking card".into(), "please wait".into()))
    );
}

#[test]
fn a_yes_shows_granted_and_a_no_shows_the_reason() {
    let mut r = reader();
    r.card(ALICE, 100);
    let out = r.message(topic::SCAN_RESPONSE, &yes(), 200);
    assert_eq!(screen(&out).unwrap().0, "ACCESS GRANTED");

    let mut r = reader();
    r.card(BOB, 100);
    let out = r.message(topic::SCAN_RESPONSE, &no("Unknown card"), 200);
    assert_eq!(
        screen(&out),
        Some(("ACCESS DENIED".into(), "Unknown card".into()))
    );
}

#[test]
fn nothing_answering_does_not_blame_the_member() {
    // A timeout is a no, and a different no from a refusal: the member did
    // nothing wrong and retrying may work, so it does not say DENIED.
    let mut r = reader();
    r.card(ALICE, 100);
    let out = r.tick(100 + 5_000);
    assert_eq!(
        screen(&out),
        Some(("No answer".into(), "try again".into())),
        "{out:?}"
    );
}

#[test]
fn an_outcome_is_replaced_by_the_resting_screen_once_it_has_been_read() {
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &no("Unknown card"), 200);
    assert_eq!(here(&r).0, "ACCESS DENIED");
    r.tick(200 + SHOW_MS - 1);
    assert_eq!(here(&r).0, "ACCESS DENIED", "retired early");
    r.tick(200 + SHOW_MS);
    assert_eq!(here(&r).0, "Present card", "the verdict never retired");
}

#[test]
fn health_news_does_not_wipe_an_outcome_off_the_panel() {
    // Wi-Fi flapping while a member is reading their verdict must not take it
    // away; the panel is the only place the answer appeared.
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &no("Training required"), 200);
    r.health(
        Health {
            wifi: false,
            broker: false,
            address: None,
            clock: None,
        },
        250,
    );
    assert_eq!(here(&r).0, "ACCESS DENIED", "the verdict was overwritten");
}

#[test]
fn the_panel_is_not_redrawn_when_nothing_changed() {
    // A 1602 over a PCF8574 is slow. Redrawing an unchanged screen every poll
    // would be the whole I2C budget spent saying nothing.
    let mut r = reader();
    let first = r.tick(1_000);
    let second = r.tick(1_100);
    assert!(
        !second.iter().any(|a| matches!(a, Action::Show(_))),
        "redrew an unchanged panel: first={first:?} second={second:?}"
    );
}

#[test]
fn the_clock_reaches_the_idle_screen_but_never_a_decision() {
    let mut r = reader();
    r.health(
        Health {
            clock: Some("14:05".into()),
            ..online()
        },
        0,
    );
    assert_eq!(here(&r), ("Present card".into(), "14:05".into()));
    // And it does not leak into a verdict, which has sixteen columns to spend
    // on the answer.
    r.card(ALICE, 100);
    let out = r.message(topic::SCAN_RESPONSE, &no("Unknown card"), 200);
    assert_eq!(screen(&out).unwrap().1, "Unknown card");
}

// ------------------------------------------------- the heartbeat gets through

#[test]
fn the_box_pulses_whatever_it_switches() {
    // `Reader` relays every publish the module makes (`carry`), and this is
    // the arm that would notice if it stopped: a reader that filtered its
    // module's actions by topic would still pass every other test here and
    // silently go quiet on the edge (CS:S #170). What the pulse CONTAINS is
    // spaces-device's claim, proven there; this proves only that it arrives,
    // from both configurations, at start and on the heartbeat timer.
    for mut r in [
        Reader::new(Config::switches_nothing(ME, TOOL)),
        Reader::new(Config::new(ME, TOOL)),
    ] {
        let first = published(&r.start(0), topic::ALIVE_REQUEST);
        assert_eq!(first.len(), 1, "no pulse at start: {first:?}");
        assert_eq!(first[0]["device_id"], ME);
        assert!(published(&r.tick(999), topic::ALIVE_REQUEST).is_empty());
        assert_eq!(published(&r.tick(1000), topic::ALIVE_REQUEST).len(), 1);
    }
}

// ------------------------------------------------- the session on the panel

#[test]
fn a_reader_with_no_relay_says_signed_in_rather_than_unlocked() {
    // It is not what is switching the machine, so it must not imply the tool is
    // hot. The distinction is the whole reason `Activity` carries `energized`.
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    r.tick(200 + SHOW_MS);
    assert_eq!(here(&r), ("Signed in".into(), "Tap to finish".into()));
}

#[test]
fn a_reader_that_gates_the_tool_says_unlocked_once_it_really_is() {
    let mut r = gating_reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    // Authorized, but no lease yet: the tool is dark, so the panel must not
    // claim otherwise.
    r.tick(200 + SHOW_MS);
    assert_eq!(here(&r).0, "Signed in", "claimed unlocked before the lease");
    r.message(topic::LEASE, &lease(600_000), 300 + SHOW_MS);
    assert_eq!(here(&r).0, "Tool unlocked");
}

#[test]
fn a_sensing_reader_shows_the_meter_and_whether_it_is_climbing() {
    // The visuals must be truthful about money: a member billed by the second
    // sees the figure, and sees it stop when the machine does. The figure is
    // the module's held one (its own arms pin the bridging); this proves the
    // panel renders it and does not flicker on a pause the bill bridges.
    let mut c = Config::switches_nothing(ME, TOOL);
    c.senses_running = true;
    c.use_hold_ms = 2_000;
    let mut r = Reader::new(c);
    r.start(0);
    r.health(online(), 0);
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    r.tick(200 + SHOW_MS);
    assert_eq!(here(&r), ("Signed in".into(), "Idle     0:00:00".into()));

    let t0 = 1_000 + SHOW_MS;
    r.running(true, t0);
    r.tick(t0 + 65_000);
    assert_eq!(here(&r).1, "In use   0:01:05");

    // Raw drops; inside the hold the panel still says in use.
    r.running(false, t0 + 65_000);
    r.tick(t0 + 66_000);
    assert_eq!(
        here(&r).1,
        "In use   0:01:06",
        "flickered on a bridged pause"
    );
    // Past it: idle, and the figure stays where the bridge ended. The release
    // is noticed on a tick (the box polls every 75 ms), so poll the boundary.
    r.tick(t0 + 67_000);
    r.tick(t0 + 70_000);
    assert_eq!(here(&r).1, "Idle     0:01:07");
    r.tick(t0 + 90_000);
    assert_eq!(here(&r).1, "Idle     0:01:07", "idle time was counted");
}

#[test]
fn a_reader_that_senses_nothing_shows_no_figure() {
    // Its bottom line is still the re-tap hint: with no sensor and no relay a
    // second tap is the only stop, and inventing a meter would hide that.
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    r.tick(200 + SHOW_MS + 30_000);
    assert_eq!(here(&r), ("Signed in".into(), "Tap to finish".into()));
}

#[test]
fn a_reader_that_gates_the_tool_drives_the_output() {
    let mut r = gating_reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    let out = r.message(topic::LEASE, &lease(600_000), 300);
    assert!(
        out.contains(&Action::Set {
            output: Output::Tool,
            on: true
        }),
        "the relay was never closed: {out:?}"
    );
}

#[test]
fn a_reader_with_no_relay_drives_no_output() {
    let mut r = reader();
    r.card(ALICE, 100);
    let out = r.message(topic::SCAN_RESPONSE, &yes(), 200);
    assert!(
        !out.iter().any(|a| matches!(
            a,
            Action::Set {
                output: Output::Tool,
                ..
            }
        )),
        "a device with no relay tried to energize a tool: {out:?}"
    );
}

#[test]
fn signing_off_releases_the_tool_and_says_so() {
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    let out = r.card(ALICE, 100 + SAME_CARD_COOLDOWN_MS + 1);
    assert_eq!(
        publishes_to(&out),
        vec![topic::USAGE_REQUEST, topic::STOP_REQUEST],
        "a second tap did not release the tool: {out:?}"
    );
    assert_eq!(
        published(&out, topic::STOP_REQUEST)[0]["card"],
        ALICE,
        "released with the wrong card"
    );
    assert_eq!(screen(&out).unwrap().0, "Session ended");
}

#[test]
fn the_stop_button_ends_the_session_and_needs_no_card() {
    // Anyone may press it: a stop needs no credential, by design -- anyone may
    // cut the power. The session is still closed with the card that opened it,
    // because that is who the usage belongs to.
    let mut r = reader();
    // With no session open the stop still goes out, naming the resource and
    // no card (spaces-device 0.4): the platform ends whatever session is open
    // on the resource, and a stop that lands on nothing is harmless.
    let idle = r.tool_off(50);
    assert_eq!(publishes_to(&idle), vec![topic::STOP_REQUEST]);
    assert!(published(&idle, topic::STOP_REQUEST)[0]
        .get("card")
        .is_none());
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    let out = r.tool_off(30_000);
    assert_eq!(
        publishes_to(&out),
        vec![topic::USAGE_REQUEST, topic::STOP_REQUEST],
        "the stop button did not release the tool: {out:?}"
    );
    assert_eq!(published(&out, topic::STOP_REQUEST)[0]["card"], ALICE);
    assert_eq!(screen(&out).unwrap().0, "Session ended");
}

#[test]
fn a_reader_that_does_not_take_a_second_tap_offers_no_hint_about_one() {
    // Telling a member to "tap to finish" where it does nothing is worse than
    // saying nothing: they will tap, nothing will happen, and they will leave.
    let mut c = Config::switches_nothing(ME, TOOL);
    c.stop_on_second_tap = false;
    let mut r = Reader::new(c);
    r.start(0);
    r.health(online(), 0);
    r.card(ALICE, 100);
    r.message(topic::SCAN_RESPONSE, &yes(), 200);
    r.tick(200 + SHOW_MS);
    assert_eq!(here(&r), ("Signed in".into(), "".into()));
}

// --------------------------------------------------------- sixteen columns

#[test]
fn every_screen_fits_the_panel() {
    // The constraint the whole display design is built around. A screen that
    // overflows is not a cosmetic fault: the 1602 wraps to the other line and
    // the member reads two half sentences.
    let long = "a reason far longer than the panel can possibly show";
    let health = Health {
        wifi: true,
        broker: true,
        address: Some("255.255.255.255".into()),
        clock: Some("23:59".into()),
    };
    let screens = [
        toolguard_core::screen::idle(&health),
        toolguard_core::screen::asking(),
        toolguard_core::screen::granted(),
        toolguard_core::screen::denied(long),
        toolguard_core::screen::no_answer(),
        toolguard_core::screen::diagnostics(&health),
        toolguard_core::screen::in_session(true, true, None),
        toolguard_core::screen::in_session(false, false, None),
        // 99:59:59 running, and idle: the widest the figure gets in a session
        // any cap allows.
        toolguard_core::screen::in_session(
            true,
            true,
            Some(Usage {
                running: true,
                seconds: 359_999,
            }),
        ),
        toolguard_core::screen::in_session(
            false,
            false,
            Some(Usage {
                running: false,
                seconds: 0,
            }),
        ),
        toolguard_core::screen::ended(),
    ];
    for s in screens {
        for line in s.rows() {
            assert!(
                line.chars().count() <= COLS,
                "{line:?} is {} characters, and the panel has {COLS}",
                line.chars().count()
            );
        }
    }
}

#[test]
fn a_multibyte_reason_is_cut_by_character_and_does_not_panic() {
    // Truncating by byte would split a UTF-8 sequence and panic on a reason
    // that happens to contain one. The edge's strings are not ours to predict.
    let s = toolguard_core::screen::denied("héllo wörld — a very long reason indeed");
    assert!(s.bottom().chars().count() <= COLS);
}

// ---------------------------------------------------------- a door box (CS:S #182)
//
// The same image, configured to drive a door's latch and switch nothing. Its
// session is the pulse: the latch is released for exactly as long as the edge
// says, the panel says so, and nothing is "signed into".

fn door_box() -> Reader {
    let mut r = Reader::new(Config::drives_a_latch(ME, "front-door"));
    r.start(0);
    r.health(online(), 0);
    r
}

fn yes_with_strike(pulse_ms: u64) -> Vec<u8> {
    serde_json::json!({"resource": "front-door", "authorized": true, "strike": {"pulse_ms": pulse_ms}})
        .to_string()
        .into_bytes()
}

#[test]
fn a_door_box_releases_the_latch_and_says_door_open() {
    let mut r = door_box();
    r.card(ALICE, 100);
    let out = r.message(topic::SCAN_RESPONSE, &yes_with_strike(4200), 200);
    assert!(
        out.contains(&Action::Set {
            output: Output::Strike,
            on: true
        }),
        "the latch was never released: {out:?}"
    );
    assert!(
        !out.iter().any(|a| matches!(
            a,
            Action::Set {
                output: Output::Tool,
                ..
            }
        )),
        "a door box has no relay to close: {out:?}"
    );
    assert_eq!(screen(&out).map(|s| s.0), Some("Door open".to_string()));
    // Past the outcome hold and still inside the pulse: the resting screen is
    // the same truth, not "Signed in".
    r.tick(200 + SHOW_MS);
    assert_eq!(here(&r).0, "Door open");
    // The pulse ends: latch closed, and the panel rests on "Present card"
    // rather than announcing a session that nobody opened.
    let end = r.tick(4400);
    assert!(
        end.contains(&Action::Set {
            output: Output::Strike,
            on: false
        }),
        "the latch was never closed again: {end:?}"
    );
    assert!(
        !end.iter()
            .any(|a| matches!(a, Action::Show(s) if s.top() == "Session ended")),
        "a pulse ending was announced as a session ending: {end:?}"
    );
    assert_eq!(here(&r).0, "Present card");
    assert!(
        publishes_to(&end).is_empty(),
        "a pulse ending sent something: {end:?}"
    );
}

#[test]
fn a_door_boxs_stop_button_releases_the_resource_without_a_card() {
    // Its session was the pulse and is long over; the stop still goes out,
    // naming the resource alone -- that is how a room held open until a stop
    // is closed from the door that opened it.
    let mut r = door_box();
    let out = r.tool_off(5000);
    assert_eq!(publishes_to(&out), vec![topic::STOP_REQUEST]);
    let stop = &published(&out, topic::STOP_REQUEST)[0];
    assert_eq!(stop["resource"], "front-door");
    assert!(
        stop.get("card").is_none(),
        "named a card it does not have: {stop}"
    );
}
