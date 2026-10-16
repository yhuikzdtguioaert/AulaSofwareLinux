mod assets;
mod config_page;
mod device;
mod hid;
mod ini;
mod profile;
mod proto;
mod remap;
mod ui;
mod worker;

use std::path::PathBuf;

const USAGE: &str = "aula [devices | info | --help]\n";

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
