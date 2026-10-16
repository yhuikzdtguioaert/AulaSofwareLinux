pub mod keyboard;
pub mod theme;
pub mod widgets;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Vec2};

use crate::assets::{Skins, Strings};
use crate::config_page::{ConfigPage, Layout, COLORS, COLOR_RANDOM};
use crate::profile::DeviceProfile;
use crate::worker::{Event, Request, Worker};
use keyboard::KeyboardView;
use theme::{Theme, CONTENT_X, DESIGN_H, DESIGN_W, NAVI_SIZE, NAVI_STEP, TITLE_H};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Light,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Light, Tab::Settings];

    /// Which vendor navigation icon (`navi<N>_nr.png`) stands for this page.
    /// The numbering follows the original artwork, not the tab order, so the
    /// bulb stays the bulb even though it now comes first.
    fn navi_index(self) -> usize {
        match self {
            Tab::Light => 3,
            Tab::Settings => 1,
        }
    }

    fn title(self, s: &Strings) -> String {
        match self {
            Tab::Light => s.get("tc_kb1", "Light effect").to_string(),
            Tab::Settings => s.get("tc_msg1", "Global").to_string(),
        }
    }
}

pub struct App {
    profiles: Vec<Arc<DeviceProfile>>,
    worker: Worker,
    strings: Strings,
    skins: Skins,
    theme: Theme,

    tab: Tab,
    connected: Option<Connected>,
    keyboard: Option<KeyboardView>,
    page: ConfigPage,
    rgb: Vec<u8>,
    power: Option<Vec<u8>>,
    selection: BTreeSet<u32>,
    paint_color: [u8; 3],

    status: String,
    error: Option<String>,
    knob_action: Option<[u8; 4]>,
    wireless: bool,
}

struct Connected {
    profile: Arc<DeviceProfile>,
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
            profiles: profiles.into_iter().map(Arc::new).collect(),
            worker,
            strings,
            skins: Skins::new(&assets_root),
            theme,
            tab: Tab::Light,
            connected: None,
            keyboard: None,
            page: ConfigPage::blank(Layout::default()),
            rgb: Vec::new(),
            power: None,
            selection: BTreeSet::new(),
            paint_color: [255, 0, 0],
            status: String::new(),
            error: None,
            knob_action: None,
            wireless: false,
        }
    }

    fn drain_events(&mut self, ctx: &egui::Context) {
        for ev in self.worker.poll() {
            match ev {
                Event::Connected {
                    profile_id,
                    wireless,
                } => {
                    if let Some(profile) =
                        self.profiles.iter().find(|p| p.id == profile_id).cloned()
                    {
                        self.page = ConfigPage::blank(Layout::load_override(&profile.dir));
                        self.keyboard = Some(KeyboardView::load(ctx, &profile));
                        self.rgb.clear();
                        self.power = None;
                        self.knob_action = None;
                        self.selection.clear();
                        self.connected = Some(Connected { profile });
                        self.wireless = wireless;
                        self.status.clear();
                        self.error = None;
                    }
                }
                Event::Disconnected(msg) => {
                    self.connected = None;
                    self.keyboard = None;
                    self.rgb.clear();
                    self.power = None;
                    self.knob_action = None;
                    self.error = None;
                    self.status = msg;
                }
                Event::Config(bytes) => {
                    let layout = self.page.layout;
                    self.page = ConfigPage::new(bytes, layout);
                }
                Event::SavedConfig { sent, bytes } => {
                    if self.page.bytes == sent {
                        self.page = ConfigPage::new(bytes, self.page.layout);
                    }
                }
                Event::RgbTable(bytes) => self.rgb = bytes,
                Event::Power(p) => self.power = Some(p),
                Event::Knob(action) => self.knob_action = Some(action),
                Event::Notice(message) => {
                    self.status = message;
                    self.error = None;
                }
                Event::Error(message) => {
                    self.error = Some(message);
                    self.page.dirty = true;
                }
            }
        }
    }

    fn profile(&self) -> Option<Arc<DeviceProfile>> {
        self.connected.as_ref().map(|c| Arc::clone(&c.profile))
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
            self.error = Some("Settings are not ready. Reconnect the keyboard.".into());
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
                ui.add_enabled_ui(self.page.looks_valid(), |ui| match self.tab {
                    Tab::Light => self.tab_light(ui, content),
                    Tab::Settings => self.tab_settings(ui),
                });
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

    /// Device summary beside the keyboard artwork.
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
                let mut lines = vec![if self.wireless {
                    "2.4 GHz".into()
                } else {
                    "USB".into()
                }];
                if let Some(level) = self
                    .power
                    .as_ref()
                    .and_then(|p| p.first())
                    .filter(|level| **level <= 100)
                {
                    lines.push(format!("Battery: {level}%"));
                }
                (format!("AULA {}", c.profile.short_name), lines)
            }
            None => (String::new(), Vec::new()),
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
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let text = match &self.error {
            Some(e) => e.clone(),
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
        let Some(profile) = self.profile() else {
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
            let rgb = &self.rgb;
            let custom = self.page.is_custom();
            let r = view.show(ui, kb_area, &profile, &self.selection, |k| {
                if custom {
                    App::key_color(rgb, k)
                } else {
                    None
                }
            });
            if let Some(idx) = r.clicked {
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
        let mut pick_custom = false;
        widgets::at(
            ui,
            list.shrink(2.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("effects")
                    .show(ui, |ui| {
                        if !self.wireless {
                            pick_custom = widgets::list_row(
                                ui,
                                &self.theme,
                                widgets::LIST_W - 4.0,
                                "Custom",
                                self.page.is_custom(),
                            )
                            .clicked();
                        }
                        for o in &profile.led_opts {
                            let label = self.effect_label(o);
                            let row = widgets::list_row(
                                ui,
                                &self.theme,
                                widgets::LIST_W - 4.0,
                                &label,
                                o.ui_index == effect && !self.page.is_custom(),
                            );
                            if row.clicked() {
                                pick_effect = Some(o.ui_index);
                            }
                        }
                    });
            },
        );
        if pick_custom && !self.rgb.is_empty() {
            self.worker.send(Request::WriteRgbTable(self.rgb.clone()));
        }
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
        let enabled = self.page.is_custom() || opt.is_some_and(|o| o.has_light);
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
        let enabled = !self.page.is_custom() && opt.is_some_and(|o| o.has_speed);
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
        let color_enabled = !self.page.is_custom() && opt.is_some_and(|o| o.has_color);
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
                if !self.page.is_custom() && opt.is_some_and(|o| o.has_random) && r.clicked() {
                    pick_color = Some(COLOR_RANDOM);
                }
            },
        );
        if let Some(v) = pick_color {
            self.page.set_color(slot, v);
            self.apply();
        }

        if self.wireless {
            return;
        }

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
                let can = !self.wireless && !self.selection.is_empty() && !self.rgb.is_empty();
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
                    for idx in &self.selection {
                        if let Some(k) = profile.keys.iter().find(|k| k.index == *idx) {
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
                let any = !self.wireless && !self.rgb.is_empty();
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
        let Some(profile) = self.profile() else {
            return;
        };
        ui.heading(self.strings.get("tc_msg1", "Global"));
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
                .unwrap_or_else(|| if current == 0 { "Never" } else { "Custom" }.to_string());
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

        if let Some(current) = self.knob_action {
            ui.add_space(24.0);
            ui.label("Knob press");
            let actions = crate::worker::KNOB_ACTIONS;
            let label = actions
                .iter()
                .find(|(_, action)| *action == current)
                .map(|(name, _)| *name)
                .unwrap_or("Custom");
            egui::ComboBox::from_id_salt("knob-action")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (name, action) in actions {
                        if ui.selectable_label(current == action, name).clicked() {
                            self.worker.send(Request::SetKnob(action));
                        }
                    }
                });
        }

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
                .add_enabled(!self.wireless, egui::Button::new("Restore defaults"))
                .clicked()
            {
                self.worker.send(Request::FactoryReset);
            }
        });
    }
}
