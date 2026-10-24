//! The rules a badge reader must not get wrong, each pinned by a test.
//!
//! Time is whatever these tests say it is: every call takes the current
//! monotonic milliseconds, which is what makes a reply timing out testable.
//!
//! There is no board yet, and that is the point rather than an excuse. Every
//! assertion here is about a decision or about what a person reads off a 1602,
//! and none of it needs an RC522 to be true.

use serde_json::json;
use spaces_device::wire::topic;
use toolguard_core::reader::{Action, Reader, REPLY_TIMEOUT_MS, SAME_CARD_COOLDOWN_MS, SHOW_MS};
use toolguard_core::screen::{Health, COLS};

const TOOL: &str = "laser-01";
const ALICE: &str = "04A1B2C3";
const BOB: &str = "04D4E5F6";

/// A reader that is online and idle.
fn reader() -> Reader {
    let mut r = Reader::new(TOOL);
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

/// The last screen an action list puts up, as `(top, bottom)`.
fn screen(actions: &[Action]) -> Option<(String, String)> {
    actions.iter().rev().find_map(|a| match a {
        Action::Show(s) => Some((s.top().to_string(), s.bottom().to_string())),
        _ => None,
    })
}

fn yes_for(tool: &str) -> Vec<u8> {
    json!({"tool_id": tool, "authorized": true, "reason": "authorized"})
        .to_string()
        .into_bytes()
}

fn no_for(tool: &str, reason: &str) -> Vec<u8> {
    json!({"tool_id": tool, "authorized": false, "reason": reason})
        .to_string()
        .into_bytes()
}

// ------------------------------------------------------------- asking

#[test]
fn a_card_asks_the_edge_about_this_tool_and_that_card() {
    let mut r = reader();
    let actions = r.card(ALICE, 100);

    let sent = published(&actions, topic::TOOL_ON_REQUEST);
    assert_eq!(sent.len(), 1, "one card, one request: {actions:?}");
    assert_eq!(sent[0]["card"], ALICE);
    assert_eq!(
        sent[0]["tool_id"], TOOL,
        "the request must name the tool this reader guards, or the edge answers about something else"
    );
}

#[test]
fn the_panel_says_it_is_working_while_it_waits() {
    let mut r = reader();
    let (top, _) = screen(&r.card(ALICE, 100)).expect("a screen while asking");
    assert_eq!(top, "Checking card");
}

#[test]
fn a_second_card_while_waiting_is_dropped_not_queued() {
    // The reply carries no card, so two requests in flight cannot be told
    // apart and the second would take the first's answer.
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.card(BOB, 200);
    assert!(
        published(&actions, topic::TOOL_ON_REQUEST).is_empty(),
        "a second card must not produce a second request: {actions:?}"
    );
}

#[test]
fn the_same_card_held_against_the_reader_is_one_tap() {
    let mut r = reader();
    r.card(ALICE, 100);
    // Answer it, so the reader is idle rather than busy -- otherwise this would
    // pass for the wrong reason, via the one-outstanding-request rule above.
    r.message(topic::TOOL_ON_RESPONSE, &yes_for(TOOL), 150);
    r.tick(150 + SHOW_MS);

    // The window runs from the last READ of that card -- not from the outcome
    // clearing, which is what my first version measured and which left the card
    // long out of cooldown.
    let again = r.card(ALICE, 100 + SAME_CARD_COOLDOWN_MS - 1);
    assert!(
        published(&again, topic::TOOL_ON_REQUEST).is_empty(),
        "the same card inside the cooldown must not ask again: {again:?}"
    );

    // AND THE WINDOW SLIDES, which the suppressed read above just moved. That
    // is the point rather than an accident: an RC522 reports a card for as long
    // as it is in the field, so a card left sitting there must stay quiet the
    // whole time instead of asking afresh every two seconds. Measuring from the
    // FIRST read would make a forgotten card a request generator.
    let held = r.card(ALICE, 100 + SAME_CARD_COOLDOWN_MS + 1);
    assert!(
        published(&held, topic::TOOL_ON_REQUEST).is_empty(),
        "a card still in the field must stay suppressed: {held:?}"
    );

    // Removed for the whole window, then presented again: a member really can
    // tap twice, and this is the arm that stops the cooldown from being a mute.
    let last_read = 100 + SAME_CARD_COOLDOWN_MS + 1;
    let later = r.card(ALICE, last_read + SAME_CARD_COOLDOWN_MS + 1);
    assert_eq!(
        published(&later, topic::TOOL_ON_REQUEST).len(),
        1,
        "after the card has been away for the whole window it must ask again"
    );
}

#[test]
fn a_different_card_is_never_held_off_by_the_cooldown() {
    // The control for the arm above: a cooldown that suppressed everything
    // would pass it, and would make the reader useless for a queue of people.
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::TOOL_ON_RESPONSE, &no_for(TOOL, "Unknown card"), 150);
    r.tick(150 + SHOW_MS);

    let bob = r.card(BOB, 150 + SHOW_MS + 1);
    assert_eq!(
        published(&bob, topic::TOOL_ON_REQUEST).len(),
        1,
        "a different card must ask immediately: {bob:?}"
    );
}

// ------------------------------------------------------------- answers

#[test]
fn a_yes_shows_granted() {
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(topic::TOOL_ON_RESPONSE, &yes_for(TOOL), 200);
    assert_eq!(
        screen(&actions).map(|(t, _)| t),
        Some("ACCESS GRANTED".into())
    );
}

#[test]
fn a_no_shows_denied_and_the_reason() {
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(
        topic::TOOL_ON_RESPONSE,
        &no_for(TOOL, "Training required"),
        200,
    );
    let (top, bottom) = screen(&actions).expect("a screen for a refusal");
    assert_eq!(top, "ACCESS DENIED");
    assert_eq!(
        bottom, "Training require",
        "the edge's own reason belongs on the panel, CUT TO FIT -- and this is \
         the finding, not a quirk of the fixture: `Training required` is 17 \
         characters and the commonest refusal the platform sends, so the most \
         frequent denial a member sees loses its last letter. Recorded in the \
         README; shortening it here would mean matching on reason text, which \
         FIRMWARE.md warns against."
    );
}

#[test]
fn an_unparseable_answer_is_a_refusal_and_not_an_absence() {
    // Something answered. Absence of a yes is never permission.
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(topic::TOOL_ON_RESPONSE, b"{not json", 200);
    assert_eq!(
        screen(&actions).map(|(t, _)| t),
        Some("ACCESS DENIED".into())
    );
}

#[test]
fn a_reply_with_no_verdict_is_not_a_yes() {
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(
        topic::TOOL_ON_RESPONSE,
        json!({"tool_id": TOOL, "reason": "hmm"})
            .to_string()
            .as_bytes(),
        200,
    );
    assert_eq!(
        screen(&actions).map(|(t, _)| t),
        Some("ACCESS DENIED".into()),
        "a reply arriving is not a yes, and a missing `authorized` is not a yes"
    );
}

#[test]
fn nothing_answering_is_a_no_that_does_not_blame_the_member() {
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.tick(100 + REPLY_TIMEOUT_MS);
    let (top, _) = screen(&actions).expect("a screen when the edge is silent");
    assert_eq!(
        top, "No answer",
        "a timeout is a no, but a different no from a refusal: the member did \
         nothing wrong and retrying may work"
    );
}

// --------------------------------------------- whose answer is this (CS:S #166)

#[test]
fn an_answer_about_another_tool_does_not_decide_anything_here() {
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(topic::TOOL_ON_RESPONSE, &yes_for("drill-07"), 200);
    assert!(
        screen(&actions).is_none(),
        "a neighbour's verdict must not reach this panel: {actions:?}"
    );
}

#[test]
fn and_our_own_answer_still_arrives_afterwards() {
    // The control, and the reason the phase must not change: treating a
    // neighbour's reply as a refusal would deny this member for someone else's
    // swipe at another machine.
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::TOOL_ON_RESPONSE, &yes_for("drill-07"), 200);
    let ours = r.message(topic::TOOL_ON_RESPONSE, &yes_for(TOOL), 300);
    assert_eq!(
        screen(&ours).map(|(t, _)| t),
        Some("ACCESS GRANTED".into()),
        "after ignoring a stray answer, ours must still be honoured"
    );
}

#[test]
fn an_answer_naming_no_tool_is_still_accepted() {
    // An edge predating CS:S #166 sends no tool_id. Refusing those would brick
    // this reader against an edge that has not been updated, so the absent case
    // falls back to the one-outstanding-request window. A deliberate hole, with
    // a test rather than a comment.
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(
        topic::TOOL_ON_RESPONSE,
        json!({"authorized": true}).to_string().as_bytes(),
        200,
    );
    assert_eq!(
        screen(&actions).map(|(t, _)| t),
        Some("ACCESS GRANTED".into())
    );
}

#[test]
fn an_answer_with_nothing_outstanding_is_ignored() {
    let mut r = reader();
    let actions = r.message(topic::TOOL_ON_RESPONSE, &yes_for(TOOL), 100);
    assert!(
        screen(&actions).is_none(),
        "a late or stray answer must not show a verdict nobody asked for: {actions:?}"
    );
}

#[test]
fn a_lease_is_somebody_elses_business() {
    // A reader has no relay and is never bound in the `power` role, so it is
    // never leased. Silence here is correct rather than an omission.
    let mut r = reader();
    let actions = r.message(
        topic::LEASE,
        json!({"tool_id": TOOL, "device_id": "x", "grant": true, "ttl_ms": 3000})
            .to_string()
            .as_bytes(),
        100,
    );
    assert!(
        actions.is_empty(),
        "a lease must change nothing: {actions:?}"
    );
}

// ------------------------------------------------------------- offline

#[test]
fn a_card_is_not_sent_when_there_is_no_broker_to_send_it_to() {
    let mut r = Reader::new(TOOL);
    r.health(
        Health {
            wifi: true,
            broker: false,
            address: Some("192.168.1.50".into()),
            clock: None,
        },
        0,
    );
    let actions = r.card(ALICE, 100);
    assert!(
        published(&actions, topic::TOOL_ON_REQUEST).is_empty(),
        "publishing into a broker we are not connected to is a silent drop, and \
         the member watches a reader do nothing: {actions:?}"
    );
    // No `Show` here, and that is correct: the diagnostic screen was already up
    // from `health()`, and redrawing an unchanged panel is suppressed. So the
    // assertion reads the panel's STATE rather than the event.
    assert_eq!(
        r.screen().map(|s| s.top().to_string()),
        Some("No edge".into()),
        "the member must be looking at which half is missing"
    );
}

#[test]
fn the_panel_names_which_half_is_missing() {
    let mut r = Reader::new(TOOL);
    let no_wifi = r.health(Health::default(), 0);
    assert_eq!(screen(&no_wifi).map(|(t, _)| t), Some("No Wi-Fi".into()));

    let mut r2 = Reader::new(TOOL);
    let no_edge = r2.health(
        Health {
            wifi: true,
            broker: false,
            address: None,
            clock: None,
        },
        0,
    );
    assert_eq!(
        screen(&no_edge).map(|(t, _)| t),
        Some("No edge".into()),
        "\"it is not working\" is not a diagnostic; which half is"
    );
}

#[test]
fn the_diagnostic_screen_carries_the_address_to_reach_the_log_at() {
    let mut r = Reader::new(TOOL);
    let actions = r.health(
        Health {
            wifi: true,
            broker: false,
            address: Some("192.168.1.50".into()),
            clock: None,
        },
        0,
    );
    let (_, bottom) = screen(&actions).expect("a diagnostic screen");
    assert_eq!(bottom, "192.168.1.50");
}

// ------------------------------------------------------------- the panel

#[test]
fn every_screen_fits_the_panel() {
    // The assertion that only exists because the panel is 16x2 and the edge's
    // reasons are not. `Tool not authorized for this user` is 38 characters.
    let long = "Tool not authorized for this user";
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(topic::TOOL_ON_RESPONSE, &no_for(TOOL, long), 200);
    let (top, bottom) = screen(&actions).expect("a screen");
    assert!(
        top.chars().count() <= COLS && bottom.chars().count() <= COLS,
        "a screen overflowed the panel: {top:?} / {bottom:?}"
    );
    assert_eq!(bottom, "Tool not authori", "cut to the panel, not dropped");
}

#[test]
fn a_multibyte_reason_is_cut_by_character_and_does_not_panic() {
    // `str::truncate` panics on a multi-byte boundary, and a reason from the
    // edge is arbitrary UTF-8 this firmware does not control.
    let mut r = reader();
    r.card(ALICE, 100);
    let actions = r.message(
        topic::TOOL_ON_RESPONSE,
        &no_for(TOOL, "ééééééééééééééééééé"),
        200,
    );
    let (_, bottom) = screen(&actions).expect("a screen");
    assert_eq!(bottom.chars().count(), COLS);
}

#[test]
fn the_panel_is_not_redrawn_when_nothing_changed() {
    // Three I2C writes per nibble on a PCF8574: redrawing an unchanged screen
    // every poll would spend the whole budget saying nothing.
    let mut r = reader();
    let first = r.tick(10);
    let second = r.tick(20);
    assert!(
        !first.iter().any(|a| matches!(a, Action::Show(_)))
            && !second.iter().any(|a| matches!(a, Action::Show(_))),
        "an idle tick must not redraw: {first:?} {second:?}"
    );
}

#[test]
fn an_outcome_is_replaced_by_the_resting_screen_once_it_has_been_read() {
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::TOOL_ON_RESPONSE, &yes_for(TOOL), 200);
    let still = r.tick(200 + SHOW_MS - 1);
    assert!(
        !still.iter().any(|a| matches!(a, Action::Show(_))),
        "the outcome must stay up long enough to read: {still:?}"
    );
    let after = r.tick(200 + SHOW_MS);
    assert_eq!(screen(&after).map(|(t, _)| t), Some("Present card".into()));
}

#[test]
fn health_news_does_not_wipe_an_outcome_off_the_panel() {
    let mut r = reader();
    r.card(ALICE, 100);
    r.message(topic::TOOL_ON_RESPONSE, &yes_for(TOOL), 200);
    let actions = r.health(online(), 250);
    assert!(
        screen(&actions).is_none(),
        "a broker reconnect mid-read must not replace ACCESS GRANTED: {actions:?}"
    );
}

#[test]
fn the_clock_reaches_the_idle_screen_but_never_a_decision() {
    let mut r = Reader::new(TOOL);
    let actions = r.health(
        Health {
            wifi: true,
            broker: true,
            address: Some("192.168.1.50".into()),
            clock: Some("14:05".into()),
        },
        0,
    );
    let (top, bottom) = screen(&actions).expect("an idle screen");
    assert_eq!(top, "Present card");
    assert_eq!(bottom, "14:05");

    // And a reader with no clock is still fully usable: the platform's tool
    // path is measured on a monotonic timer by design, so SNTP never gates a
    // card.
    let mut r2 = reader();
    let sent = published(&r2.card(ALICE, 100), topic::TOOL_ON_REQUEST);
    assert_eq!(sent.len(), 1, "no wall clock must not stop a swipe");
}
