# spaces-toolguard-firmware

A generic badge reader for [Cooperative Systems: Spaces][css], the
access-control platform at the [Cambridge Hackspace][hackspace]: scan a card,
ask the edge, and tell the person standing there what the answer was on a 16x2
LCD.

> Work in progress. The protocol core and the display model are host-tested; the
> firmware compiles for all three chips in CI. **No board has run any of it** —
> the pin maps for the C3 and S3 in particular compile and nothing more, since
> neither board exists here yet.

## What it is, and is not

A reader **asks** and **reports**. Energizing a machine is a different device's
job — a module bound to the same tool in the `power` role — and this firmware
deliberately has no relay output. That is also why it does not build on
`spaces_device::Module`: that type is lease-gated, and a reader is never bound
in the `power` role, so it is never leased. Every valid badge would have died
five seconds later in `NoFirstLease`.

## Layout

| Path | What it is |
|---|---|
| `toolguard-core/` | the scan state machine and the display model: no hardware, no I/O, host-tested |
| `toolguard-fw/` | the ESP-IDF firmware: RC522 over SPI, 1602 over I2C, three chips |

The protocol is not redefined here. Topics, `ToolRequest` and `ToolOnReply` come
from [`spaces-firmware`][sf]'s `spaces-device`, pinned to tag `v0.1.0`, so this
reader and the power modules speak the same wire by construction.

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

Seven pins always: SPI (SCLK, MOSI, MISO, CS) plus RST for the RC522, and
SDA/SCL for the panel. Two more are used only by the roles that need them —
a **sense** input for a box that watches the machine, and a **relay** output for
one that switches it. The assignment is a `cfg`-gated tuple in
`toolguard-fw/src/main.rs`, one arm per chip, and the reasoning is in
`toolguard-fw/src/board.rs`.

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
