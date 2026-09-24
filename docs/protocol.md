# Keychron M6 HID protocol notes

Reverse-engineered from the traffic between [launcher.keychron.com](https://launcher.keychron.com)
(WebHID) and a Keychron M6 8K, connected through the **Keychron Ultra-Link 8K** 2.4 GHz dongle
and over USB cable, plus the launcher's own JavaScript. Only what has been observed or read in
the launcher source is written here; guesses are marked as such.

## Devices

| Device                    | VID      | PID      | Notes                                  |
| ------------------------- | -------- | -------- | -------------------------------------- |
| Keychron Ultra-Link 8K    | `0x3434` | `0xd028` | 2.4 GHz dongle, 3 vendor collections visible to WebHID |
| Keychron M6 8K            | `0x3434` | `0xd049` | Reported by the dongle (see `b2`); also its own PID when connected by cable. The launcher's built-in demo device uses the same PID and name |

Not yet captured: Bluetooth.

### Wired (USB cable)

The mouse itself enumerates as "Keychron M6 8K", PID `0xd049`, with collections `0xFFC1 : 0x01`
and `0x008C : 0x01` (no `0xFF60`). The same commands work on `0xB3`/`0xB4` with identical
replies. Report sizes were not dumped, but the traffic matches the dongle's (63-byte `0xB4`/`0xB6`).

Differences in the launcher's requests: it sends `04 00` and `02 00` when wired, and `04 01` and
`02 01` through the dongle. The second byte may select "local device" vs "device behind
dongle" (unconfirmed). `06 00` is the same in both cases.

### Cable and dongle at the same time

The mouse switches to the cable. The dongle's `b2` reply then has status `00` (not connected),
and a `06 00` sent to the dongle's `0xFFC1` collection got **no reply**. So a dongle without a
connected mouse just times out, and the same mouse does not answer through both paths at once.

## Conventions

- Byte offsets below are into the report **data**, i.e. excluding the report ID (as WebHID
  presents it). With Win32 `WriteFile`/`ReadFile`/`HidD_*`, byte 0 of the buffer is the report
  ID, so add 1 to every offset.
- Every command is echoed back in the first byte(s) of its reply.
- Unused bytes are zero in requests. Replies may contain **stale bytes** from a previous reply
  (the firmware appears to reuse its buffer), so only trust fields a command is known to fill.

## HID collections (Ultra-Link 8K)

As reported by WebHID `device.collections`. Each top-level collection is a separate HID device
path on Windows. WebHID hides the standard mouse/keyboard collections, so the dongle has more
than these three.

| Usage page : usage      | Out reports (data bytes) | In reports (data bytes) | Purpose                          |
| ----------------------- | ------------------------ | ----------------------- | -------------------------------- |
| `0xFFC1 : 0x0001`       | `0xB3` (63), `0xB5` (63) | `0xB4` (63), `0xB6` (63) | **Mouse protocol — squeak uses this** |
| `0xFF60 : 0x0061`       | none/`0x00` (32)         | none/`0x00` (32)        | QMK/VIA raw HID; dongle queries (`b2`, `bc`) |
| `0x008C : 0x0001`       | `0xB2` (32)              | `0xB1` (32)             | "Bridge" per the launcher; dongle's own framed protocol (page `0x8C` is officially "Bar Code Scanner") |

On Windows (`HidP_GetCaps`) the `0xFFC1` collection should report `OutputReportByteLength` and
`InputReportByteLength` = 64 (largest report + 1 byte report ID).

### How the launcher classifies collections

`requestDevice` filters and `getDeviceInfo` in `main.*.js` map usage page : usage to a device
kind, independent of VID/PID:

| Usage page : usage | Launcher key  |
| ------------------ | ------------- |
| `0xFF60 : 0x61`    | `keyboard`    |
| `0xFFC1 : 0x01`    | `mouse`       |
| `0xFF0A : 0x01`    | `mouse_4k`    |
| `0x008C : 0x01`    | `bridge`      |

So Keychron itself treats `0xFFC1 : 0x01` as "speaks the mouse protocol". `mouse_4k` is a
different mouse family, likely the one using the `0xA7` battery command below.

## Report channels

| Out report ID | In report ID | Data size | Collection | Used for                                |
| ------------- | ------------ | --------- | ---------- | --------------------------------------- |
| `0xB3` (179)  | `0xB4` (180) | 63        | `0xFFC1`   | Mouse commands (`04`, `06`, `61`, `62`) |
| `0xB5` (181)  | `0xB6` (182) | 63        | `0xFFC1`   | Link/device info (`02`, `0b`)           |
| `0x00` (none) | `0x00`       | 32        | `0xFF60`   | Dongle queries (`b2`, `bc`)             |

The launcher writes only 20 bytes to `0xB5`; the browser/OS pads it to the declared 63.

## Probing safety

The `0xFF60 : 0x61` collection speaks the QMK/VIA raw-HID protocol, where command `0x06` is
`dynamic_keymap_reset`. The same VID and page are used by Keychron keyboards. **Never send the
mouse commands on it.** Identify the mouse channel by its declared reports (output `0xB3`,
input `0xB4`, 63 data bytes), not by VID or usage page alone, and probe only with the
read-only `06 00`.

## Commands

### `0xB3` `06 00` — status / settings (contains battery)

```
→ 06 00 00 …
← 06 00 11 11 01 40 06 80 0c 80 0c 80 0c 80 0c 17 02 04 0a 5b 00 … b7 00 00 00 04 04 14 04 …
```

Field layout per the launcher's parser (`main.*.js`, `class I` → `get()`, which filters
input reports on `data[0] === 6`):

| Offset | Example         | Meaning                                                            |
| ------ | --------------- | ------------------------------------------------------------------ |
| 1      | `00`            | Current profile                                                    |
| 2–4    | `11 11 01`      | Per-profile level indices: low nibble = DPI, high nibble = polling |
| 5–14   | `40 06` `80 0c`… | 5 DPI stages, u16 LE (1600, 3200, 3200, 3200, 3200)               |
| 15     | `17`            | Bit flags: LOD (bits 0–1), wave (2), line (3), motion (4), scroll direction (6) |
| 16     | `02`            | Number of DPI levels                                               |
| 17     | `04`            | Debounce                                                           |
| 18     | `0a`            | Sleep time                                                         |
| **19** | **`5b`**        | **Power: bits 0–6 = battery %, bit 7 = charging** |
| 26     | `b7`            | Feature flags 1 (scroll, debounce, profiles, …)                    |
| 27–29  |                 | Scroll speed, inertia, SPL                                         |
| 30–39  |                 | Debounce values                                                    |
| 40–41  |                 | Max DPI, u16 LE (step is 100)                                      |
| 43–48  |                 | Polling rate levels                                                |
| 49–50  |                 | Polling level count, profile level count                           |
| 51     |                 | Wake source flags (key, scroll, move, side scroll)                 |
| 52     |                 | Bit 0: 20K FPS mode                                                |
| 53, 60 |                 | Feature flags 2 and 3                                              |
| 55     |                 | Angle (signed)                                                     |

Battery: `data[19] & 0x7F` = percent (`0x5b` = 91, matched the launcher), `data[19] >> 7` =
charging. Confirmed with an external charger (dongle connection): `0x5b` → `0xdb` when the
charger was plugged in, and back to `0x5b` when it was removed. Also set while charging over
the USB cable.

The percentage reads a few points differently while charging (89 → 93 within ~2 minutes of
cable charging). Low-battery decisions should use readings taken while not charging.

### `0xB3` `04 01` / `04 00` — firmware version and name

`04 01` through the dongle, `04 00` over cable; same reply.

```
← 04 06 "1.0.1" 00 … 0c "Keychron M6" 00 …
```

- Offset 2: version string `1.0.1`, NUL-terminated (offset 1 = `06` is probably its length incl. NUL).
- Offset 21: `0c` (12), probably the length of the following name, `Keychron M6` at offset 22.

### `0xB3` `61 00`, `62 xx` — unknown

`62` was sent with `xx` = `09 08 0a 0b 0e 0d`. All replies were the echoed command followed by
zeros.

### `0xB5` `02 01` / `02 00` — connected device info

`02 01` through the dongle, `02 00` over cable.

```
← 02 04 00 34 34 49 d0 01 01 11 04 00 …   dongle
← 02 04 00 34 34 49 d0 01 01 10 04 00 …   cable
```

Offsets 3–4 = VID `0x3434`, 5–6 = PID `0xd049` (little-endian). Offset 9 is `11` through the
dongle and `10` over cable (connection mode?). The rest is unknown.

### `0xB6` `e2` — pushed power notification (unsolicited)

Arrives on input report `0xB6` without any request.

```
← e2 01 01 03 5b 01 01 05 00 …   dongle, external charger plugged in
← e2 01 01 00 5b 01 01 05 00 …   dongle, charger unplugged
← e2 00 00 03 5a 01 01 05 00 …   cable, charging
```

```
← e2 01 00 ff ff ff ff …         dongle, link to the mouse lost (sleep, switched to cable, off)
```

| Offset | Meaning                                                              |
| ------ | -------------------------------------------------------------------- |
| 1      | `01` through the dongle, `00` over cable                             |
| 2      | `01` = mouse connected, `00` = dongle has lost the mouse             |
| 3      | Charge state: `03` = charging, `00` = not charging (`ff` = no data)  |
| 4      | Battery percent, plain value without charging bit (`ff` = no data)   |
| 5–7    | `01 01 05`, unknown (`ff` = no data)                                 |

When it's sent:
- Through the dongle, while the mouse is awake: seen twice about a minute apart after handling
  the mouse, including a drop (`5f` → `5e`). **Not confirmed as periodic**: in a later idle run
  no pushes arrived. Unknown whether pushes follow usage, battery changes, or a timer.
- Through the dongle: when the mouse reconnects or **wakes from sleep** (one to three pushes
  within a few seconds), and when the link is lost (the `ff` variant): on switching to the
  cable, and when the mouse **goes to sleep** after ~10 idle minutes (matches sleep time `0a`
  in the `06` reply). Observed over two sleep/wake cycles.
- The open device handle kept receiving pushes after the PC hibernated and resumed (one sample).
- Through the dongle: on a charging-state change. Seen on plugging in and unplugging an external
  charger once, but another time plugging in produced no push. Not reliable on its own; the
  next periodic push carries the new state anyway.
- Over cable while charging: roughly every 10 s, with the percentage climbing (`5a` → `5d`).
- While the mouse sleeps, nothing is pushed, and a `06 00` poll through the dongle gets no
  reply either.

Through the dongle, the launcher reacts to each `e2` by sending `b2` on report 0.

### `0xB6` `85 02` — unknown notification

Seen once, unsolicited, right after the dongle was reconnected while the mouse was on the
cable. Possibly a link-state change.

### `0xB5` `0b 02` — unknown

```
← 0b 00 09 03 00 …
```

### Report `0x00` `b2` — dongle: paired device

```
← b2 01 34 34 49 d0 01 00 …
```

Up to three paired devices, 5 bytes each at offsets 2, 7 and 12: VID (u16 LE), PID (u16 LE),
status. Here: VID `0x3434`, PID `0xd049`, status `01`. Offset 1 (`01`) is unknown.

Status is `01` when the mouse is connected through the dongle and `00` when it isn't (seen
while the mouse was on the cable).

### Report `0xB2` → `0xB1` on `0x008C` — bridge protocol

Only seen when the dongle was plugged in while the launcher was open. Uses a framed format:

```
→ aa 55 03 fc 01 60 60 00 …
← aa 55 2c d3 01 a3 01 60 00 "LMKDGP01" 00 00 05 03 "0.1.6" 00 … "1.0.0" …   (continues in a 2nd report)
→ aa 55 03 fc 02 61 61 00 …
← aa 55 09 f6 02 a3 02 61 00 01 00 01 08 01 …
```

`aa 55` header, then a length byte and its complement (`03`/`fc`, `2c`/`d3`), then a sequence
number and payload. The reply to `60` looks like dongle info: model `LMKDGP01`, versions `0.1.6`
and `1.0.0`. Not needed by squeak.

### Report `0x00` `bc` — dongle state

Sent several times; no reply seen in the captures. The launcher parses `0xBC` input reports as
dongle state (see "Other command families" below).

## Observed launcher sequence

On connect (run once or a few times in quick succession):

1. `b2` on report 0 (dongle only)
2. `06 00` on `0xB3`
3. `61 00` on `0xB3`
4. `02 01`/`02 00` on `0xB5`
5. `62 09`, `62 08`, `62 0a`, `62 0b`, `62 0e`, `62 0d` on `0xB3`
6. `04 01`/`04 00` on `0xB3`

`0b 02` was sent once.

After that the launcher did **not** poll `06 00` on a timer in the captures. It sent only `b2`
in response to pushed `e2` notifications, so it appears to rely on those for battery updates.

## Minimum needed for squeak

1. Open the HID collection with usage page `0xFFC1`, usage `0x0001` that declares report IDs
   `0xB3`/`0xB4`.
2. Write output report `0xB3` with data `06 00` + zero padding (64 bytes including report ID).
3. Read input reports until one with ID `0xB4` and data starting with `06` arrives.
4. Battery % = `data[19] & 0x7F` (buffer byte 20 on Win32); charging = `data[19] >> 7`.
5. No reply within a timeout means no mouse is reachable on that path right now: asleep,
   switched off, or on the cable instead of the dongle.
6. After that, listen for unsolicited `0xB6` `e2` reports. Whether a slow fallback poll is
   also needed depends on how regularly these arrive during normal use (still to be measured).

## Open questions

Test with the launcher closed; it may influence what the mouse pushes.

- Is an `e2` pushed reliably when an external charger is plugged in (with the mouse awake)?
- Is an `e2` pushed when the percentage drops during normal use (e.g. 93 → 92)?
- Near empty: more frequent pushes? Lowest reported value before shutdown?
- Bluetooth mode: not examined at all.

## Other command families in the launcher

The launcher also contains a different battery command (`0xA7` + `KC_USER_CMD_NAPE_GET_BAT_REPORT`
on report 0, reply `[2]` = value, `[3]` = status) used by another device class, probably the
`mouse_4k` (`0xFF0A`) family. The M6 through the Ultra-Link dongle did **not** use it in the
captured traffic.

Report 0 `0xBC` is parsed as a dongle-state notification: `[1]` = keyboard 1, `[2]` = keyboard 2,
`(data[3] >> 2) & 1` = mouse connected. Report 0 `0xB2` lists up to three paired devices
(VID/PID/status at offsets 2–6, 7–11, 12–16).
