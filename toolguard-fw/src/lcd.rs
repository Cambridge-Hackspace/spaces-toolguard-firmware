//! The 1602 panel, behind a PCF8574 I2C backpack.
//!
//! The content is decided in `toolguard_core::screen`; this is only the wire.
//! What it adds is the two things that make a character LCD work on a board
//! nobody has seen yet: finding the backpack, and not writing to it more than
//! it can take.
//!
//! ## Finding it
//!
//! The address is **scanned**, not configured. A PCF8574T answers in
//! `0x20..=0x27` and a PCF8574**A**T in `0x38..=0x3F`; the parts look the same
//! and vendors ship whichever. A hardcoded `0x27` is a reader that works on one
//! board and is blank on the next, with nothing in the log to say why — so the
//! scan happens at boot and the address it found is logged.
//!
//! ## Not over-writing it
//!
//! A PCF8574 in 4-bit mode costs **three I2C transactions per nibble**: set the
//! data with E high, drop E, and the backlight bit rides along. A full 16x2
//! repaint is therefore on the order of a hundred transactions at 100 kHz, and
//! repainting every poll would be the entire I2C budget spent re-displaying the
//! same characters. The core only emits `Action::Show` when the content
//! actually changes, and this module writes only what it is given.

use anyhow::{anyhow, Context};
use esp_idf_svc::hal::delay::Delay;
use esp_idf_svc::hal::gpio::{InputPin, OutputPin};
use esp_idf_svc::hal::i2c::{I2c, I2cConfig, I2cDriver};
use esp_idf_svc::hal::units::FromValueType as _;
use i2c_character_display::{CharacterDisplayPCF8574T, LcdDisplayType};
use toolguard_core::Screen;

/// The panel, once something has answered on the bus.
pub struct Panel<'d> {
    lcd: CharacterDisplayPCF8574T<I2cDriver<'d>, Delay>,
}

impl Panel<'_> {
    /// Put a screen up. Both rows, every time.
    ///
    /// Rewriting both rows rather than diffing them: a 1602 has no way to be
    /// asked what it is currently showing, so a diff would be against this
    /// firmware's *belief*, and the one time that belief is wrong — a glitched
    /// transaction, a brown-out on the backpack — is exactly when the panel is
    /// stuck showing something stale and a diff would never correct it. The
    /// change detection that matters already happened in the core.
    pub fn show(&mut self, screen: &Screen) -> anyhow::Result<()> {
        self.lcd
            .clear()
            .map_err(|e| anyhow!("clearing the panel: {e:?}"))?;
        for (row, line) in screen.rows().enumerate() {
            self.lcd
                .set_cursor(0, row as u8)
                .map_err(|e| anyhow!("moving to row {row}: {e:?}"))?;
            self.lcd
                .print(line)
                .map_err(|e| anyhow!("writing row {row}: {e:?}"))?;
        }
        Ok(())
    }
}

/// Bring the bus up, find the backpack, and initialise the panel.
pub fn start<'d, I2C: I2c + 'd>(
    i2c: I2C,
    sda: impl InputPin + OutputPin + 'd,
    scl: impl InputPin + OutputPin + 'd,
    addresses: &[u8],
) -> anyhow::Result<Panel<'d>> {
    // 100 kHz: the PCF8574 is a standard-mode part and the HD44780 behind it is
    // slower still. Internal pull-ups on, because a cheap backpack may not
    // populate them and a floating bus reads as "nothing is there".
    let config = I2cConfig::new()
        .baudrate(100_u32.kHz().into())
        .sda_enable_pullup(true)
        .scl_enable_pullup(true);
    let mut driver = I2cDriver::new(i2c, sda, scl, &config).context("I2C bus for the panel")?;

    let found = scan(&mut driver, addresses).ok_or_else(|| {
        anyhow!(
            "no I2C device answered at any of {addresses:02X?}. A PCF8574T is 0x20-0x27 and a \
             PCF8574AT is 0x38-0x3F; check SDA/SCL are not swapped, and that the backpack has \
             power -- its LCD rail is usually 5V even where the bus is 3.3V"
        )
    })?;
    log::info!("LCD backpack answered at {found:#04x}");

    let mut lcd =
        CharacterDisplayPCF8574T::new(driver, LcdDisplayType::Lcd16x2, Delay::new_default());
    lcd.init()
        .map_err(|e| anyhow!("initialising the 1602 at {found:#04x}: {e:?}"))?;
    lcd.backlight(true)
        .map_err(|e| anyhow!("backlight on: {e:?}"))?;

    Ok(Panel { lcd })
}

/// The first address that acknowledges a zero-length write.
///
/// A bare address probe, which is what an I2C scan is: the device ACKs its
/// address or it does not, and nothing is read or written. Harmless to try
/// against an address with nothing on it.
fn scan(driver: &mut I2cDriver<'_>, addresses: &[u8]) -> Option<u8> {
    for &addr in addresses {
        if driver.write(addr, &[], 100).is_ok() {
            return Some(addr);
        }
    }
    None
}
