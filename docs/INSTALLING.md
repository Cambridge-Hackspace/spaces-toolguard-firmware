# Installing a ToolGuard

For the person wiring one of these to a machine. You should not have to read any
source to follow this; where something is genuinely unknown, it says so rather
than guessing.

If you are writing firmware that talks to the platform, you want
[`FIRMWARE.md`][fw] in the `cooperative-systems-spaces` repository instead. This
is the other document.

---

## 1. Safety

**Read this part even if you skip the rest.**

### The laser supply stays dangerous after power-off

A CO₂ laser power supply holds charge in its capacitors after the machine is
switched off and unplugged. Treat the HV side as live regardless of what the
switch says. Do not open the PSU enclosure. Everything in this guide is done on
the **control** side — the low-voltage signal connector on the DSP controller —
and nothing in it requires you to go near the high-voltage side.

### This device is not a safety interlock

It is **access control and metering**. It decides whether a member is allowed to
start a machine, and it records how long the machine ran.

It is **not** a personnel-safety device and must never replace or bypass one:

- a door or lid interlock switch
- an emergency stop
- a key switch, a guard, or a light curtain

If removing this box would defeat an existing safety function, the box has been
wired into the wrong place. The platform has a separate mechanism for safety
interlocks, with its own enforcement tiers, and it is not this.

### How it fails

Worth knowing before you choose a wiring mode, because the answers differ.

| what happens | box switches the tool | box does not switch the tool |
|---|---|---|
| box loses power | relay opens, tool stops | nothing stops; the tool was never gated by this box |
| Wi-Fi drops | relay opens when the lease runs out (seconds) | the session stays open until it times out |
| broker/edge down | relay opens when the lease runs out | the panel shows `No edge`; cards are refused locally and not sent |
| member walks away | relay opens at the lease or the session cap | the session cap is the only thing that ends it — **set one** |
| this box is removed | the tool cannot be energized | the tool is completely unguarded |

The last row is the important one. A box that does not switch the tool makes
access control **advisory**: it tells a member no, and nothing stops them
proceeding anyway. That is a legitimate configuration — it is how you meter a
machine you cannot interrupt — but be clear which one you have installed.

### A latch is not an egress or fire-safety device

A box that drives a door's latch (§4d) is access control for coming **in**. It
must never be the only way **out**: free egress — a handle, a push bar, a
request-to-exit button that cuts the latch's supply directly — is a matter of
building code and of people, and it does not go through this firmware, its
Wi-Fi link or its edge. Likewise, a latch that must release on a fire alarm
does so through the alarm panel's own relay in the latch's supply, not through
anything here. How the latch fails when this box loses power is the latch's
own property (§4d), and you choose it when you buy the hardware.

---

## 2. Measure before you wire

Three things are not published anywhere and must be measured on your machine.
None takes long and getting them wrong is expensive.

### a. The voltage on the signal

Put a meter between the signal and its ground, with the controller powered and
the machine idle, then again mid-job. Expect **5 V**. Some controllers and most
PLC-style outputs are **24 V**.

**Either level destroys an ESP32 input**, which is 3.3 V and not 5 V tolerant.
This is why §4 isolates the input rather than wiring it straight to a pin.

### b. How much current the output can sink

The laser controller's enable output is open-collector: it pulls the line to
ground and the load sits between the line and a positive supply. **No Ruida
manual publishes the sink limit** — the three official PDFs for the RDC644xG
family are scanned images and none states it.

That matters because this line is already driving the laser PSU's enable input.
Anything you add is extra load on an output whose budget you do not know, and
the failure mode is the laser not firing when it should.

So: prefer a **high-impedance** sense (a comparator or an opto driven through a
large resistor, microamps) over anything that draws milliamps. If you must draw
real current, measure the line's behaviour with your addition fitted, firing the
laser, before you trust it.

### c. Whether the signal holds or chatters

**Fire a job and watch the line.** On a Ruida controller the laser-enable signal
drops on **every scan-line return** — low while a pass cuts, high while the head
overshoots and steps in Y. On a raster engrave that is hundreds of transitions a
minute.

This sets **"Bridge gaps (seconds)"** in §6, and it is the difference between
metering one engrave and metering three hundred fragments. If your machine's
signal instead stays asserted for a whole job, you can set the bridge to 0 and
measure beam-on time directly.

A meter with a min/max hold will show you the chatter; a scope or a logic
analyser will show you its rate, which is what you want for choosing the bridge.

---

## 3. Finding the signal on a Ruida controller

On an RDC644xG-family controller (the RDC6442S-B(EC) among them) the laser
control connector is **CN5**:

| pin | name | what it is |
|---|---|---|
| 1 | `GND` | signal ground |
| 2 | `L-ON1` | laser enable — the signal you want |
| — | `L-AN1` / `L-PWM1` | laser **power** (analog or PWM). Not this. |

`L-ON1` is normally wired to the laser PSU's enable terminal. You are tapping
it, not interrupting it, unless you are installing in gating mode (§4b).

### Check the polarity on your machine, do not assume it

`L-ON1` is **active low by default** — it pulls to ground to enable the laser —
but that is a **machine-level setting**, not a property of the controller:

> **Edit → Machine Settings → Vendor Settings → "Laser Output Signal"**

in RDWorks or LightBurn. It can be either way round, and installations differ.
Read it, write it down, and set **"Sense is active low"** in §6 to match.

Getting this backwards in watch mode means usage is recorded exactly when the
machine is *not* working. In gating mode it means the box enables the laser when
it means to disable it — which is why §4b asks you to test it with the tube
disconnected.

---

## 4. Wiring

Two modes. Pick one before you start; the box's configuration must match.

### a. Watch mode — the box observes and cannot interfere

```
  Ruida CN5
  ┌──────────────┐
  │ 2  L-ON1  ───┼──┬────────────────────► laser PSU enable  (UNTOUCHED)
  │              │  │
  │              │  └──[ isolated, high-Z sense ]──► ToolGuard sense pin
  │ 1  GND    ───┼─────────────────────────────────► isolator ground
  └──────────────┘
```

Nothing is in series with the laser's enable, so no failure of this box can
stop — or start — the machine. This is the mode to use when you are metering a
tool you must not interrupt, and the mode to start with while you are learning
what the signal does.

### b. Gating mode — the box is in series with the enable

```
  Ruida CN5
  ┌──────────────┐
  │ 2  L-ON1  ───┼──┬──[ isolated sense ]──► ToolGuard sense pin
  │              │  │
  │              │  └──[ ToolGuard relay ]──► laser PSU enable
  │ 1  GND    ───┼─────────────────────────► isolator ground
  └──────────────┘
```

The relay must be wired so that **de-energized = laser disabled**. Then a box
that loses power, crashes, or loses its lease fails to the safe state without
having to do anything.

**Test it with the laser tube disconnected first.** Confirm that an
unauthorized card leaves the enable line in the disabled state and an authorized
one releases it, with a meter on the line, before the tube is connected.

### c. The stop button — optional, and anyone may press it

A momentary, normally-open switch between the **stop** pin and **GND**. No
resistor: the pin uses the chip's internal pull-up, so an open contact — or no
button at all — reads as "not pressed". Turn on "Has a stop button" in §6.

Pressing it **ends the session**: usage reported, the tool released on the
platform, and in gating mode the relay opens within one poll (under 100 ms). It
needs no card — and that holds on a **plug** too (a box with no card reader):
its stop names the tool alone, the platform ends whatever session is open
there, and the plug switches off when its lease is withdrawn. That is deliberate and matches the platform's rule — *anyone may
cut the power* — so a member's colleague, or a passer-by who sees something
wrong, can stop the job without finding the member. The session is still closed
under the card that opened it, because that is who the usage belongs to.

It is **not an emergency stop** (§1). In watch mode nothing physical happens —
the box observes and cannot interfere — and in gating mode the relay is a
software path through this firmware, its lease and a Wi-Fi link. A machine that
needs a stop that works when all of that is dead needs a real E-stop in series
with its own supply, and this button beside it, not instead of it.

### d. A door's latch — optional, and the box then registers as a `strike`

Turn on "Drives a latch" in §6. On a yes, the **strike** pin is driven **high for
exactly the pulse the platform names** — the resource's strike binding's
`pulse_ms`, set under Resource Wiring — and then low. The box does not choose
the duration, and a yes that names no pulse releases nothing and says so in the
log: bind a strike to the resource, or it has no latch as far as the platform
knows.

The pin is a 3.3 V logic output and must drive a **latch driver**, never the
latch: a MOSFET or a relay module with its own supply, with a flyback diode
across any coil. Two kinds of latch, and the difference is what happens when
this box loses power:

- an **electric strike** is usually *fail-secure*: energize to release, and it
  stays locked with no power. Drive it directly through the driver.
- a **maglock** is *fail-safe*: energized to hold, released without power. It
  needs an inverting driver (the box's pulse must *cut* the supply), and a box
  rebooting then opens the door for as long as it reboots.

Both are legitimate; know which you have, and see §1 on egress. The box's own
behaviour is the same for both: low at boot, high for the pulse, low after.

A box that drives a latch and switches nothing is a **door box**: its session
is the pulse, the panel reads `Door open` while the latch is released and
`Present card` after, and nothing is signed into. Its stop button (§4c) still
works and is how a **room** is closed — a door the platform holds open until
somebody stops it, with the room's power bindings leased meanwhile — because
the stop names the resource and the platform ends whatever session is open on
it. A box that drives a latch **and** switches the tool is a room in one box:
the latch pulses on the yes, the relay follows its lease as in gating mode.

### Isolation is not optional

- **5 V (or 24 V) must never reach an ESP32 pin.** They are 3.3 V and not
  tolerant.
- An optocoupler also breaks the **ground loop** between the controller's supply
  and this box's. That matters next to a switching HV supply, where a shared
  ground carries noise that a logic input will happily interpret as the machine
  working.
- Prefer high impedance, per §2b. A PC817-style opto fed through a resistor
  sized for **microamps, not milliamps**, or a comparator.

### Other peripherals

- **The RC522 is 3.3 V only.** Absolute maximum VCC is 3.6 V. Power it from 3V3,
  never 5 V.
- **The 1602 is the awkward one.** Its HD44780 bias wants about 5 V for readable
  contrast, but a backpack powered at 5 V idles SDA and SCL at 5 V through its
  own pull-ups, into pins that are not 5 V tolerant. Power the backpack from 5 V
  and move its pull-ups to 3V3 — one desolder — rather than hoping. And a
  "dead" 1602 is very often just its contrast trimpot.

---

## 5. Pins

Seven are always used; **sense**, **relay**, **stop** and **strike** only by the
modes that need them.

<!-- PINMAP: checked against toolguard-fw/src/main.rs by
     toolguard-core/tests/pinmap.rs. Keep the table shape if you edit it. -->

| signal | esp32c6 | esp32c3 | esp32s3 |
|---|---|---|---|
| light | 8 | 8 | 48 |
| setup | 9 | 9 | 0 |
| sclk | 0 | 10 | 12 |
| mosi | 1 | 7 | 11 |
| miso | 2 | 6 | 13 |
| cs | 3 | 5 | 10 |
| rst | 18 | 4 | 14 |
| sda | 22 | 1 | 4 |
| scl | 23 | 0 | 5 |
| sense | 14 | 2 | 15 |
| relay | 19 | 3 | 16 |
| stop | 20 | 20 | 17 |
| strike | 21 | 21 | 18 |

### Do not move these on a C6 Super Mini

| pin | why |
|---|---|
| **GPIO8** | the RGB LED *and* a strapping pin — download mode needs it high |
| **GPIO9** | BOOT, a strapping pin, and the hold-for-setup gesture |
| **GPIO12 / GPIO13** | **native USB, and the only console there is.** This board has no USB-serial chip, so losing these loses flashing *and* logs |
| **GPIO4–7** | strapping, JTAG and flash-adjacent |

Freely usable on that board: IO0–IO3, IO14, IO18–IO23. SPI and I²C take seven of
them, which is why sense, relay, stop and strike are 14, 19, 20 and 21 — and
with all four roles wired a C6 Super Mini has no pin left.

### The sensed input is digital, and cannot be analog here

On the ESP32-C6, ADC1 is **GPIO0–GPIO6 only**, and SPI occupies all of those. So
there is no ADC pin left for a sense input on this pin map. That is no loss for
an optoisolated enable line, which is a logic signal — but it does mean you
cannot put a current-clamp straight onto this board without an external I²C ADC
on the existing bus.

The ESP32's own ADC is also badly non-linear at low voltages, so a clamp is a
poor primary signal for "is the machine running" even where a pin is available.
An interlock or enable line is the better signal wherever one exists.

### SPI mode is 3, not 0, deliberately

ESP-IDF's SPI master on the **S3** samples MISO on the wrong edge in mode 0:
every register read comes back corrupted while writes still land, so the RC522
appears to reset, polling appears to run, and **no card is ever detected**. Mode
3 at 1 MHz is used on all three chips for consistency. If you are debugging a
reader that detects nothing, check §7 before suspecting the antenna.

---

## 6. Configuration

Two places, and the split is deliberate.

### On the device's own setup page

Hold **BOOT** for three seconds to force the portal, or it appears on its own
until the required fields are filled. Join the `spaces-setup-XXXXXX` network it
advertises.

Everything here is **optional with a default**, so an existing box keeps working
after a firmware update rather than dropping into its portal.

| setting | default | what it does |
|---|---|---|
| Has a card reader | yes | `no` makes this a plug: it switches the tool on the edge's lease and never reads a badge |
| Switches the tool | **no** | `yes` drives the relay — and registers this device in the `power` role |
| Senses the machine | **no** | `yes` reads the sense pin and reports working time. Without one, a box that switches the tool reports the time the power was on; a box that does neither reports no time |
| Sense is active low | yes | match §3. Wrong = usage recorded when the machine is idle |
| Bridge gaps (seconds) | 10 | how long the machine may pause and still count as working. See §2c |
| Ignore blips under (ms) | 100 | the signal must hold this long to count |
| Tap again to finish | yes, unless gating | the member's own card ends their session |
| Has a stop button | **no** | `yes` reads the stop pin (§4c). Pressing it ends the session; anyone may, and it is not an E-stop |
| Drives a latch | **no** | `yes` drives the strike pin for the pulse the platform names on a yes (§4d) — and registers this device in the `strike` role. Not an egress or fire-safety device |
| Finish after idle (s) | 0 (never) | needs a sensor; ignored without one |
| Longest session (s) | 0 (none) | the walk-away backstop |
| Report usage every (s) | 30 | usage is sent in increments, so a reboot costs one interval |
| Reports usage | yes | `no` for a box that must not meter — see "A second plug on the same tool" below. Nothing else changes |
| Wait for lease up to (s) | 5 | after a yes, how long a box that switches the tool waits for its first lease before giving up. Must exceed the tool's whole start sequence (below). Ignored without a relay |

Yes/no fields accept `yes`/`no`, `y`/`n`, `true`/`false`, `1`/`0`, `on`/`off`.
**Anything unrecognised uses the default and logs a warning** — a typo does not
quietly turn an output on or off.

#### A second plug on the same tool

A tool may have more than one box switching power — an extractor fan on its own
circuit that must run whenever the laser does. Bind the second box to the same
tool in the `power` role: the edge leases every `power` binding on a tool, so
both follow the same session and switch within a poll of each other. Two
things to set:

- **"Reports usage" = `no` on the second box.** A box that switches power
  reports the time the power was on; two of them would report the same minutes
  twice, and reports accumulate.
- **Decide its "On disconnect"** in Tool Wiring. The default `fail_off` means a
  silent extractor plug withholds the laser's lease too — no laser without the
  filter — which may be exactly what you want. `ignore` lets the laser run
  without it.

**Order, and run-on.** Each `power` binding in Tool Wiring carries three
timings, and the platform applies them; this box only obeys the lease it is
given:

- *Start stage* — boxes in a lower stage must have **confirmed they are on**
  (their own power report saying `relay_on: true`) before this one is leased.
  Any number of boxes per stage. A box that cannot report its relay state can
  never confirm, so nothing behind it will ever start — this firmware always
  reports it when "Switches the tool" is on.
- *Start delay* — then wait this long more: spin-up.
- *Run-on* — after **any** stop or trip, keep this box on this long.

So for a machine behind an extractor: extractor at stage 0 with no delay and a
run-on; machine at stage 1 with a delay for the fan to spin up. Two things
follow for the machine's box:

- While its turn has not come it receives **no lease at all** — silence, not a
  refusal. Set **"Wait for lease up to"** on it longer than the whole sequence:
  the stage before it confirming (about a second after it energizes) plus its
  own start delay, with margin. A box that gives up at the default five seconds
  gives up on a three-second spin-up more often than not.
- A lease is granted **only while somebody is signed on** to the tool. A box
  with no reader on its tool is never energized; the extractor in this example
  is on because the machine's reader opened a session on the tool they share.

#### Three that are easy to get wrong

- **"Senses the machine" must be `no` unless something really is wired in.** A
  device that reports zero working time is *believed*. On a tool billed by
  working time that bills nothing, for every member, silently.
- **"Switches the tool" must be `yes` only if a relay is wired**, because it is
  also what makes this box register as a `power` device. Claim it falsely and
  the tool's power is nominally this box's responsibility while nothing is
  connected.
- **"Tap again to finish" is the only stop a bare reader has.** A box that
  neither switches the tool nor senses it has no other way to know the member
  has finished. Turn it off only if you have set a session cap instead.

### On the platform, in Resource Wiring

An administrator binds this device to a tool **in a role**. Until that binding
exists, `tool-on` refuses every card and the refusal looks exactly like a
credential problem.

The roles this box registers with are derived from its settings, so bind it in
the roles it actually claims:

| settings | registers as |
|---|---|
| reader only | `reader` |
| reader + switches | `reader`, `power` |
| reader + senses | `reader`, `sensor` |
| reader + switches + senses | `reader`, `power`, `sensor` |
| no reader + switches | `power` |
| reader + drives a latch | `reader`, `strike` — a door box |
| reader + switches + drives a latch | `reader`, `power`, `strike` — a room in one box |

---

## 7. Commissioning, in order

1. **Flash and bring up the portal.** Set Wi-Fi, the server, the local broker,
   the device invite, and the tool's `external_id`.
2. **Set the role settings** from §6 to match what you actually wired.
3. **Check the boot log.** The device logs one line saying what it believes it
   is — `reads_cards=… gates_power=… senses_running=… (active low) hold=…` — and
   the roles it registered with. Read it. Most misconfigurations show up as a
   tool that quietly never works, and this line is the fastest way to the cause.
4. **Bind the device to the resource** — the tool, or the door — in Resource
   Wiring, in the roles from §6. A door box is bound in `reader` and `strike`;
   the strike binding is where its pulse is set.
5. **Confirm the RC522 answers.** Its version register should read `0x91` or
   `0x92`. A reader that detects nothing with a healthy version register is
   usually the SPI-mode problem in §5.
6. **Check the panel.** If it is blank, turn the contrast trimpot before
   suspecting anything else. The backpack is at `0x27` or `0x3F` — both are
   scanned, and the parts look identical.
7. **Swipe a known-good card** and watch the panel: `Checking card` →
   `ACCESS GRANTED` → `Signed in`. In gating mode it should read
   `Tool unlocked` once the lease arrives. With "Senses the machine" on, the
   bottom line is the meter -- `Idle     0:00:00` until the machine works,
   then `In use   0:00:07` climbing -- and it is the same figure the box
   reports, so a member can check what they are being billed. Without a
   sensor the bottom line is the `Tap to finish` hint instead; a box that
   measures nothing shows no figure.
8. **Swipe again** and confirm `Session ended`, if "Tap again to finish" is on.
9. **If a stop button is wired, press it mid-session.** The panel should read
   `Session ended` at once and, in gating mode, the relay should open. If
   nothing happens, "Has a stop button" is off (§6) or the switch is not
   between the stop pin and GND (§4c). A press with no session open does
   nothing, by design.
10. **In gating mode, test the fail-safe with the tube disconnected** (§4b).
    **With a latch, swipe with the latch driver disconnected from the latch**
    and watch the driver: it should fire for the pulse set on the strike
    binding and not a moment longer, and the panel should read `Door open`
    then `Present card`. If the log says `named no pulse`, the resource has no
    strike binding yet.
11. **Fire a real job** and check the reported usage against the clock. This is
    where a wrong "Bridge gaps" shows up: far too little usage means the bridge
    is shorter than the machine's pauses.

### On liveness, and what this box says about itself

The platform listens for every device bound to a tool, and a binding's
**"On disconnect"** setting says what a device going quiet does to that tool:

| role | default | why |
|---|---|---|
| `power` | `fail_off` | a silent plug is an unobserved circuit, so the lease is refused |
| `sensor` | `fail_off` | a silent sensor is an unobserved machine, same reason |
| `reader` | `ignore` | a dead reader cannot produce a swipe, so there is nothing to protect against, and it is no reason to stop a job already running |

**This firmware says it is alive in every configuration**, on the heartbeat
timer, by two messages: a power report (with the draw and relay fields null when
it has no relay, because it cannot honestly report them) and an *alive* pulse
that carries its device id and nothing else. The pulse is what the platform
counts, whatever the role, and it also reaches the server, so a reader that
dies shows up as silent on the admin device list even though the tool is not
withheld over it. The defaults above are therefore safe to leave alone.

Two consequences worth knowing:

- Firmware that sends *neither* message will be refused a lease forever if bound
  as `power` or `sensor` with the default. That is not this firmware, but it may
  be another device on the same tool.
- The binding has to exist first. Until it does, the edge logs that the report
  *"matched no module binding; it will be refused a lease until its wiring
  arrives."* That message means step 4 has not been done, not that anything is
  broken.

---

## 8. When something is wrong

| what you see | what it usually is |
|---|---|
| Panel reads `No Wi-Fi` | credentials, or out of range. Cards are refused locally and not sent |
| Panel reads `No edge` | Wi-Fi is up, the local broker is not. Check the broker address and that the edge is running |
| Every card is `ACCESS DENIED` with a credential-shaped reason | the device is not bound to the tool. Step 4 |
| Panel goes `ACCESS GRANTED` then `Session ended` a few seconds later | configured as switching the tool but not bound in the `power` role, so no lease ever arrives. The log says `is this module bound in the power role?` |
| Tool stays "in use" after a member leaves | nothing closed the session. Turn on "Tap again to finish", or set a session cap, or both. The server does eventually release it -- once the tool's session cap plus a grace has run out, hours at the defaults -- and raises a Warning-severity `tool_released_by_sweep` event when it does. That is a net for a crashed box, not a way to run one |
| A member is billed nothing for a long job | usage was never reported. On a box that switches the tool, check it is bound in the `power` role and the lease arrives (the figure is the time the power was on). On a box that only senses, check "Senses the machine" is on, the sense polarity, and the job against §2c. A box that does neither has no figure to report |
| Usage is a fraction of the real time | "Bridge gaps" is shorter than the machine's pauses. §2c |
| Usage counts time the machine was idle | sense polarity is inverted. §3 |
| No card is ever detected, version register fine | SPI mode. §5 |
| Pressing the stop button does nothing | "Has a stop button" is off, or the switch is not between the stop pin and GND (§4c). With no session open on this box the stop still goes out naming the resource, so a room held open by a door box is closed by it; the panel shows nothing for that |
| `Door open` shows but the latch does not release | the driver, its supply, or the latch's own polarity (§4d: a maglock needs an inverting driver). The pin itself went high — the log says `Strike ON` — so the fault is downstream of this box |
| `ACCESS GRANTED` on a box that should say `Door open`, or the latch never fires | "Drives a latch" is off, or the box also switches the tool (then it is a room in one box and keeps the tool's screens). The log line at boot says `drives_latch=` |
| The latch fires, the log says `named no pulse` next time | the resource has no `strike` binding on the platform; bind one and set its pulse |
| The stop button fires on its own | the stop pin is floating or picking up noise from the HV side — check the switch really goes to GND and keep its wire away from the laser supply. Only a contact held for 50 ms counts, so this takes real interference |
| Panel blank | contrast trimpot, then the I²C address, then the wiring |
| `Reader fault — see /log` | the RC522 did not answer at all. Check 3V3 and the SPI wiring |

The full log is at `http://<device-address>/log`, over the network, and the
diagnostic screen shows the address — which is what that line of the panel is
for. The log always has more than the panel's sixteen columns can.

---

## 9. What this document does not promise

Stated here rather than discovered on a bench:

- **No board has run this firmware.** It compiles and is tested for every
  decision it makes, on a host, but nothing in §7 beyond step 1 has been
  performed on real hardware.
- **The C3 and S3 pin maps are compiled and nothing more.** Only the C6 Super
  Mini map has been reasoned against a real board's constraints; the other two
  are chosen to avoid known-reserved pins and are unverified.
- **The sink budget in §2b is unmeasured** and unpublished. The advice to stay
  high-impedance follows from not knowing it, not from having measured a
  problem.
- **Whether your machine's enable line chatters** is a property of your machine
  and its settings, which is why §2c asks you to look rather than telling you.

[fw]: https://github.com/Cambridge-Hackspace/cooperative-systems-spaces/blob/dev/FIRMWARE.md
