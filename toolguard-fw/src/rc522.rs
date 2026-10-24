//! The MFRC522 front end: probe it, then poll it for card UIDs.
//!
//! ## Provenance
//!
//! Adapted from `Cambridge-Hackspace/nfc-522`'s `src/nfc.rs` by
//! gmorell <gmp@gmp.io>, which is **GPL-3.0**. This file is therefore the one
//! place in this AGPL-3.0-or-later repository with a second copyright; AGPLv3
//! §13 permits the combination, and the GPL-licensed portion keeps its own
//! terms. What is taken is the shape rather than the text: probe by version
//! register, REQA/HLTA to make a resting card read once, and a same-UID
//! cooldown.
//!
//! That firmware is a good hardware reference and not a protocol one — it
//! publishes to `{topic_root}/scans/<channel>` and speaks no `toolguard/*` at
//! all — so only this layer is borrowed.
//!
//! ## Two deliberate departures
//!
//! **Mode 3, not mode 0.** `nfc-522` configures `SpiConfig::new().baudrate(..)`
//! and takes the default mode 0. That works on the C3 it targets and is the
//! documented failure case on the S3, where ESP-IDF samples MISO on the wrong
//! edge and every register read is corrupted while writes still land. See
//! [`crate::board::SPI_MODE_3`].
//!
//! **The cooldown lives in the core, not here.** `nfc-522` debounces in the
//! driver; this crate keeps the decision in `toolguard_core::reader`, where it
//! is testable without hardware, and leaves the driver to report what the
//! antenna saw. The REQA/HLTA halt stays here, because it is a property of the
//! card and not of the policy.

use anyhow::Context;
use esp_idf_svc::hal::gpio::{InputPin, Output, OutputPin, PinDriver};
use esp_idf_svc::hal::peripheral::Peripheral;
use esp_idf_svc::hal::spi::config::{Config as SpiConfig, DriverConfig};
use esp_idf_svc::hal::spi::{SpiAnyPins, SpiDeviceDriver, SpiDriver};
use esp_idf_svc::hal::units::Hertz;
use mfrc522::comm::blocking::spi::{DummyDelay, SpiInterface};
use mfrc522::{Initialized, Mfrc522};

use crate::board;

/// A reader that has answered with a plausible version byte.
pub struct Reader<'d, RST: OutputPin> {
    mfrc522: Mfrc522<SpiInterface<SpiDeviceDriver<'d, SpiDriver<'d>>, DummyDelay>, Initialized>,
    /// Held for the program's lifetime: dropping it would let RST fall and put
    /// the module back into power-down.
    _rst: PinDriver<'d, RST, Output>,
}

impl<RST: OutputPin> Reader<'_, RST> {
    /// Poll once. `Some(uid)` only for a card that has just entered the field.
    ///
    /// REQA is answered only by cards in IDLE, so a card that was read and then
    /// halted stays silent until it leaves and comes back. That is what makes a
    /// card resting on the reader one event rather than a stream of them — the
    /// policy-level cooldown in the core is the second guard, for a tap that
    /// bounces.
    pub fn poll(&mut self) -> Option<String> {
        let atqa = self.mfrc522.reqa().ok()?;
        let uid = self.mfrc522.select(&atqa).ok()?;
        // Halt it and drop any crypto state, so the next presentation is a new
        // detection rather than a continuation.
        let _ = self.mfrc522.hlta();
        let _ = self.mfrc522.stop_crypto1();
        Some(to_hex(uid.as_bytes()))
    }
}

/// Bring up the SPI bus and the reader on it, or say why not.
///
/// Returns an error rather than `None` on every failure path: a reader that
/// cannot be reached is the whole function of this device missing, and the
/// caller shows it on the panel instead of idling as though it were waiting for
/// a card.
pub fn start<'d, SPI: SpiAnyPins + 'd, RST: OutputPin>(
    spi: impl Peripheral<P = SPI> + 'd,
    sclk: impl Peripheral<P = impl OutputPin> + 'd,
    mosi: impl Peripheral<P = impl OutputPin> + 'd,
    miso: impl Peripheral<P = impl InputPin> + 'd,
    cs: impl Peripheral<P = impl OutputPin> + 'd,
    rst: impl Peripheral<P = RST> + 'd,
) -> anyhow::Result<Reader<'d, RST>> {
    // RST high FIRST and kept: the MFRC522 is in power-down until it is, and a
    // bus brought up against a sleeping chip reads zeros that look like a wiring
    // fault.
    let mut rst = PinDriver::output(rst).context("RC522 reset pin")?;
    rst.set_high().context("RC522 reset high")?;

    let driver = SpiDriver::new(spi, sclk, mosi, Some(miso), &DriverConfig::new())
        .context("SPI bus for the RC522")?;

    // Mode 3 explicitly. See board::SPI_MODE_3 -- mode 0 is the documented S3
    // corruption and the default if nothing says otherwise.
    let config = SpiConfig::new()
        .baudrate(Hertz(board::SPI_HZ))
        .data_mode(board::SPI_MODE);
    let device =
        SpiDeviceDriver::new(driver, Some(cs), &config).context("SPI device for the RC522")?;

    let mut mfrc522 = Mfrc522::new(SpiInterface::new(device))
        .init()
        .map_err(|e| anyhow::anyhow!("MFRC522 init failed: {e:?}"))?;

    // THE ORACLE FOR THE WHOLE SPI PATH, and it is cheap. `VersionReg` is the
    // exact register the S3 mode-0 bug corrupts: a correct bus reads 0x91 or
    // 0x92, and a wrongly-sampled one reads 0x6e or similar. Checking it here
    // turns "the reader never sees a card" -- which looks like an antenna, a
    // wiring or a card problem -- into one line at boot naming the bus.
    let version = mfrc522
        .version()
        .map_err(|e| anyhow::anyhow!("could not read the MFRC522 version register: {e:?}"))?;
    match version {
        0x91 | 0x92 => log::info!("MFRC522 present, version {version:#04x}"),
        0x00 | 0xFF => anyhow::bail!(
            "no MFRC522 answered (version {version:#04x}): nothing on the bus, or RST/CS miswired"
        ),
        other => anyhow::bail!(
            "MFRC522 version {other:#04x} is not 0x91 or 0x92. The chip answered, so the wiring \
             is live and the BUS SETTINGS are wrong -- on an ESP32-S3 in SPI mode 0 this register \
             is the one that comes back corrupted while writes still appear to work"
        ),
    }

    Ok(Reader { mfrc522, _rst: rst })
}

/// Uppercase hex, which is how the platform's own examples write a card id
/// (`"card_id": "04A1B2C3"`).
///
/// The UID is sent onward EXACTLY as read: `FIRMWARE.md` is explicit that a
/// card value must not be trimmed, case-folded or interpreted, because what
/// counts as "the card" is a server-side setting (`toolguard.profile_field`)
/// and may not be a UID at all.
fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02X}");
    }
    s
}
