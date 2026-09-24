# AGENTS.md

Guidance for coding agents working in this repository. See `README.md` for what squeak is and
its current status.

## Commands

- `cargo run --release` talks to the real mouse (Enter polls, Ctrl+C stops).
- A running `squeak.exe` locks the file: `cargo build --release` fails with "Access is denied"
  until it's stopped.

## Protocol

`docs/protocol.md` is the source of truth for the mouse protocol (report layouts, byte offsets,
push behaviour, open questions). Its offsets exclude the report ID; on Win32 buffers add 1. Keep
it updated when new behaviour is observed.

`reference/launcher/` (git-ignored) holds the formatted launcher.keychron.com source the
protocol was read from.

## Rules

- **Minimal memory and CPU use is a core requirement.** Raw Win32 via `windows-sys`; no async
  runtime, no GUI framework, no new dependencies unless clearly necessary. Prefer blocking on
  events over timers.
- **Never write to a HID collection unless it matches:** VID `0x3434`, usage `0xFFC1:0x0001`,
  64-byte input/output reports, and declared report IDs `0xB3` (out) / `0xB4` (in). The dongle's
  `0xFF60:0x61` collection (also used by Keychron keyboards) speaks QMK/VIA, where `0x06` is
  `dynamic_keymap_reset`.
- Only send read-only commands (`06 00`). Never experiment with unknown commands on the device.
- No periodic polling: the mouse pushes `e2` reports on wake, sleep, link and charging changes.
  Add polling only if testing shows it's needed.
