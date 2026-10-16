# aula

AULA keyboard settings for Linux. Change lighting, brightness, colours and sleep time with the familiar AULA interface.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/yhuikzdtguioaert/AulaSofwareLinux/main/install.sh | sh
```

The installer builds the app, adds it to your application menu and asks for your password to set up keyboard access. Reconnect the keyboard if needed, then open **AULA**.

Prebuilt Linux x86_64 packages are under [Actions → Build](https://github.com/yhuikzdtguioaert/AulaSofwareLinux/actions/workflows/build.yml). Download the artifact, unpack the archive and run `./install.sh` from its folder.

## Devices

F75, F87, F87 Pro, F87 Pro Wired, F99, F99 Pro, F99 Wired and H108. USB and the 2.4 GHz receiver are supported. Wake the keyboard with a key press if it is asleep.

Use USB for per-key colours and factory reset. Bluetooth configuration and other AULA device families are not supported yet. Knob press assignments work over USB when the keyboard exposes an identifiable knob entry.

## Build

```sh
cargo build --release --locked
./target/release/aula
```

Requires Rust and a C compiler. On Debian/Ubuntu, install `build-essential pkg-config libxkbcommon-dev libwayland-dev libgl1-mesa-dev libx11-dev`. On Void, install `base-devel pkg-config libxkbcommon-devel wayland-devel MesaLib-devel libX11-devel`.

Unofficial project, not affiliated with AULA. Code: [MIT](LICENSE). Bundled artwork and device profiles belong to AULA.
