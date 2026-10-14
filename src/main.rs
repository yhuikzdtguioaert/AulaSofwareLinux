//! `aula` — Linux control software for AULA keyboards.
//!
//! Run with no arguments for the window; the subcommands below are for
//! inspecting hardware and for the protocol work that has not been confirmed
//! yet.
//!
//! Parts of the protocol surface (whole commands, profile fields) are described
//! but not yet used by the UI; they are kept so each module stays a complete
//! description of the device rather than only what one screen needs.

#![allow(dead_code)]

mod assets;
mod config_page;
mod device;
mod hid;
mod ini;
mod profile;
mod proto;
mod ui;
mod worker;

use std::path::PathBuf;

use config_page::PAGE_LEN;

const USAGE: &str = "\
aula — control software for AULA keyboards

USAGE:
    aula                       open the window
    aula devices               list hidraw nodes and the profiles that match
    aula info                  identify the attached keyboard
    aula dump [len]            read the settings page and print it
    aula read <cmd> <param> <len>
                               raw read from the command channel
    aula poke <offset> <value> write one byte of the settings page
    aula reset                 restore the factory configuration
    aula udev                  print the udev rule needed for non-root access

ENVIRONMENT:
    AULA_ASSETS                path to the assets/ directory
";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = assets::root().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot find the assets directory.\n\
             Expected assets/ next to the binary, or point AULA_ASSETS at it."
        )
    })?;
    let profiles = profile::load_all(&root.join("devices"));

    match args.first().map(String::as_str) {
        None => run_gui(root, profiles),
        Some("devices") => cmd_devices(&profiles),
        Some("info") => cmd_info(&profiles),
        Some("dump") => cmd_dump(&profiles, args.get(1).and_then(|s| s.parse().ok())),
        Some("read") => cmd_read(&profiles, &args[1..]),
        Some("poke") => cmd_poke(&profiles, &args[1..]),
        Some("reset") => cmd_reset(&profiles),
        Some("udev") => {
            print!("{}", udev_rule(&profiles));
            Ok(())
        }
        Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => {
            eprintln!("aula: unknown command {other:?}\n");
            print!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn run_gui(root: PathBuf, profiles: Vec<profile::DeviceProfile>) -> anyhow::Result<()> {
    if profiles.is_empty() {
        eprintln!(
            "aula: no device profiles found under {}",
            root.join("devices").display()
        );
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([ui::theme::DESIGN_W, ui::theme::DESIGN_H])
            .with_min_inner_size([ui::theme::DESIGN_W, ui::theme::DESIGN_H])
            .with_resizable(false)
            .with_decorations(false)
            .with_title("AULA"),
        ..Default::default()
    };
    eframe::run_native(
        "AULA",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, root, profiles)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

fn cmd_devices(profiles: &[profile::DeviceProfile]) -> anyhow::Result<()> {
    println!("hidraw nodes:");
    for n in hid::enumerate() {
        let features: Vec<String> = n
            .feature_reports
            .iter()
            .map(|(id, len)| format!("{id}:{len}B"))
            .collect();
        let mark = if device::candidate_for(&n).is_some() {
            "*"
        } else {
            " "
        };
        println!(
            "{mark} {:<14} {:04x}:{:04x}  {:<28} feature[{}]",
            n.dev_path.display(),
            n.vid,
            n.pid,
            n.name,
            features.join(" ")
        );
    }
    println!("\n(* = carries the vendor command channel)\n");
    println!("profiles ({}):", profiles.len());
    for p in profiles {
        println!(
            "  {:<18} {:<14} {:04x}:{:04x}  psd={}  keys={} leds={}",
            p.id,
            p.short_name,
            p.vid,
            p.pid,
            device::fmt_password(&p.password),
            p.keys.len(),
            p.led_count()
        );
    }
    Ok(())
}

fn connect(profiles: &[profile::DeviceProfile]) -> anyhow::Result<device::Keyboard> {
    let (kb, problems) = device::autodetect(profiles);
    kb.ok_or_else(|| {
        let mut msg = String::from("no AULA keyboard found");
        for p in problems {
            msg.push_str(&format!("\n  {p}"));
        }
        anyhow::anyhow!(msg)
    })
}

fn cmd_info(profiles: &[profile::DeviceProfile]) -> anyhow::Result<()> {
    let kb = connect(profiles)?;
    println!("model      {}", kb.profile.name);
    println!("profile    {}", kb.profile.id);
    println!("node       {}", kb.link.node_path());
    println!("report id  {}", kb.link.report_id());
    println!("password   {}", device::fmt_password(&kb.password));
    println!(
        "keys/leds  {} / {}",
        kb.profile.keys.len(),
        kb.profile.led_count()
    );
    if kb.profile.show_power {
        match kb.link.power() {
            Ok(p) => println!("battery    {p:?}"),
            Err(e) => println!("battery    unavailable: {e}"),
        }
    }
    Ok(())
}

fn cmd_dump(profiles: &[profile::DeviceProfile], len: Option<usize>) -> anyhow::Result<()> {
    let kb = connect(profiles)?;
    let page = kb.link.read_config(len.unwrap_or(PAGE_LEN))?;
    print!("{}", hex_dump(&page));
    Ok(())
}

fn cmd_read(profiles: &[profile::DeviceProfile], args: &[String]) -> anyhow::Result<()> {
    if args.len() < 3 {
        anyhow::bail!("usage: aula read <cmd> <param> <len>");
    }
    let cmd = parse_u8(&args[0])?;
    let param = parse_u8(&args[1])?;
    let len: usize = args[2].parse()?;
    let kb = connect(profiles)?;
    let data = kb.link.read(cmd, param, len)?;
    print!("{}", hex_dump(&data));
    Ok(())
}

fn cmd_poke(profiles: &[profile::DeviceProfile], args: &[String]) -> anyhow::Result<()> {
    if args.len() < 2 {
        anyhow::bail!("usage: aula poke <offset> <value>");
    }
    let offset = parse_u8(&args[0])? as usize;
    let value = parse_u8(&args[1])?;
    let kb = connect(profiles)?;
    let mut page = kb.link.read_config(PAGE_LEN)?;
    page.resize(PAGE_LEN, 0);
    let before = page[offset];
    page[offset] = value;
    kb.link.write_config(&page)?;
    println!("[0x{offset:02x}] 0x{before:02x} -> 0x{value:02x}");
    let after = kb.link.read_config(PAGE_LEN)?;
    print!("{}", hex_dump(&after));
    Ok(())
}

fn cmd_reset(profiles: &[profile::DeviceProfile]) -> anyhow::Result<()> {
    let kb = connect(profiles)?;
    kb.link.reset()?;
    println!("factory reset sent to {}", kb.profile.name);
    Ok(())
}

fn parse_u8(s: &str) -> anyhow::Result<u8> {
    let v = ini::parse_num(s).ok_or_else(|| anyhow::anyhow!("not a number: {s:?}"))?;
    u8::try_from(v).map_err(|_| anyhow::anyhow!("{s} does not fit in a byte"))
}

fn hex_dump(data: &[u8]) -> String {
    let mut s = String::new();
    for (row, chunk) in data.chunks(16).enumerate() {
        s.push_str(&format!("{:04x}  ", row * 16));
        for b in chunk {
            s.push_str(&format!("{b:02x} "));
        }
        s.push('\n');
    }
    s
}

/// udev rule granting the logged-in user access to the command channel.
fn udev_rule(profiles: &[profile::DeviceProfile]) -> String {
    let mut ids: Vec<(u16, u16)> = profiles
        .iter()
        .flat_map(|p| {
            let mut v = vec![(p.vid, p.pid)];
            if let (Some(vid), Some(pid)) = (p.wireless_vid, p.wireless_pid) {
                v.push((vid, pid));
            }
            v
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();

    // The 60- prefix is load-bearing: systemd grants the uaccess ACL from
    // 73-seat-late.rules, and udev runs rule files in ascending order, so a
    // file numbered above 73 tags the device too late to get one.
    let mut out = String::from(
        "# AULA keyboards — allow the seated user to reach the vendor HID channel.\n\
         # Install as /etc/udev/rules.d/60-aula.rules — the number must stay below 73.\n\
         #   sudo udevadm control --reload && sudo udevadm trigger\n",
    );
    for (vid, pid) in ids {
        out.push_str(&format!(
            "KERNEL==\"hidraw*\", ATTRS{{idVendor}}==\"{vid:04x}\", ATTRS{{idProduct}}==\"{pid:04x}\", TAG+=\"uaccess\", MODE=\"0660\"\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_dump_is_16_bytes_per_row() {
        let d: Vec<u8> = (0..20).collect();
        let s = hex_dump(&d);
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("0000  00 01 02"));
        assert!(lines[1].starts_with("0010  10 11 12 13"));
    }

    #[test]
    fn parses_decimal_and_hex_arguments() {
        assert_eq!(parse_u8("10").unwrap(), 10);
        assert_eq!(parse_u8("0x0a").unwrap(), 10);
        assert!(parse_u8("300").is_err());
        assert!(parse_u8("nope").is_err());
    }

    #[test]
    fn udev_rule_covers_wired_and_wireless_ids_once() {
        let mut a = fake_profile([3, 0, 0, 0, 0, 0xcd]);
        let mut b = fake_profile([3, 0, 0, 0, 3, 0x7c]);
        a.wireless_vid = Some(0x3554);
        a.wireless_pid = Some(0xfa09);
        b.wireless_vid = Some(0x3554);
        b.wireless_pid = Some(0xfa09);
        let rule = udev_rule(&[a, b]);
        assert_eq!(rule.matches("258a").count(), 1, "duplicate ids collapse");
        assert_eq!(rule.matches("3554").count(), 1);
        assert!(rule.contains("uaccess"));
    }

    fn fake_profile(password: [u8; 6]) -> profile::DeviceProfile {
        let text = format!(
            "[OPT]\nVID=0x258a\nPID=0x010C\nPsd={}\nShortName=X\n",
            password
                .iter()
                .map(|b| format!("{b:x}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        profile::DeviceProfile::from_text("x", std::path::Path::new("."), &text).unwrap()
    }
}
