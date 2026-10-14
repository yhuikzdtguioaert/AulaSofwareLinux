//! The keyboard picture with its clickable keys.
//!
//! `keyimg.png` and the pixel rectangle of every key both come straight from
//! the vendor profile, so the layout is the original one rather than a
//! reconstruction. Key rectangles are given in image pixels and scaled with the
//! picture.

use std::collections::BTreeSet;

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::profile::DeviceProfile;

pub struct KeyboardView {
    pub texture: Option<egui::TextureHandle>,
    pub image_size: Vec2,
}

/// What a click did, so the caller can react without re-hit-testing.
pub struct KeyboardResponse {
    /// Key index under the pointer, if any.
    pub hovered: Option<u32>,
    /// Key index that was clicked this frame.
    pub clicked: Option<u32>,
    /// True while the user is dragging a rubber-band selection.
    pub dragging: bool,
    pub rect: Rect,
}

impl KeyboardView {
    pub fn load(ctx: &egui::Context, profile: &DeviceProfile) -> KeyboardView {
        let Some(path) = &profile.keyimg else {
            return KeyboardView {
                texture: None,
                image_size: Vec2::new(679.0, 304.0),
            };
        };
        match crate::assets::load_png(path) {
            Ok((img, size)) => {
                let image_size = Vec2::new(size[0] as f32, size[1] as f32);
                let texture = ctx.load_texture(
                    format!("keyimg-{}", profile.id),
                    img,
                    egui::TextureOptions::LINEAR,
                );
                KeyboardView {
                    texture: Some(texture),
                    image_size,
                }
            }
            Err(_) => KeyboardView {
                texture: None,
                image_size: Vec2::new(679.0, 304.0),
            },
        }
    }

    /// Largest rectangle inside `area` with the picture's aspect ratio.
    pub fn fit(&self, area: Rect) -> Rect {
        let scale = (area.width() / self.image_size.x)
            .min(area.height() / self.image_size.y)
            .max(0.01);
        let size = self.image_size * scale;
        Rect::from_center_size(area.center(), size)
    }

    /// Draw the keyboard and hit-test it.
    ///
    /// `key_color` supplies the per-key tint (the current LED colours);
    /// `selected` is highlighted with the profile's `clrKeyDn`.
    pub fn show(
        &self,
        ui: &mut egui::Ui,
        area: Rect,
        profile: &DeviceProfile,
        selected: &BTreeSet<u32>,
        key_color: impl Fn(&crate::profile::KeyDef) -> Option<Color32>,
    ) -> KeyboardResponse {
        let rect = self.fit(area);
        let scale = rect.width() / self.image_size.x;
        let painter = ui.painter_at(area);

        if let Some(tex) = &self.texture {
            painter.image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            painter.rect_filled(rect, 6.0, Color32::from_rgb(24, 24, 24));
        }

        let response = ui.interact(rect, ui.id().with("keyboard"), Sense::click_and_drag());
        let pointer = response.hover_pos();

        let hover_col = super::theme::from_slice(profile.color_key_hover);
        let down_col = super::theme::from_slice(profile.color_key_down);

        let mut hovered = None;
        let mut clicked = None;

        for key in profile.keys.iter().filter(|k| k.is_drawn()) {
            let kr = Rect::from_min_max(
                rect.min + Vec2::new(key.rect[0] as f32 * scale, key.rect[1] as f32 * scale),
                rect.min + Vec2::new(key.rect[2] as f32 * scale, key.rect[3] as f32 * scale),
            );

            if let Some(c) = key_color(key) {
                // Tint rather than fill, so the key cap art stays visible.
                painter.rect_filled(kr, 2.0, c.gamma_multiply(0.55));
            }

            let is_hovered = pointer.is_some_and(|p| kr.contains(p));
            if is_hovered {
                hovered = Some(key.index);
                if response.clicked() {
                    clicked = Some(key.index);
                }
            }

            let stroke = if selected.contains(&key.index) {
                Stroke::new(2.0, down_col)
            } else if is_hovered {
                Stroke::new(2.0, hover_col)
            } else {
                Stroke::NONE
            };
            if stroke != Stroke::NONE {
                painter.rect_stroke(kr, 2.0, stroke, egui::StrokeKind::Inside);
            }
        }

        KeyboardResponse {
            hovered,
            clicked,
            dragging: response.dragged(),
            rect,
        }
    }

    /// Keys whose rectangle intersects `band`, in image space.
    pub fn keys_in_band(&self, profile: &DeviceProfile, image_rect: Rect, band: Rect) -> Vec<u32> {
        let scale = image_rect.width() / self.image_size.x;
        profile
            .keys
            .iter()
            .filter(|k| k.is_drawn())
            .filter(|k| {
                let kr = Rect::from_min_max(
                    image_rect.min + Vec2::new(k.rect[0] as f32 * scale, k.rect[1] as f32 * scale),
                    image_rect.min + Vec2::new(k.rect[2] as f32 * scale, k.rect[3] as f32 * scale),
                );
                kr.intersects(band)
            })
            .map(|k| k.index)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> KeyboardView {
        KeyboardView {
            texture: None,
            image_size: Vec2::new(679.0, 304.0),
        }
    }

    #[test]
    fn fit_preserves_the_aspect_ratio() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let r = view().fit(area);
        let ratio = r.width() / r.height();
        assert!((ratio - 679.0 / 304.0).abs() < 0.001);
        assert!(r.width() <= area.width() + 0.01);
        assert!(r.height() <= area.height() + 0.01);
    }

    #[test]
    fn fit_is_centred_in_the_area() {
        let area = Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(800.0, 600.0));
        assert_eq!(view().fit(area).center(), area.center());
    }

    #[test]
    fn fit_survives_a_degenerate_area() {
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::ZERO);
        let r = view().fit(area);
        assert!(r.width().is_finite() && r.height().is_finite());
    }
}
