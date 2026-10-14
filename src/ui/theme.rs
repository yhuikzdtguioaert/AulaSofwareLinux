//! Colours and metrics taken from the vendor `Cfg.ini`, so the window matches
//! the original pixel for pixel.

use egui::Color32;

use crate::ini::Ini;
use std::path::Path;

/// Design size of the original window (`skins/main_nr.png` is exactly this).
pub const DESIGN_W: f32 = 1200.0;
pub const DESIGN_H: f32 = 720.0;

/// Title bar height, measured from `main_nr.png`.
pub const TITLE_H: f32 = 66.0;
/// Left edge of the content area, where the vendor background draws its rule.
pub const CONTENT_X: f32 = 286.0;
/// Left rail / profile column.
pub const RAIL_X: f32 = 66.0;
/// Navigation buttons are 50x50 and start at `ptNavi` from Cfg.ini.
pub const NAVI_SIZE: f32 = 50.0;
pub const NAVI_STEP: f32 = 60.0;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub text: Color32,
    pub button_text: Color32,
    pub button_text_hover: Color32,
    pub button_text_active: Color32,
    pub list_bg: Color32,
    pub list_item: Color32,
    pub menu_bg: Color32,
    pub menu_hover: Color32,
    pub menu_frame: Color32,
    pub line: Color32,
    pub edit: Color32,
    pub edit_alt: Color32,
    /// Position of the first navigation button (`ptNavi=`).
    pub navi: egui::Pos2,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            text: rgb(180, 180, 180),
            button_text: rgb(180, 180, 180),
            button_text_hover: rgb(240, 240, 240),
            button_text_active: rgb(10, 147, 255),
            list_bg: rgb(33, 33, 33),
            list_item: rgb(0, 93, 145),
            menu_bg: rgb(37, 37, 37),
            menu_hover: rgb(0, 93, 145),
            menu_frame: rgb(71, 71, 71),
            line: rgb(49, 49, 49),
            edit: rgb(84, 84, 84),
            edit_alt: rgb(50, 50, 50),
            navi: egui::pos2(120.0, 12.0),
        }
    }
}

impl Theme {
    /// Read the palette out of a vendor `Cfg.ini`, keeping the defaults for
    /// anything the file leaves out.
    pub fn from_cfg(path: &Path) -> Theme {
        let mut t = Theme::default();
        let Ok(ini) = Ini::load(path) else { return t };
        let set = |slot: &mut Color32, key: &str| {
            if let Some(c) = ini.rgb("OPT", key) {
                *slot = rgb(c[0], c[1], c[2]);
            }
        };
        set(&mut t.text, "clrTxtColor");
        set(&mut t.button_text, "clrBtnTxtNr");
        set(&mut t.button_text_hover, "clrBtnTxtOv");
        set(&mut t.button_text_active, "clrBtnTxtDn");
        set(&mut t.list_bg, "clrListBoxBkg");
        set(&mut t.list_item, "clrListBoxBkgItem");
        set(&mut t.menu_bg, "clrMenuNr");
        set(&mut t.menu_hover, "clrMenuOv");
        set(&mut t.menu_frame, "clrMenuFrame");
        set(&mut t.line, "clrLine");
        set(&mut t.edit, "clrEdit");
        set(&mut t.edit_alt, "clrEdit2");
        let p = ini.nums("OPT", "ptNavi");
        if p.len() >= 2 {
            t.navi = egui::pos2(p[0] as f32, p[1] as f32);
        }
        t
    }

    /// Apply the palette to egui's own widgets so sliders and combo boxes look
    /// like the rest of the window.
    ///
    /// Both the light and the dark style are overwritten, so the window looks
    /// the same whatever the desktop is set to — the original has one skin.
    pub fn install(&self, ctx: &egui::Context) {
        ctx.all_styles_mut(|style| self.apply_to(style));
    }

    fn apply_to(&self, style: &mut egui::Style) {
        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = Some(self.text);
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = self.menu_bg;
        v.window_stroke = egui::Stroke::new(1.0, self.menu_frame);
        v.extreme_bg_color = self.list_bg;
        v.faint_bg_color = self.edit_alt;
        v.selection.bg_fill = self.list_item;
        v.selection.stroke = egui::Stroke::new(1.0, self.button_text_hover);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.menu_bg;
        w.noninteractive.bg_stroke = egui::Stroke::new(1.0, self.line);
        w.noninteractive.fg_stroke = egui::Stroke::new(1.0, self.text);
        w.inactive.bg_fill = self.edit_alt;
        w.inactive.weak_bg_fill = self.edit_alt;
        w.inactive.fg_stroke = egui::Stroke::new(1.0, self.button_text);
        w.hovered.bg_fill = self.menu_hover;
        w.hovered.weak_bg_fill = self.menu_hover;
        w.hovered.fg_stroke = egui::Stroke::new(1.0, self.button_text_hover);
        w.active.bg_fill = self.list_item;
        w.active.weak_bg_fill = self.list_item;
        w.active.fg_stroke = egui::Stroke::new(1.0, self.button_text_active);
        w.open.bg_fill = self.menu_bg;

        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.slider_width = 220.0;
    }
}

pub const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn from_slice(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_shipped_cfg_ini() {
        let t = Theme::default();
        assert_eq!(t.text, rgb(180, 180, 180));
        assert_eq!(t.button_text_active, rgb(10, 147, 255));
        assert_eq!(t.navi, egui::pos2(120.0, 12.0));
    }
}
