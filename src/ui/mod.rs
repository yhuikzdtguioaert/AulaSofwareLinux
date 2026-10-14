//! The application window.
//!
//! Chrome, palette, artwork and strings are the vendor's own, so the layout
//! matches the Windows original: a 1200x720 undecorated window, `main_nr.png`
//! as the background, the 50x50 navigation icons in the title bar and the
//! keyboard drawn from `keyimg.png`.

pub mod keyboard;
pub mod theme;
pub mod widgets;

use std::collections::BTreeSet;
use std::path::PathBuf;

use egui::{Color32, Rect, Sense, Vec2};

use crate::assets::{Skins, Strings};
use crate::config_page::{ConfigPage, Layout, COLORS, COLOR_RANDOM, PAGE_LEN};
use crate::profile::DeviceProfile;
use crate::worker::{Event, Request, Worker};
use keyboard::KeyboardView;
use theme::{Theme, CONTENT_X, DESIGN_H, DESIGN_W, NAVI_SIZE, NAVI_STEP, TITLE_H};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Light,
    Settings,
    Diagnostics,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Light, Tab::Settings, Tab::Diagnostics];

    /// Which vendor navigation icon (`navi<N>_nr.png`) stands for this page.
    /// The numbering follows the original artwork, not the tab order, so the
    /// bulb stays the bulb even though it now comes first.
    fn navi_index(self) -> usize {
        match self {
            Tab::Light => 3,
            Tab::Settings => 1,
            Tab::Diagnostics => 4,
        }
    }

    fn title(self, s: &Strings) -> String {
        match self {
            Tab::Light => s.get("tc_kb1", "Light effect").to_string(),
            Tab::Settings => s.get("tc_msg1", "Global").to_string(),
            Tab::Diagnostics => "Protocol".to_string(),
        }
    }
}

pub struct App {
    profiles: Vec<DeviceProfile>,
    worker: Worker,
    strings: Strings,
    skins: Skins,
    theme: Theme,
    assets_root: PathBuf,

    tab: Tab,
    connected: Option<Connected>,
    keyboard: Option<KeyboardView>,
    page: ConfigPage,
    rgb: Vec<u8>,
    power: Option<Vec<u8>>,
    selection: BTreeSet<u32>,
    last_key: Option<u32>,
    paint_color: [u8; 3],

    status: String,
    error: Option<String>,
    log: Vec<String>,
    /// Offset/value pair for the byte poker in the diagnostics tab.
    poke: (usize, u8),
    raw: (u8, u8, usize),
}

struct Connected {
    profile: DeviceProfile,
    display_name: String,
    password: [u8; 6],
    report_id: u8,
    node: String,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        assets_root: PathBuf,
        profiles: Vec<DeviceProfile>,
    ) -> App {
        let strings = Strings::load(&assets_root.join("text/en.xml"));
        let theme = Theme::from_cfg(&assets_root.join("Cfg.ini"));
        theme.install(&cc.egui_ctx);

        let worker = Worker::spawn(profiles.clone());
        worker.send(Request::Connect);

        App {
            profiles,
            worker,
            strings,
            skins: Skins::new(&assets_root),
            theme,
            assets_root,
            tab: Tab::Light,
            connected: None,
            keyboard: None,
            page: ConfigPage::blank(Layout::default()),
            rgb: Vec::new(),
            power: None,
            selection: BTreeSet::new(),
            last_key: None,
            paint_color: [255, 0, 0],
            status: "Looking for a keyboard…".into(),
            error: None,
            log: Vec::new(),
            poke: (0x0a, 1),
            raw: (crate::proto::cmd::GET_LED, 0, PAGE_LEN),
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        for ev in self.worker.poll() {
            match ev {
                Event::Connected {
                    profile_id,
                    display_name,
                    password,
                    report_id,
                    node,
                } => {
                    let profile = self.profiles.iter().find(|p| p.id == profile_id).cloned();
                    if let Some(profile) = profile {
                        self.page = ConfigPage::blank(Layout::load_override(&profile.dir));
                        self.keyboard = Some(KeyboardView::load(ctx, &profile));
                        self.rgb = vec![0; profile.led_count() * 3];
                        // Kept short: the side panel already carries the full name.
                        self.status = format!("{} · {}", profile.short_name, node);
                        self.connected = Some(Connected {
                            profile,
                            display_name,
                            password,
                            report_id,
                            node,
                        });
                        self.error = None;
                    }
                }
                Event::Disconnected(msg) => {
                    self.connected = None;
                    self.keyboard = None;
                    self.status = msg;
                }
                Event::Config(bytes) => {
                    let layout = self.page.layout;
                    self.page = ConfigPage::new(bytes, layout);
                }
                Event::RgbTable(bytes) => self.rgb = bytes,
                Event::Power(p) => self.power = Some(p),
                Event::RawData { command, data } => {
                    self.log.push(format!("0x{command:02x}: {}", hex(&data)));
                }
                Event::Notice(m) => self.status = m,
                Event::Error(m) => {
                    self.error = Some(m.clone());
                    self.log.push(format!("error: {m}"));
                }
                Event::Log(m) => self.log.push(m),
            }
        }
        if self.log.len() > 400 {
            let cut = self.log.len() - 400;
            self.log.drain(..cut);
        }
    }

    fn profile(&self) -> Option<&DeviceProfile> {
        self.connected.as_ref().map(|c| &c.profile)
    }

    /// Send the settings page if anything changed.
    ///
    /// Refuses to write a page that does not carry the keyboard's own end
    /// marker: that means the read failed and the buffer is still zeroed, and
    /// pushing zeroes would wipe the device's settings.
    fn apply(&mut self) {
        if !self.page.dirty {
            return;
        }
        if !self.page.looks_valid() {
            self.error = Some(
                "refusing to write: the settings page was never read back from the keyboard".into(),
            );
            return;
        }
        self.worker
            .send(Request::WriteConfig(self.page.bytes.clone()));
        self.page.dirty = false;
    }

    /// Label for a hardware effect. The vendor translations leave the tag for
    /// the "lighting off" row blank, so that one is named from `tc_kb_led20`.
    fn effect_label(&self, opt: &crate::profile::LedOpt) -> String {
        if opt.is_off() {
            self.strings.get("tc_kb_led20", "OFF").to_string()
        } else {
            self.strings.effect_name(opt.ui_index)
        }
    }

    /// Colour currently assigned to `key`, from the per-key RGB table.
    fn key_color(rgb: &[u8], key: &crate::profile::KeyDef) -> Option<Color32> {
        if key.led < 0 {
            return None;
        }
        let at = key.led as usize * 3;
        let s = rgb.get(at..at + 3)?;
        if s == [0, 0, 0] {
            None
        } else {
            Some(Color32::from_rgb(s[0], s[1], s[2]))
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.drain_events(&ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(250));

        // eframe hands us a Ui with no margin and no background, which is
        // exactly what a fully skinned window wants.
        let full = ui.max_rect();
        match self.skins.get(&ctx, "main_nr.png") {
            Some(tex) => {
                ui.painter().image(
                    tex.id(),
                    full,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            None => {
                ui.painter()
                    .rect_filled(full, 0.0, Color32::from_rgb(24, 24, 24));
            }
        }
        self.title_bar(ui, full);
        self.side_panel(ui, full);

        let content = Rect::from_min_max(
            egui::pos2(CONTENT_X + 20.0, TITLE_H + 20.0),
            egui::pos2(DESIGN_W - 20.0, DESIGN_H - 56.0),
        );
        egui::Area::new(egui::Id::new(("tab", self.tab as usize)))
            .fixed_pos(content.min)
            .order(egui::Order::Middle)
            .show(&ctx, |ui| {
                ui.set_min_size(content.size());
                ui.set_max_size(content.size());
                match self.tab {
                    Tab::Light => self.tab_light(ui, content),
                    Tab::Settings => self.tab_settings(ui),
                    Tab::Diagnostics => self.tab_diagnostics(ui),
                }
            });

        self.status_bar(&ctx);
    }
}

impl App {
    fn title_bar(&mut self, ui: &mut egui::Ui, full: Rect) {
        let bar = Rect::from_min_size(full.min, Vec2::new(full.width(), TITLE_H));

        // Dragging the title bar moves the window, like the original.
        //
        // Only on drag_started: once the compositor takes the window over, it
        // swallows the button release, so egui keeps reporting the drag as
        // active and repeating the command would move the window forever.
        let drag_zone = Rect::from_min_max(
            egui::pos2(bar.min.x + 560.0, bar.min.y),
            egui::pos2(bar.max.x - 90.0, bar.max.y),
        );
        let dragged = ui
            .interact(drag_zone, ui.id().with("drag"), Sense::click_and_drag())
            .drag_started_by(egui::PointerButton::Primary);
        if dragged {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        for (i, tab) in Tab::ALL.iter().enumerate() {
            let pos = egui::pos2(
                full.min.x + self.theme.navi.x + i as f32 * NAVI_STEP,
                full.min.y + self.theme.navi.y,
            );
            let rect = Rect::from_min_size(pos, Vec2::splat(NAVI_SIZE));
            let r = ui.interact(rect, ui.id().with(("navi", i)), Sense::click());
            let active = *tab == self.tab;
            let name = format!(
                "navi50x50/navi{}_{}.png",
                tab.navi_index(),
                if active || r.hovered() { "dn" } else { "nr" }
            );
            if let Some(tex) = self.skins.get(ui.ctx(), &name) {
                ui.painter().image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                ui.painter().rect_filled(
                    rect,
                    4.0,
                    if active {
                        self.theme.list_item
                    } else {
                        self.theme.menu_bg
                    },
                );
            }
            if r.clicked() {
                self.tab = *tab;
            }
            r.on_hover_text(tab.title(&self.strings));
        }

        // Minimise / close, using the vendor's own 18x18 icons.
        let buttons = [
            ("icon18x18/min_nr.png", "icon18x18/min_ov.png", false),
            ("icon18x18/exit_nr.png", "icon18x18/exit_ov.png", true),
        ];
        for (i, (nr, ov, is_close)) in buttons.iter().enumerate() {
            let rect = Rect::from_min_size(
                egui::pos2(full.max.x - 78.0 + i as f32 * 34.0, full.min.y + 22.0),
                Vec2::splat(20.0),
            );
            let r = ui.interact(rect, ui.id().with(("chrome", i)), Sense::click());
            let name = if r.hovered() { *ov } else { *nr };
            if let Some(tex) = self.skins.get(ui.ctx(), name) {
                ui.painter().image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            if r.clicked() {
                ui.ctx().send_viewport_cmd(if *is_close {
                    egui::ViewportCommand::Close
                } else {
                    egui::ViewportCommand::Minimized(true)
                });
            }
        }
    }

    /// The narrow column the original uses for the device and its state.
    /// The narrow column the original uses for the device and its state.
    fn side_panel(&mut self, ui: &mut egui::Ui, full: Rect) {
        let x = full.min.x + theme::RAIL_X + 24.0;
        let width = CONTENT_X - theme::RAIL_X - 48.0;

        // Product picture, as the original shows above the device details.
        if let Some(view) = &self.keyboard {
            let area = Rect::from_min_size(
                egui::pos2(x, full.min.y + TITLE_H + 24.0),
                Vec2::new(width, 92.0),
            );
            let r = view.fit(area);
            if let Some(tex) = &view.texture {
                ui.painter().image(
                    tex.id(),
                    r,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }

        let mut y = full.min.y + TITLE_H + 132.0;
        let painter = ui.painter();
        let font = egui::FontId::proportional(15.0);
        let small = egui::FontId::proportional(12.0);

        let (title, lines) = match &self.connected {
            Some(c) => {
                let mut lines = vec![
                    format!(
                        "{}: {}",
                        self.strings.get("tc_msg2", "Device model"),
                        c.profile.short_name
                    ),
                    format!(
                        "{}: 0x{:02x}",
                        self.strings.get("tc_msg3", "Firmware"),
                        c.profile.fw
                    ),
                    format!("Report id: 0x{:02x}", c.report_id),
                    format!("Node: {}", c.node),
                    format!("Psd: {}", crate::device::fmt_password(&c.password)),
                ];
                if let Some(p) = &self.power {
                    lines.push(format!(
                        "{}: {}",
                        self.strings.get("tc_msg5", "Battery"),
                        p.first().copied().unwrap_or(0)
                    ));
                }
                (c.display_name.clone(), lines)
            }
            None => (
                self.strings
                    .get("tc_msg35", "Please connect your device.")
                    .to_string(),
                vec![format!("{} profiles loaded", self.profiles.len())],
            ),
        };

        painter.text(
            egui::pos2(x, y),
            egui::Align2::LEFT_TOP,
            title,
            font,
            self.theme.button_text_hover,
        );
        y += 30.0;
        for line in lines {
            painter.text(
                egui::pos2(x, y),
                egui::Align2::LEFT_TOP,
                line,
                small.clone(),
                self.theme.text,
            );
            y += 20.0;
        }

        // Kept clear of the status line along the bottom edge.
        let buttons = Rect::from_min_size(
            egui::pos2(x, full.max.y - 130.0),
            Vec2::new(widgets::BUTTON.x, 70.0),
        );
        widgets::at(
            ui,
            buttons,
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                if widgets::button(ui, &mut self.skins, &self.theme, "Rescan", true).clicked() {
                    self.worker.send(Request::Connect);
                }
                ui.add_space(6.0);
                if widgets::button(ui, &mut self.skins, &self.theme, "Re-read", true).clicked() {
                    self.worker.send(Request::Refresh);
                }
            },
        );
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let text = match &self.error {
            Some(e) => format!("⚠ {e}"),
            None => self.status.clone(),
        };
        let color = if self.error.is_some() {
            Color32::from_rgb(230, 120, 90)
        } else {
            self.theme.text
        };
        egui::Area::new(egui::Id::new("status"))
            .fixed_pos(egui::pos2(theme::RAIL_X + 24.0, DESIGN_H - 28.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.colored_label(color, text);
            });
    }

    fn tab_light(&mut self, ui: &mut egui::Ui, content: Rect) {
        let Some(profile) = self.profile().cloned() else {
            self.empty_page(ui, content);
            return;
        };

        let x0 = content.min.x;
        let y0 = content.min.y;
        self.page_title(ui, content, self.strings.get("tc_kb1", "Light effect"));

        // The keyboard picture sits across the top, as in the original.
        let kb_area = Rect::from_min_max(
            egui::pos2(x0, y0 + 30.0),
            egui::pos2(content.max.x, y0 + 326.0),
        );
        if let Some(view) = &self.keyboard {
            let rgb = self.rgb.clone();
            let r = view.show(ui, kb_area, &profile, &self.selection, |k| {
                App::key_color(&rgb, k)
            });
            if let Some(idx) = r.clicked {
                self.last_key = Some(idx);
                if ui.input(|i| i.modifiers.ctrl) {
                    if !self.selection.insert(idx) {
                        self.selection.remove(&idx);
                    }
                } else {
                    self.selection.clear();
                    self.selection.insert(idx);
                }
            }
        }

        // The page stores the effect as its 1-based position in the profile's
        // list, and the settings table is indexed by that minus one.
        let effect = self.page.effect();
        let opt = profile
            .led_opts
            .iter()
            .find(|o| o.ui_index == effect)
            .copied();
        let slot = ConfigPage::slot_of(effect);
        let cur = self.page.current();

        // Effect list, bottom left.
        let list = Rect::from_min_size(
            egui::pos2(x0, y0 + 340.0),
            Vec2::new(widgets::LIST_W, 216.0),
        );
        widgets::list_frame(ui, &self.theme, list);
        let mut pick_effect = None;
        widgets::at(
            ui,
            list.shrink(2.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("effects")
                    .show(ui, |ui| {
                        for o in &profile.led_opts {
                            let label = self.effect_label(o);
                            let row = widgets::list_row(
                                ui,
                                &self.theme,
                                widgets::LIST_W - 4.0,
                                &label,
                                o.ui_index == effect,
                            );
                            if row.clicked() {
                                pick_effect = Some(o.ui_index);
                            }
                        }
                    });
            },
        );
        if let Some(v) = pick_effect {
            self.page.set_effect(v);
            self.apply();
        }

        // Controls, bottom right.
        let cx = list.max.x + 40.0;
        let sx = cx + 96.0;
        let sw = (content.max.x - sx - 8.0).min(450.0);
        let mut row_y = list.min.y + 12.0;
        let row = |y: f32| Rect::from_min_size(egui::pos2(sx, y - 9.0), Vec2::new(sw, 18.0));

        widgets::caption(
            ui,
            &self.theme,
            egui::pos2(cx, row_y),
            self.strings.get("tc_msg22", "Brightness"),
        );
        let mut b = profile.brightness_ui(cur.brightness);
        let max = profile.brightness_levels().saturating_sub(1);
        let enabled = opt.map(|o| o.has_light).unwrap_or(true);
        let (changed, resp) = widgets::at(
            ui,
            row(row_y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| widgets::slider(ui, &mut self.skins, &self.theme, sw, &mut b, max, enabled),
        );
        if changed {
            self.page.set_brightness(slot, profile.brightness_hw(b));
        }
        if widgets::settled(&resp) {
            self.apply();
        }
        ui.painter().text(
            egui::pos2(sx + sw + 4.0, row_y),
            egui::Align2::LEFT_CENTER,
            b.to_string(),
            egui::FontId::proportional(12.0),
            self.theme.text,
        );

        row_y += 38.0;
        widgets::caption(
            ui,
            &self.theme,
            egui::pos2(cx, row_y),
            self.strings.get("tc_msg23", "Speed"),
        );
        let mut sp = profile.speed_ui(cur.speed);
        let max = profile.speed_levels().saturating_sub(1);
        let enabled = opt.map(|o| o.has_speed).unwrap_or(true);
        let (changed, resp) = widgets::at(
            ui,
            row(row_y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| widgets::slider(ui, &mut self.skins, &self.theme, sw, &mut sp, max, enabled),
        );
        if changed {
            self.page.set_speed(slot, profile.speed_hw(sp));
        }
        if widgets::settled(&resp) {
            self.apply();
        }
        ui.painter().text(
            egui::pos2(sx + sw + 4.0, row_y),
            egui::Align2::LEFT_CENTER,
            sp.to_string(),
            egui::FontId::proportional(12.0),
            self.theme.text,
        );

        // Colour slots. A click applies straight away — the keyboard is the
        // preview, so waiting for a Save button only hides what a colour does.
        row_y += 38.0;
        widgets::caption(
            ui,
            &self.theme,
            egui::pos2(cx, row_y),
            self.strings.get("tc_msg24", "Color"),
        );
        let color_enabled = opt.map(|o| o.has_color).unwrap_or(true);
        let mut pick_color = None;
        widgets::at(
            ui,
            Rect::from_min_size(egui::pos2(sx, row_y - 12.0), Vec2::new(sw, 26.0)),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (i, (name, c)) in COLORS.iter().enumerate() {
                    let r = widgets::swatch(ui, &self.theme, *c, cur.color == i as u8, 22.0);
                    if color_enabled && r.clicked() {
                        pick_color = Some(i as u8);
                    }
                    r.on_hover_text(*name);
                }
                ui.add_space(6.0);
                let r = widgets::list_row(
                    ui,
                    &self.theme,
                    90.0,
                    self.strings.get("tc_msg25", "Colourful"),
                    cur.color == COLOR_RANDOM,
                );
                if color_enabled && r.clicked() {
                    pick_color = Some(COLOR_RANDOM);
                }
            },
        );
        if let Some(v) = pick_color {
            self.page.set_color(slot, v);
            self.apply();
        }

        // Per-key colours, written with their own command.
        row_y += 42.0;
        widgets::caption(
            ui,
            &self.theme,
            egui::pos2(cx, row_y),
            self.strings.get("tc_msg14", "Custom color"),
        );
        let wheel = Rect::from_min_size(egui::pos2(sx, row_y - 10.0), Vec2::splat(96.0));
        let (_, picked) = widgets::at(
            ui,
            wheel,
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| widgets::color_wheel(ui, &mut self.skins, &self.theme, 96.0),
        );
        if let Some(c) = picked {
            self.paint_color = c;
        }
        widgets::at(
            ui,
            Rect::from_min_size(
                egui::pos2(sx + 110.0, row_y - 10.0),
                Vec2::new(sw - 110.0, 96.0),
            ),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                widgets::swatch(ui, &self.theme, self.paint_color, false, 26.0);
                ui.add_space(6.0);
                let can = !self.selection.is_empty() && !self.rgb.is_empty();
                if widgets::button(
                    ui,
                    &mut self.skins,
                    &self.theme,
                    self.strings.get("tc_kb10", "Current"),
                    can,
                )
                .clicked()
                    && can
                {
                    for idx in self.selection.clone() {
                        if let Some(k) = profile.keys.iter().find(|k| k.index == idx) {
                            if k.led >= 0 {
                                let at = k.led as usize * 3;
                                if at + 3 <= self.rgb.len() {
                                    self.rgb[at..at + 3].copy_from_slice(&self.paint_color);
                                }
                            }
                        }
                    }
                    self.worker.send(Request::WriteRgbTable(self.rgb.clone()));
                }
                ui.add_space(6.0);
                let any = !self.rgb.is_empty();
                if widgets::button(ui, &mut self.skins, &self.theme, "Fill all", any).clicked()
                    && any
                {
                    for chunk in self.rgb.chunks_mut(3) {
                        chunk.copy_from_slice(&self.paint_color);
                    }
                    self.worker.send(Request::WriteRgbTable(self.rgb.clone()));
                }
            },
        );

        widgets::caption(
            ui,
            &self.theme,
            egui::pos2(x0, content.max.y - 6.0),
            self.strings
                .get("tc_kb4", "Hold ctrl to pick more than one key"),
        );
    }

    /// Heading drawn in the same place on every page.
    fn page_title(&self, ui: &egui::Ui, content: Rect, text: &str) {
        ui.painter().text(
            egui::pos2(content.min.x, content.min.y),
            egui::Align2::LEFT_TOP,
            text,
            egui::FontId::proportional(17.0),
            self.theme.button_text_hover,
        );
    }

    fn empty_page(&self, ui: &egui::Ui, content: Rect) {
        ui.painter().text(
            egui::pos2(content.min.x, content.min.y + 40.0),
            egui::Align2::LEFT_TOP,
            self.strings.get("tc_msg35", "Please connect your device."),
            egui::FontId::proportional(14.0),
            self.theme.text,
        );
    }

    fn tab_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading(self.strings.get("tc_msg45", "Device Info"));
        ui.add_space(8.0);

        let Some(profile) = self.profile().cloned() else {
            ui.label(self.strings.get("tc_msg35", "Please connect your device."));
            ui.add_space(12.0);
            ui.label(format!("Assets: {}", self.assets_root.display()));
            ui.label(format!("{} device profiles loaded", self.profiles.len()));
            ui.add_space(8.0);
            ui.label(
                "If the keyboard is plugged in but not listed, /dev/hidraw* is probably \
                 root-only. Install udev/60-aula.rules and replug it.",
            );
            return;
        };

        egui::Grid::new("dev-info")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label(self.strings.get("tc_msg2", "Device model"));
                ui.label(&profile.name);
                ui.end_row();
                ui.label("USB");
                ui.label(format!("{:04x}:{:04x}", profile.vid, profile.pid));
                ui.end_row();
                if let (Some(v), Some(p)) = (profile.wireless_vid, profile.wireless_pid) {
                    ui.label("Wireless USB");
                    ui.label(format!("{v:04x}:{p:04x}"));
                    ui.end_row();
                }
                ui.label("Password");
                ui.label(crate::device::fmt_password(&profile.password));
                ui.end_row();
                ui.label(self.strings.get("tc_msg3", "Firmware version"));
                ui.label(format!("0x{:02x}", profile.fw));
                ui.end_row();
                ui.label("Keys / LEDs");
                ui.label(format!("{} / {}", profile.keys.len(), profile.led_count()));
                ui.end_row();
                ui.label("Matrix length");
                ui.label(profile.matrix_len.to_string());
                ui.end_row();
            });

        ui.add_space(16.0);
        ui.label(self.strings.get("tc_msg34", "Sleep after idle"));
        if !profile.sleep_ui.is_empty() {
            let current = self.page.sleep();
            let label = profile
                .sleep_hw
                .iter()
                .position(|h| *h as u8 == current)
                .and_then(|i| profile.sleep_ui.get(i))
                .map(|s| format!("{s} s"))
                .unwrap_or_else(|| format!("0x{current:02x}"));
            egui::ComboBox::from_id_salt("sleep")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for s in profile.sleep_ui.clone() {
                        let hw = profile.sleep_hw(s);
                        if ui
                            .selectable_label(current == hw, format!("{s} s"))
                            .clicked()
                        {
                            self.page.set_sleep(hw);
                        }
                    }
                });
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(self.strings.get("tc_msg54", "Debounce"));
            let mut d = self.page.debounce();
            if ui
                .add(egui::Slider::new(&mut d, 0..=30).suffix(" ms"))
                .changed()
            {
                self.page.set_debounce(d);
            }
        });

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.page.dirty,
                    egui::Button::new(self.strings.get("tc_apply", "Save")),
                )
                .clicked()
            {
                self.apply();
            }
            if ui
                .button(
                    self.strings
                        .get("tc_msg21", "Restore the device to the factory settings"),
                )
                .clicked()
            {
                self.worker.send(Request::FactoryReset);
            }
        });
    }

    fn tab_diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.heading("Protocol");
        ui.label(
            "Direct access to the vendor command channel. Reads are harmless; writes go \
             straight to the keyboard.",
        );
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("cmd");
            ui.add(egui::DragValue::new(&mut self.raw.0).hexadecimal(2, false, false));
            ui.label("param");
            ui.add(egui::DragValue::new(&mut self.raw.1).hexadecimal(2, false, false));
            ui.label("len");
            ui.add(egui::DragValue::new(&mut self.raw.2).range(1..=4096));
            if ui.button("Read").clicked() {
                self.worker.send(Request::RawRead {
                    command: self.raw.0,
                    parameter: self.raw.1,
                    len: self.raw.2,
                });
            }
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Poke settings byte");
            ui.add(egui::DragValue::new(&mut self.poke.0).range(0..=PAGE_LEN - 1));
            ui.label("=");
            ui.add(egui::DragValue::new(&mut self.poke.1));
            if ui.button("Write & read back").clicked() {
                self.worker.send(Request::PokeConfig {
                    offset: self.poke.0,
                    value: self.poke.1,
                });
            }
        });

        ui.add_space(12.0);
        ui.label("Settings page");
        egui::ScrollArea::vertical()
            .id_salt("page")
            .max_height(150.0)
            .show(ui, |ui| {
                ui.monospace(self.page.hex_dump());
            });

        ui.add_space(8.0);
        ui.label("Log");
        egui::ScrollArea::vertical()
            .id_salt("log")
            .max_height(160.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
            });
    }
}

fn hex(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_keeps_its_own_vendor_icon() {
        // The icons are the original artwork, so the numbers are fixed to the
        // meaning of each page rather than to its position in the bar.
        assert_eq!(Tab::Light.navi_index(), 3, "the bulb");
        assert_eq!(Tab::Settings.navi_index(), 1, "the sliders");
        assert_eq!(Tab::Diagnostics.navi_index(), 4, "the code window");
        let mut icons: Vec<usize> = Tab::ALL.iter().map(|t| t.navi_index()).collect();
        icons.sort_unstable();
        icons.dedup();
        assert_eq!(icons.len(), Tab::ALL.len(), "no two pages share an icon");
    }

    #[test]
    fn hex_formats_bytes() {
        assert_eq!(hex(&[0x00, 0x0a, 0xff]), "00 0a ff");
    }
}
