//! The stop button fires once per press, after the bounce, and never on a
//! glitch. Each arm is the thing a real switch does that a clean signal does
//! not.

use toolguard_core::StopButton;

#[test]
fn a_press_fires_once_after_the_debounce_and_not_again_while_held() {
    let mut b = StopButton::new(50);
    assert!(!b.sample(true, 0), "fired on the first bounce");
    assert!(!b.sample(true, 49));
    assert!(
        b.sample(true, 50),
        "did not fire once the contact had settled"
    );
    // A finger stays down for a long time. That is one stop, not many.
    assert!(!b.sample(true, 51));
    assert!(!b.sample(true, 5_000));
}

#[test]
fn releasing_re_arms_it_for_the_next_press() {
    let mut b = StopButton::new(50);
    b.sample(true, 0);
    assert!(b.sample(true, 50));
    assert!(!b.sample(false, 100));
    b.sample(true, 200);
    assert!(
        b.sample(true, 250),
        "a second press after a release did not fire"
    );
}

#[test]
fn a_glitch_shorter_than_the_debounce_never_fires() {
    // Contact bounce and a noisy line next to a laser supply: closed for less
    // than the debounce, then open. Must not stop somebody's job.
    let mut b = StopButton::new(50);
    assert!(!b.sample(true, 0));
    assert!(!b.sample(true, 30));
    assert!(!b.sample(false, 40));
    // And the clock restarts from the next closure, not the first.
    assert!(!b.sample(true, 60));
    assert!(
        !b.sample(true, 100),
        "counted the earlier glitch towards this press"
    );
    assert!(b.sample(true, 110));
}

#[test]
fn an_open_contact_never_fires() {
    let mut b = StopButton::new(50);
    for t in [0, 50, 100, 10_000] {
        assert!(!b.sample(false, t), "fired with nothing pressed at {t}");
    }
}
