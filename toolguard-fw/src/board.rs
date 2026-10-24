//! Which pin is which, per chip, and the two settings that are not the same
//! on all three.
//!
//! Everything chip-specific lives here so the rest of the firmware never names
//! a GPIO. `spaces-firmware` hardcodes the C6 in five independent places — the
//! triple, `MCU`, two espflash flags, the partition table's arithmetic — and
//! retargeting it means finding all five; this crate keeps the pins in one
//! module and the chip in `.cargo/config.toml`.
//!
//! ## Why these pins
//!
//! On an ESP32-C6 Super Mini, four pins are effectively unavailable and one of
//! them is a trap:
//!
//! - **GPIO8** carries the board's RGB LED *and* is a strapping pin that
//!   download mode needs pulled HIGH. Espressif: "The strapping combination of
//!   GPIO8 = 0 and GPIO9 = 0 is invalid and will trigger unexpected behavior."
//!   An external peripheral here fights both.
//! - **GPIO9** is BOOT, also strapping, and is the hold-for-setup gesture.
//! - **GPIO12/13** are native USB. The Super Mini has no USB-serial chip, so
//!   these are the only console there is; losing them loses flashing and logs.
//! - **GPIO4/5/6/7** are strapping, JTAG and flash-adjacent.
//!
//! That leaves IO0–IO3, IO14 and IO18–IO23 freely usable, which is more than
//! the seven this needs. The C6's GPIO matrix routes SPI2 and I2C to almost any
//! pin, and ESP-IDF is explicit that below 40 MHz routing through the matrix is
//! no worse than the IOMUX pins — so at 1 MHz this is a pin-budget question and
//! not a capability one.

//! ## Where the pins actually are
//!
//! Not here, and that is forced rather than chosen: `esp-idf-hal` takes pins as
//! CONCRETE types behind generic bounds (`impl Peripheral<P = impl InputPin +
//! OutputPin>`), which is also how `spaces-device-espidf` takes them. A runtime
//! struct of GPIO numbers would have to be erased to `AnyIOPin` to fit in one,
//! losing exactly the compile-time check that stops a pin being used twice.
//!
//! So the assignment lives in `main.rs` as a `cfg`-gated tuple, one arm per
//! chip, and this module holds the reasoning plus the settings that are not
//! pins. The pin table is in this file's docs above; the compiler enforces it
//! in `main.rs`.

/// SPI clock for the RC522, in Hz.
///
/// 1 MHz, well under what the MFRC522 tolerates, chosen for wiring slack: this
/// is a breadboard-and-jumpers peripheral and the data rate is irrelevant when
/// the payload is a card UID.
pub const SPI_HZ: u32 = 1_000_000;

/// The I2C addresses a PCF8574 LCD backpack might answer on.
///
/// The bus is scanned for these at startup rather than one being hardcoded: the
/// PCF8574T answers in `0x20..=0x27` and the PCF8574**A**T in `0x38..=0x3F`,
/// the two parts are visually near-identical, and vendors ship both. A reader
/// that works on one board and shows nothing on the next, for that reason, is a
/// bad afternoon.
pub const LCD_ADDRESSES: &[u8] = &[0x27, 0x3F, 0x20, 0x38];

/// SPI mode for the RC522.
///
/// **Mode 3, and this is not a preference.** ESP-IDF's SPI master on the
/// ESP32-S3 samples MISO on the wrong edge in mode 0, so every register read
/// comes back corrupted while writes still land: the chip resets, polling
/// appears to run, `VersionReg` reads `0x6e` instead of `0x92`, and no tag is
/// ever detected. It presents as "the reader does not work" with nothing in the
/// log (esphome#19065; espressif/esp-idf#7825 and #8815).
///
/// Mode 3 also works on the RISC-V parts, so one value serves all three rather
/// than a per-chip constant nobody can test. The `version()` check at startup
/// is the oracle for this: it reads the exact register the bug corrupts.
///
/// A constant the driver actually passes, not a note: a `bool` saying "use mode
/// 3" would be a second place to disagree with the code.
pub const SPI_MODE: embedded_hal::spi::Mode = embedded_hal::spi::MODE_3;

/// The chip this build is for, for the boot banner and the device record.
pub const CHIP: &str = if cfg!(esp32c6) {
    "esp32c6"
} else if cfg!(esp32c3) {
    "esp32c3"
} else if cfg!(esp32s3) {
    "esp32s3"
} else {
    "unknown"
};
