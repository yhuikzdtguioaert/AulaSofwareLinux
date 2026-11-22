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

Use USB to change per-key colours, reassign the knob press, or restore factory settings. The F99 Pro also shows its stored per-key colours and knob assignment over the receiver. In **Global**, use **Exit knob lighting mode** to return the F99 Pro knob to its normal controls after a long press switches it to onboard lighting.

Configuration over Bluetooth requires switching to USB or a supported 2.4 GHz receiver.

thanks to [xx12ek](https://github.com/xx12ek) for testing the f99 pro on hardware and sharing the [aula hub protocol notes](https://github.com/xx12ek/aula-f99-pro-hub/blob/main/docs/PROTOCOL.md)

## Build

```sh
cargo build --release --locked
./target/release/aula
```

Requires Rust and a C compiler. On Debian/Ubuntu, install `build-essential pkg-config libxkbcommon-dev libwayland-dev libgl1-mesa-dev libx11-dev`. On Void, install `base-devel pkg-config libxkbcommon-devel wayland-devel MesaLib-devel libX11-devel`.

Unofficial project, not affiliated with AULA. Code: [MIT](LICENSE). Bundled artwork and device profiles belong to AULA.
