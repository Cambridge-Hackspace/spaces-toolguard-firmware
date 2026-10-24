# spaces-toolguard-firmware

A generic badge reader for [Cooperative Systems: Spaces][css], the
access-control platform at the [Cambridge Hackspace][hackspace]: scan a card,
ask the edge, and tell the person standing there what the answer was on a 16x2
LCD.

> Work in progress. The protocol core and the display model are here and
> host-tested; the ESP-IDF firmware is next, and no board has run any of it yet.

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
| `toolguard-fw/` | the ESP-IDF firmware *(not yet written)* |

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

```sh
cargo test -p toolguard-core --target x86_64-unknown-linux-gnu
```

The core is plain Rust and runs anywhere. `--target` is required because
`.cargo/config.toml` will set an ESP target as the default once the firmware
crate lands.

## License

[AGPL-3.0-or-later](LICENSE), matching the platform and `spaces-firmware`.

[css]: https://github.com/Cambridge-Hackspace/cooperative-systems-spaces
[hackspace]: https://cambridgehackspace.com
[sf]: https://github.com/Cambridge-Hackspace/spaces-firmware
