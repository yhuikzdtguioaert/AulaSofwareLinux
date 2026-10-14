//! Controls drawn from the vendor's own bitmaps.
//!
//! The original window skins every control, and the bitmaps carry their exact
//! metrics: the slider track is `bar_nr.png` at 450x3 with a 12x12 handle,
//! buttons are `button_nr.png` at 120x26, list rows use the `clrListBox*`
//! colours from `Cfg.ini`. Matching those keeps the layout identical rather
//! than merely similar.

use egui::{Color32, Rect, Response, Sense, Ui, Vec2};

use super::theme::Theme;
use crate::assets::Skins;

/// Native size of `bar_nr.png` / `bar_ov.png`.
pub const TRACK_H: f32 = 3.0;
/// Native size of `slider.png`.
pub const HANDLE: f32 = 12.0;
/// Native size of `button_nr.png`.
pub const BUTTON: Vec2 = Vec2::new(120.0, 26.0);
/// Native size of `combo_nr.png`, which the original also uses for list boxes.
pub const LIST_W: f32 = 236.0;
/// Row height inside a list box.
pub const ROW_H: f32 = 24.0;

fn image(ui: &Ui, tex: &egui::TextureHandle, rect: Rect) {
    ui.painter().image(
        tex.id(),
        rect,
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

/// A skinned horizontal slider over `0..=max`. Returns whether the value
/// changed this frame, plus the response so the caller can tell when the
/// gesture finished and it is time to talk to the keyboard.
///
/// `width` lets a caller use less than the bitmap's native 450 px; the track is
/// a 3 px strip so stretching it horizontally is lossless.
pub fn slider(
    ui: &mut Ui,
    skins: &mut Skins,
    theme: &Theme,
    width: f32,
    value: &mut usize,
    max: usize,
    enabled: bool,
) -> (bool, Response) {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, HANDLE + 6.0),
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );

    let track = Rect::from_min_size(
        egui::pos2(rect.min.x, rect.center().y - TRACK_H / 2.0),
        Vec2::new(width, TRACK_H),
    );
    let span = (width - HANDLE).max(1.0);
    let frac = if max == 0 {
        0.0
    } else {
        *value as f32 / max as f32
    };

    match skins.get(ui.ctx(), "bar_nr.png") {
        Some(t) => image(ui, &t, track),
        None => {
            ui.painter().rect_filled(track, 1.5, theme.edit_alt);
        }
    }
    let filled = Rect::from_min_size(
        track.min,
        Vec2::new((HANDLE / 2.0 + span * frac).min(width), TRACK_H),
    );
    match skins.get(ui.ctx(), "bar_ov.png") {
        Some(t) => image(ui, &t, filled),
        None => {
            ui.painter()
                .rect_filled(filled, 1.5, theme.button_text_active);
        }
    }

    let handle = Rect::from_min_size(
        egui::pos2(rect.min.x + span * frac, rect.center().y - HANDLE / 2.0),
        Vec2::splat(HANDLE),
    );
    match skins.get(ui.ctx(), "slider.png") {
        Some(t) => image(ui, &t, handle),
        None => {
            ui.painter()
                .circle_filled(handle.center(), HANDLE / 2.0, theme.button_text_hover);
        }
    }

    if !enabled {
        // Grey the whole control out the way the original does.
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_black_alpha(120));
        return (false, response);
    }

    let mut changed = false;
    if response.dragged() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            let t = ((p.x - rect.min.x - HANDLE / 2.0) / span).clamp(0.0, 1.0);
            let next = (t * max as f32).round() as usize;
            if next != *value {
                *value = next;
                changed = true;
            }
        }
    }
    (changed, response)
}

/// True once the user finished a slider gesture, i.e. a good moment to send the
/// value to the keyboard rather than on every pixel of a drag.
pub fn settled(response: &Response) -> bool {
    response.drag_stopped() || response.clicked()
}

/// Draw a plain caption at a fixed position, as the original does for the
/// labels beside its controls.
pub fn caption(ui: &Ui, theme: &Theme, pos: egui::Pos2, text: &str) {
    ui.painter().text(
        pos,
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(13.0),
        theme.text,
    );
}

/// Run `add` inside a child Ui pinned to `rect`, which is how the fixed-layout
/// pages place their controls.
pub fn at<R>(ui: &mut Ui, rect: Rect, layout: egui::Layout, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), add)
        .inner
}

/// A skinned push button.
pub fn button(
    ui: &mut Ui,
    skins: &mut Skins,
    theme: &Theme,
    text: &str,
    enabled: bool,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        BUTTON,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    match skins.get(ui.ctx(), "button_nr.png") {
        Some(t) => image(ui, &t, rect),
        None => {
            ui.painter().rect_filled(rect, 3.0, theme.edit_alt);
        }
    }
    if hovered {
        ui.painter()
            .rect_filled(rect, 3.0, Color32::from_white_alpha(18));
    }
    let color = if !enabled {
        theme.text.gamma_multiply(0.45)
    } else if hovered {
        theme.button_text_hover
    } else {
        theme.button_text
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        color,
    );
    response
}

/// One row of a list box. Returns the row's response so the caller can act on
/// a click.
pub fn list_row(ui: &mut Ui, theme: &Theme, width: f32, text: &str, selected: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, theme.list_item);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, theme.menu_hover.gamma_multiply(0.5));
    }
    let color = if selected || response.hovered() {
        theme.button_text_hover
    } else {
        theme.button_text
    };
    ui.painter().text(
        egui::pos2(rect.min.x + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(13.0),
        color,
    );
    response
}

/// Background and border for a list box, drawn before its rows.
pub fn list_frame(ui: &Ui, theme: &Theme, rect: Rect) {
    ui.painter().rect_filled(rect, 2.0, theme.list_bg);
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme.line),
        egui::StrokeKind::Inside,
    );
}

/// A colour swatch. Returns true when clicked.
pub fn swatch(ui: &mut Ui, theme: &Theme, rgb: [u8; 3], selected: bool, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    ui.painter()
        .rect_filled(rect, 2.0, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
    let stroke = if selected {
        egui::Stroke::new(2.0, theme.button_text_hover)
    } else if response.hovered() {
        egui::Stroke::new(1.0, theme.button_text_hover)
    } else {
        egui::Stroke::new(1.0, theme.line)
    };
    ui.painter()
        .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Outside);
    response
}

/// The vendor colour wheel. Returns the colour under the pointer when clicked.
///
/// `colorwheel.png` is a 116x117 HSV disc; the colour is computed from the
/// pointer position rather than sampled, which keeps it exact at any scale.
pub fn color_wheel(
    ui: &mut Ui,
    skins: &mut Skins,
    theme: &Theme,
    size: f32,
) -> (Response, Option<[u8; 3]>) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
    match skins.get(ui.ctx(), "colorwheel.png") {
        Some(t) => image(ui, &t, rect),
        None => {
            ui.painter()
                .circle_filled(rect.center(), size / 2.0, theme.edit_alt);
        }
    }

    let mut picked = None;
    if response.dragged() || response.clicked() {
        if let Some(p) = response.interact_pointer_pos() {
            let c = rect.center();
            let r = size / 2.0;
            let dx = (p.x - c.x) / r;
            let dy = (p.y - c.y) / r;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= 1.0 {
                let hue = dy.atan2(dx).to_degrees().rem_euclid(360.0);
                picked = Some(hsv_to_rgb(hue, dist.min(1.0), 1.0));
            }
        }
    }
    (response, picked)
}

/// `h` in degrees, `s` and `v` in 0..=1.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [
        (((r + m) * 255.0).round()).clamp(0.0, 255.0) as u8,
        (((g + m) * 255.0).round()).clamp(0.0, 255.0) as u8,
        (((b + m) * 255.0).round()).clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_hits_the_primaries() {
        assert_eq!(hsv_to_rgb(0.0, 1.0, 1.0), [255, 0, 0]);
        assert_eq!(hsv_to_rgb(120.0, 1.0, 1.0), [0, 255, 0]);
        assert_eq!(hsv_to_rgb(240.0, 1.0, 1.0), [0, 0, 255]);
    }

    #[test]
    fn hsv_desaturates_to_white() {
        assert_eq!(hsv_to_rgb(200.0, 0.0, 1.0), [255, 255, 255]);
    }

    #[test]
    fn hsv_wraps_the_hue() {
        assert_eq!(hsv_to_rgb(360.0, 1.0, 1.0), hsv_to_rgb(0.0, 1.0, 1.0));
        assert_eq!(hsv_to_rgb(-120.0, 1.0, 1.0), hsv_to_rgb(240.0, 1.0, 1.0));
    }

    #[test]
    fn control_metrics_match_the_vendor_bitmaps() {
        assert_eq!(TRACK_H, 3.0, "bar_nr.png is 450x3");
        assert_eq!(HANDLE, 12.0, "slider.png is 12x12");
        assert_eq!(BUTTON, Vec2::new(120.0, 26.0), "button_nr.png is 120x26");
        assert_eq!(LIST_W, 236.0, "combo_nr.png is 236x32");
    }
}
