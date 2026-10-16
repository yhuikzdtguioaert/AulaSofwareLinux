#!/bin/sh
set -eu

repo=https://github.com/yhuikzdtguioaert/AulaSofwareLinux
prefix="${XDG_DATA_HOME:-$HOME/.local/share}/aula"
work=
rule=
program=
trap '[ -z "$program" ] || rm -f "$program"; [ -z "$rule" ] || rm -f "$rule"; [ -z "$work" ] || rm -rf "$work"' EXIT HUP INT TERM

here=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd)
if [ ! -d "$here/assets" ] || { [ ! -f "$here/Cargo.toml" ] && [ ! -x "$here/aula" ]; }; then
    work=$(mktemp -d)
    curl -fL "$repo/archive/refs/heads/main.tar.gz" -o "$work/source.tar.gz"
    tar -xzf "$work/source.tar.gz" -C "$work"
    here="$work/AulaSofwareLinux-main"
fi

if [ -x "$here/aula" ]; then
    binary="$here/aula"
else
    if ! command -v cargo >/dev/null 2>&1; then
        [ ! -f "$HOME/.cargo/env" ] || . "$HOME/.cargo/env"
    fi
    if ! command -v cargo >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
        . "$HOME/.cargo/env"
    fi
    (cd "$here" && cargo build --release --locked)
    binary="$here/target/release/aula"
fi

mkdir -p "$prefix" "$HOME/.local/bin" "${XDG_DATA_HOME:-$HOME/.local/share}/applications"
if [ "$here" != "$prefix" ]; then
    cp -R "$here/assets" "$prefix/"
    program=$(mktemp "$prefix/.aula.XXXXXX")
    cp "$binary" "$program"
    chmod 755 "$program"
    mv -f "$program" "$prefix/aula"
    program=
    mkdir -p "$prefix/udev"
    cp "$here/udev/60-aula.rules" "$prefix/udev/"
    cp "$here/install.sh" "$here/LICENSE" "$prefix/"
fi
chmod +x "$prefix/aula"
# Use a wrapper so asset lookup works when launched through ~/.local/bin.
rm -f "$HOME/.local/bin/aula"
cat > "$HOME/.local/bin/aula" <<LAUNCHER
#!/bin/sh
exec "$prefix/aula" "\$@"
LAUNCHER
chmod +x "$HOME/.local/bin/aula"
cat > "${XDG_DATA_HOME:-$HOME/.local/share}/applications/aula.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=AULA
Exec="$prefix/aula"
Categories=Settings;HardwareSettings;
Terminal=false
DESKTOP

# OWNER also works on desktops without systemd-logind/uaccess (e.g. Void).
owner=$(id -un)
rule=$(mktemp)
sed "s/MODE=\"0660\"/OWNER=\"$owner\", MODE=\"0660\"/" "$prefix/udev/60-aula.rules" > "$rule"
if cmp -s "$rule" /etc/udev/rules.d/60-aula.rules; then
    :
elif [ "$(id -u)" -eq 0 ]; then
    install -Dm644 "$rule" /etc/udev/rules.d/60-aula.rules
    rm -f /etc/udev/rules.d/99-aula.rules
    udevadm control --reload-rules
    udevadm trigger --subsystem-match=hidraw --action=add
else
    printf 'Device access setup (administrator password):\n'
    sudo install -Dm644 "$rule" /etc/udev/rules.d/60-aula.rules
    sudo rm -f /etc/udev/rules.d/99-aula.rules
    sudo udevadm control --reload-rules
    sudo udevadm trigger --subsystem-match=hidraw --action=add
fi
rm -f "$rule"
printf 'Installed. Open AULA from your application menu or run ~/.local/bin/aula.\n'
