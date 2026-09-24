# squeak

Small Windows utility that keeps track of your mouse's battery level.

> **Focus: minuscule memory and CPU impact.** squeak is meant to run all day, unnoticed. It uses
> raw Win32 APIs with no runtime, GUI framework or async executor, doesn't poll the mouse, and
> sleeps until the mouse itself reports a change. Contributions are expected to keep it that
> way.

## Status

Early proof of concept. squeak currently runs in a console: it reads the battery once at
startup and then prints every update the mouse pushes. Planned: a tray icon showing the battery
level and a notification when it runs low.

## Supported devices

| Device                | Connection                         |
| --------------------- | ---------------------------------- |
| Keychron M6 8K        | Keychron Ultra-Link 8K dongle, USB cable |

Other Keychron mice that use the same protocol (HID usage page `0xFFC1`) may work but are
untested. Bluetooth is not supported.

## Getting started

### Requirements

- Windows 10 or 11
- [Rust](https://rustup.rs) with the MSVC toolchain (`stable-x86_64-pc-windows-msvc`)
- Visual Studio Build Tools with the "Desktop development with C++" workload (for the MSVC
  linker and Windows SDK)

### Build and run

```sh
git clone https://github.com/oBusk/squeak.git
cd squeak
cargo run --release
```

Example output:

```
00:55:42  found Keychron Ultra-Link 8K (pid 0xd028)
00:55:42  listening for pushes… (Enter to poll, Ctrl+C to stop)
00:55:43  Keychron Ultra-Link 8K (pid 0xd028)  poll   93%  not charging
01:20:25  Keychron Ultra-Link 8K (pid 0xd028)  push  mouse asleep or disconnected
```

Press Enter to request the battery level manually. The release build is a single ~125 KB
executable (`target/release/squeak.exe`).

## How it works

The mouse (and its dongle) expose a vendor-specific HID channel. squeak finds it, asks for the
mouse's status once, and then waits for the reports the mouse sends on its own when it wakes,
goes to sleep, or starts or stops charging. The protocol was reverse-engineered from Keychron's
web configurator and is documented in [docs/protocol.md](docs/protocol.md).

squeak only sends read-only requests, and only to HID channels whose layout exactly matches the
mouse protocol. It never writes to other devices or channels.

## License

Licensed under the [MIT License](LICENSE).
