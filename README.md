# spaces-toolguard-firmware

A generic tool box for [Cooperative Systems: Spaces][css], the access-control
platform at the [Cambridge Hackspace][hackspace]. One image, configured at
setup into whichever of these a machine needs: a badge reader that asks the
edge and shows the answer on a 16x2 LCD; a reader that also switches the
machine; a reader that watches it work and meters the time; a plug with no
reader at all; and, on any of them, a stop button anyone may press.

> Work in progress. The protocol core and the display model are host-tested; the
> firmware compiles for all three chips in CI. **No board has run any of it** —
> the pin maps for the C3 and S3 in particular compile and nothing more, since
> neither board exists here yet.

## What it is, and is not

The box **asks**, **reports**, and — only if it is configured to switch the
machine — **energizes**, and then only on an explicit yes *and* an unexpired
lease from the edge. The protocol, the session, the lease and the metering are
`spaces_device::Module` from [`spaces-firmware`][sf]; this crate adds the
panel, the card reader, the relay, the sense input, the stop button, and the
setup page that says which of those are wired. The roles are independent and
are settings, not builds: a box registers as whatever it is configured to be,
and the platform binds it to a tool in those roles.

It is **access control and metering, not a safety device.** It must never
replace a door interlock, a guard, or an emergency stop — the stop button
included, which goes through this firmware, a lease and a Wi-Fi link.
[`docs/INSTALLING.md`](docs/INSTALLING.md) opens with why.

## Layout

| Path | What it is |
|---|---|
| `toolguard-core/` | the glue over `spaces_device::Module`, the display model and the stop-button debouncer: no hardware, no I/O, host-tested |
| `toolguard-fw/` | the ESP-IDF firmware: RC522 over SPI, 1602 over I2C, three chips |

The protocol is not redefined here. `Module`, the topics and the wire types come
from [`spaces-firmware`][sf]'s `spaces-device`, pinned by tag in `Cargo.toml`
(currently `v0.3.1`), so this box and every other module speak the same wire by
construction.

## The panel is sixteen columns, and that is a design constraint

`toolguard-core/src/screen.rs` decides what a 1602 shows, as a pure function of
state, so "does this outcome fit" is a host test rather than something learned
one reflash at a time.

**A known rough edge, found by exactly that test:** `Training required` is 17
characters — the platform's commonest refusal — so it reaches the panel as
`Training require`. The full reason is in the log. Shortening it in the firmware
would mean matching on reason text, which `FIRMWARE.md` warns against ("branch
on the boolean, display the string"), so the truncation stands until there is a
better answer than a lookup table that silently stops matching.

The diagnostic screen is a first-class view rather than an error case: a reader
whose edge is unreachable is the situation a member is most likely to be
standing in front of, so the panel says **which half** is missing and shows the
device's address, which is where `/log` lives.

## Building

The core is plain Rust and runs anywhere. `--target` is required because
`.cargo/config.toml` makes an ESP target the default:

```sh
cargo test -p toolguard-core --target x86_64-unknown-linux-gnu
```

The firmware needs `cargo install ldproxy` and, for the two RISC-V chips, the
nightly pinned in `rust-toolchain.toml`. **`MCU` is not optional** — a target
triple alone is ambiguous (`riscv32imc-esp-espidf` maps to the C3, C2, H2 *and*
C5, and esp-idf-sys picks the first it supports, warning as it goes):

```sh
MCU=esp32c6 cargo build --release --target riscv32imac-esp-espidf
MCU=esp32c3 cargo build --release --target riscv32imc-esp-espidf
```

The **S3 is Xtensa**, which upstream rustc cannot target at all, so it needs
Espressif's compiler fork via espup — pin `0.17.1`, since `0.18.0` is yanked:

```sh
cargo install espup --locked --version 0.17.1
espup install --targets esp32s3 --std
. $HOME/export-esp.sh
MCU=esp32s3 cargo +esp build --release --target xtensa-esp32s3-espidf
```

That combination is measured rather than assumed: CI builds all three, and the
`esp` channel does serve the Xtensa target while `rust-toolchain.toml` keeps the
RISC-V pair on plain nightly, so the C6 and C3 do not pay for a forked compiler.

## Wiring

**If you are installing one of these on a machine, read
[`docs/INSTALLING.md`](docs/INSTALLING.md) instead of this section.** It covers
the safety, the three things you have to measure on your own machine before
wiring anything, both wiring modes, commissioning in order, and what to do when
it does not work. This section is the summary.

Seven pins always: SPI (SCLK, MOSI, MISO, CS) plus RST for the RC522, and
SDA/SCL for the panel. Three more are used only by the roles that need them —
a **sense** input for a box that watches the machine, a **relay** output for one
that switches it, and a **stop** input for a momentary switch to GND that ends
the session when anyone presses it. The assignment is a `cfg`-gated tuple in
`toolguard-fw/src/main.rs`, one arm per chip, the reasoning is in
`toolguard-fw/src/board.rs`, and the table in `docs/INSTALLING.md` is checked
against the tuple by a test.

**What the box is, is configuration and not a build.** One image is flashed and
the same board becomes a badge reader beside a laser, a reader that also
switches it, a reader that watches it work, or a plug with no reader at all. The
settings are on the device's own setup page and are read at boot; see
`toolguard-fw/src/settings.rs` for each one, its default, and what going wrong
looks like. Two are worth knowing before wiring anything:

- **"Senses the machine"** must be `no` unless something really is wired to the
  sense input. A device that reports zero working time is believed, and on a
  tool billed by working time that bills nothing.
- **"Switches the tool"** must be `yes` only if a relay is wired, because it is
  also what makes this device register in the `power` role — and the platform
  will not lease a tool to a box that claims to switch it and does not.

Two electrical notes worth having before wiring anything:

- **The RC522 is 3.3V only.** Its absolute maximum VCC is 3.6V, so power it from
  3V3 and never from 5V. With a 3.3V MCU this is the easy peripheral.
- **The 1602 is the awkward one.** Its HD44780 bias wants ~5V for readable
  contrast, but a backpack powered at 5V idles SDA/SCL at 5V through its own
  pull-ups, and ESP32 GPIOs are not 5V tolerant. The clean fix is to power the
  backpack from 5V and move its pull-ups to 3V3 — one desolder — rather than
  relying on it working anyway. A "dead" 1602 is also very often just its
  contrast trimpot.

## License

[AGPL-3.0-or-later](LICENSE), matching the platform and `spaces-firmware`.

[css]: https://github.com/Cambridge-Hackspace/cooperative-systems-spaces
[hackspace]: https://cambridgehackspace.com
[sf]: https://github.com/Cambridge-Hackspace/spaces-firmware
