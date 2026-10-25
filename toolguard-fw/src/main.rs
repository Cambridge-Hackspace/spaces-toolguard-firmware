//! A ToolGuard badge reader.
//!
//! Scan a card, ask the edge about it, show the answer. Every decision is in
//! `toolguard_core`; this is the shell that gives them hardware. Nothing in
//! this file names a GPIO — the pin table is in [`board`], per chip.
//!
//! ## What it reuses rather than rebuilds
//!
//! `spaces-device-espidf::boot` does everything that is not reader-specific,
//! and does it with scars worth inheriting: a setup portal that handles an
//! eight-emoji invite without length-checking or normalising it, a Wi-Fi join
//! that ends a failed attempt on the disconnect event instead of waiting out a
//! ten-second timeout, the NVS store, register-once-and-persist-before-anything
//! -else, the OTA endpoint with milestone rollback, and the web log.
//!
//! ## What is reader-specific
//!
//! The roles it declares (`reader` alone — it has no relay), the one setting it
//! adds (its tool), the two peripherals, and the loop at the bottom.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use esp_idf_svc::hal::gpio::{Output, PinDriver, Pull};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::{eventloop::EspSystemEventLoop, nvs::EspDefaultNvsPartition};
use spaces_device_espidf::status_light::{Status, StatusLight};
use spaces_device_espidf::store::{key, Field, Store};
use spaces_device_espidf::update::Updates;
use spaces_device_espidf::{boot, watch_setup_button, weblog, Firmware};
use toolguard_core::screen::Screen;
use toolguard_core::{Action, Health, Reader};

mod board;
mod broker;
mod lcd;
mod rc522;
mod settings;

/// How often the antenna is asked whether a card is present.
///
/// 75 ms, which is what `nfc-522` settled on: fast enough that a tap by a
/// person moving at human speed is never missed, slow enough that the SPI bus
/// is mostly idle.
const POLL: Duration = Duration::from_millis(75);

/// This device reads cards. It switches nothing.
///
/// `capabilities.roles` is the device's own claim about itself, and the server
/// lets an administrator bind it in any role it declares — so claiming `power`
/// here would let someone make a reader with no relay the thing that energizes
/// a machine, and the mistake surfaces at the far end of the system as a tool
/// that never comes on.
/// The tool this box guards. The rest of what it is lives in [`settings`],
/// which is read from the same store.
const TOOL_FIELD: &[Field] = &[Field {
    key: key::TOOL_ID,
    label: "Tool",
    hint: "the machine this reader guards: its external id on the platform, e.g. laser-01",
    secret: false,
    required: true,
}];

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    // Before any other logging call: this installs the tee that also captures
    // ESP-IDF's own C-side output, so `/log` holds the whole story.
    weblog::start();

    let peripherals = Peripherals::take()?;
    // Taken apart immediately, because the status light and the setup button are
    // wanted before `boot()` and the two peripherals after it, and
    // `peripherals.pins` cannot be moved from twice.
    // --- every pin this firmware uses, per chip -----------------------------
    //
    // Concrete pins rather than GPIO numbers: that is what esp-idf-hal's
    // generic bounds want, and it makes using one pin twice a COMPILE error
    // rather than a runtime surprise. Which matters -- an earlier draft of this
    // had the S3 using GPIO8 for both the status LED and I2C SDA, and only the
    // move checker would have caught it, on a chip nobody here can build.
    //
    // All nine in one tuple so the conflicts are visible in one place. The
    // reasoning per choice is in `board`.
    let pins = peripherals.pins;

    // C6 Super Mini: GPIO8 is the RGB LED and a strapping pin, GPIO9 is BOOT,
    // GPIO12/13 are native USB and the only console there is.
    #[cfg(esp32c6)]
    let (light_pin, setup_pin, sclk, mosi, miso, cs, rst, sda, scl, sense, relay) = (
        pins.gpio8,
        pins.gpio9,
        pins.gpio0,
        pins.gpio1,
        pins.gpio2,
        pins.gpio3,
        pins.gpio18,
        pins.gpio22,
        pins.gpio23,
        // Both from the freely usable set (IO0-3, IO14, IO18-23), which SPI and
        // I2C leave 14, 19, 20 and 21 of. Neither can do ADC: on the C6 that is
        // GPIO0-6 only, and SPI has all of those -- so a sensed input here is
        // digital, which is what an optoisolated enable line gives anyway.
        pins.gpio14,
        pins.gpio19,
    );

    // UNTESTED: no C3 here, so this compiles and nothing more. Same strapping
    // pair (8 and 9) as the C6; USB is on 18/19 rather than 12/13.
    #[cfg(esp32c3)]
    let (light_pin, setup_pin, sclk, mosi, miso, cs, rst, sda, scl, sense, relay) = (
        pins.gpio8,
        pins.gpio9,
        pins.gpio10,
        pins.gpio7,
        pins.gpio6,
        pins.gpio5,
        pins.gpio4,
        pins.gpio1,
        pins.gpio0,
        pins.gpio2,
        pins.gpio3,
    );

    // UNTESTED: no S3 here. Its addressable LED is on GPIO48 on the boards that
    // have one, flash and PSRAM occupy 26-32, and USB is 19/20 -- so this stays
    // clear of all of them. GPIO9 is kept for BOOT for consistency with the
    // other two rather than because the S3 needs it there.
    #[cfg(esp32s3)]
    let (light_pin, setup_pin, sclk, mosi, miso, cs, rst, sda, scl, sense, relay) = (
        pins.gpio48,
        pins.gpio0,
        pins.gpio12,
        pins.gpio11,
        pins.gpio13,
        pins.gpio10,
        pins.gpio14,
        pins.gpio4,
        pins.gpio5,
        pins.gpio15,
        pins.gpio16,
    );
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let version = env!("CARGO_PKG_VERSION");

    log::info!("spaces-toolguard-firmware {version} on {}", board::CHIP);

    // Not on the critical path: `start` is infallible and degrades to a no-op,
    // because a reader with a dead LED should still read cards.
    let light = StatusLight::start(light_pin, Status::Connecting);
    // Early, so an image on probation is watched from the first moment rather
    // than from whenever the loop happens to start.
    let updates = Updates::start(
        nvs.clone(),
        Store::new(nvs.clone()),
        light.clone(),
        env!("CARGO_PKG_NAME"),
        version,
    );
    watch_setup_button(
        PinDriver::input(setup_pin, Pull::Up)?,
        Store::new(nvs.clone()),
    )?;

    // The shared settings plus ours. `Box::leak` is the idiom the demo
    // establishes: the portal wants a `&'static [Field]`, and this list is
    // built once at boot and lives as long as the program.
    let fields: &'static [Field] = Box::leak(
        spaces_device_espidf::store::FIELDS
            .iter()
            .copied()
            .chain(TOOL_FIELD.iter().copied())
            .chain(settings::FIELDS.iter().copied())
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    // Before boot, because `roles` is what this device registers AS: a device
    // declares what it can do, and an administrator may then bind it to a tool
    // in a role it declared. Claiming `power` with nothing wired to a relay is
    // how a box with no relay becomes what a machine's safety rests on.
    let mut settings = settings::Settings::read(&Store::new(nvs.clone()));
    let roles = settings.roles();
    log::info!("this device: {}", settings.describe());
    log::info!("registering as {roles:?}");

    let online = match boot(
        peripherals.modem,
        sysloop,
        nvs,
        Firmware {
            fields,
            roles: &roles,
            software_version: version,
        },
        light.clone(),
        updates,
    ) {
        Ok(online) => online,
        Err(e) => {
            light.set(Status::Failed);
            log::error!("{e:#}");
            std::thread::sleep(Duration::from_secs(30));
            esp_idf_svc::hal::reset::restart();
        }
    };

    let tool_id = online
        .store
        .get(key::TOOL_ID)
        .unwrap_or_default()
        .trim()
        .to_string();
    settings.config.device_id = online.credentials.device_id.clone();
    settings.config.tool_id = tool_id.clone();

    // --- the two peripherals ------------------------------------------------
    //
    // The panel first, so a reader fault has somewhere to be displayed.
    let mut panel = match lcd::start(peripherals.i2c0, sda, scl, board::LCD_ADDRESSES) {
        Ok(p) => Some(p),
        Err(e) => {
            // Degraded, not broken: the reader can still authorize a member,
            // who sees the result on the machine rather than on a display. So
            // it complains and carries on.
            log::error!("no panel: {e:#}");
            None
        }
    };

    // Only when this box is supposed to have one. A plug has no antenna and
    // failing on its absence would make the one configuration that needs no
    // reader impossible to run.
    let mut card_reader = if settings.config.reads_cards {
        match rc522::start(peripherals.spi2, sclk, mosi, miso, cs, rst) {
            Ok(r) => Some(r),
            Err(e) => {
                // On a box that should have read cards, no antenna is the
                // entire function of the device missing, so this is terminal —
                // and it says so ON the panel, which is where a member
                // standing there will look.
                log::error!("{e:#}");
                if let Some(p) = panel.as_mut() {
                    let _ = p.show(&Screen::new("Reader fault", "see /log"));
                }
                light.set(Status::Failed);
                std::thread::sleep(Duration::from_secs(30));
                esp_idf_svc::hal::reset::restart();
            }
        }
    } else {
        log::info!("configured with no card reader; not starting the antenna");
        None
    };

    // The relay, only when something is meant to be wired to it. Driven low at
    // boot before anything else can ask for it: an output whose state nobody
    // has set yet is an output that might be closed.
    let mut relay: Option<PinDriver<'_, Output>> = if settings.config.gates_power {
        match PinDriver::output(relay) {
            Ok(mut p) => {
                let _ = p.set_low();
                Some(p)
            }
            Err(e) => {
                log::error!("configured to switch the tool, but the relay pin failed: {e:#}");
                light.set(Status::Failed);
                std::thread::sleep(Duration::from_secs(30));
                esp_idf_svc::hal::reset::restart();
            }
        }
    } else {
        None
    };

    // The sensed input. Pulled up, so an unconnected pin reads as "not
    // working" when the sense is active-low, which is the quiet default rather
    // than a machine that looks permanently busy.
    let mut sense = if settings.config.senses_running {
        match PinDriver::input(sense, Pull::Up) {
            Ok(p) => Some(p),
            Err(e) => {
                log::error!("configured to sense the machine, but the input pin failed: {e:#}");
                None
            }
        }
    } else {
        None
    };
    let sense_active_low = settings.sense_active_low;

    // --- the loop -----------------------------------------------------------
    let mut reader = Reader::new(settings.config.clone());
    let (tx, rx) = mpsc::channel::<broker::Event>();
    let mut client = broker::connect(&online, tx)?;

    let started = Instant::now();
    let now = || started.elapsed().as_millis() as u64;
    let address = online.address.clone();

    // Resting screen before the broker answers: wifi is up by here (boot does
    // not return until it is), so this reads "No edge" and names the half that
    // is actually missing.
    let mut broker_up = false;
    // `start` first: it puts the outputs in a known state and sends the first
    // heartbeats -- the power report and the alive pulse -- which are what
    // credit this device with being alive on the local broker.
    apply(reader.start(now()), &mut panel, &mut client, &mut relay);
    apply(
        reader.health(health(&address, broker_up), now()),
        &mut panel,
        &mut client,
        &mut relay,
    );

    loop {
        // One look at the antenna, which is the thing with a person waiting on
        // it, before anything else.
        if let Some(uid) = card_reader.as_mut().and_then(|r| r.poll()) {
            apply(
                reader.card(&uid, now()),
                &mut panel,
                &mut client,
                &mut relay,
            );
        }

        // Then the machine. Sampled every pass rather than on a change, which
        // the filter in the core is built for: it only moves its clock when the
        // value actually changes, so handing it every reading costs nothing and
        // means no edge can be missed between polls.
        if let Some(pin) = sense.as_mut() {
            // The ONE inversion in the whole firmware. `Reader::running` takes
            // a semantic bool -- "the machine is working" -- so the sense of
            // this installation's wiring is applied here and nowhere else.
            let working = pin.is_low() == sense_active_low;
            apply(
                reader.running(working, now()),
                &mut panel,
                &mut client,
                &mut relay,
            );
        }

        while let Ok(event) = rx.try_recv() {
            let actions = match event {
                broker::Event::Connected => {
                    // Every connect, not once: a clean session means the broker
                    // has forgotten us, and a client that subscribes only at
                    // startup is deaf after its first reconnect.
                    broker::resubscribe(&mut client);
                    light.set(Status::Online);
                    broker_up = true;
                    reader.health(health(&address, broker_up), now())
                }
                broker::Event::Disconnected => {
                    light.set(Status::BrokerLost);
                    broker_up = false;
                    reader.health(health(&address, broker_up), now())
                }
                broker::Event::Message { topic, payload } => {
                    reader.message(&topic, &payload, now())
                }
            };
            apply(actions, &mut panel, &mut client, &mut relay);
        }

        apply(reader.tick(now()), &mut panel, &mut client, &mut relay);
        std::thread::sleep(POLL);
    }
}

/// Carry out what the core decided.
fn apply(
    actions: Vec<Action>,
    panel: &mut Option<lcd::Panel<'_>>,
    client: &mut esp_idf_svc::mqtt::client::EspMqttClient<'static>,
    relay: &mut Option<PinDriver<'_, Output>>,
) {
    use esp_idf_svc::mqtt::client::QoS;
    for action in actions {
        match action {
            Action::Publish { topic, payload } => {
                // `enqueue`, never `publish`: publish can block for up to ten
                // seconds and this is the thread that must keep polling the
                // antenna.
                if let Err(e) = client.enqueue(topic, QoS::AtLeastOnce, false, payload.as_bytes()) {
                    log::warn!("could not enqueue to {topic}: {e}");
                }
            }
            Action::Show(screen) => {
                if let Some(p) = panel.as_mut() {
                    if let Err(e) = p.show(&screen) {
                        // Not fatal and not silent: the panel may recover on the
                        // next change, and the log says that it did not now.
                        log::warn!("panel write failed: {e:#}");
                    }
                }
            }
            Action::Note(note) => log::info!("{note}"),
            Action::Set { output, on } => {
                log::info!("{output:?} {}", if on { "ON" } else { "off" });
                // Only the tool output reaches hardware here. `Output::Running`
                // is an indicator and this box has no lamp for it -- the panel
                // is what tells a member the machine is working, and the status
                // LED is already spoken for by the network state.
                if output == toolguard_core::Output::Tool {
                    match relay.as_mut() {
                        Some(pin) => {
                            let r = if on { pin.set_high() } else { pin.set_low() };
                            if let Err(e) = r {
                                // Loud: this is the output a machine's power
                                // depends on, and a write that failed means the
                                // relay is not where the core believes it is.
                                log::error!("relay write failed: {e:#}");
                            }
                        }
                        // Unreachable by construction -- the core emits no Tool
                        // action unless `gates_power` is set, and the relay is
                        // opened whenever it is. Logged rather than ignored
                        // because if it ever happens, the thing that did not
                        // happen is a machine switching off.
                        None => log::error!("asked to switch the tool with no relay configured"),
                    }
                }
            }
        }
    }
}

/// What the core is told about the network.
fn health(address: &str, broker: bool) -> Health {
    Health {
        // `boot()` does not return until Wi-Fi is joined, so by here it is.
        wifi: true,
        broker,
        address: Some(address.to_string()),
        // No SNTP yet, deliberately. The platform's tool path is measured on a
        // monotonic timer by design and nothing in the protocol carries a
        // usable wall clock, so a clock is an LCD nicety rather than a
        // prerequisite — and it stays out until it can be added without ever
        // reaching a decision.
        clock: None,
    }
}
