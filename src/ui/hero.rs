use crate::hero::{Lighting, EFFECTS, KEY_COUNT, KEY_ROWS};
use crate::worker::{Request, Worker};

pub struct Panel {
    effect: usize,
    color: [u8; 3],
    custom: bool,
    keys: [[u8; 3]; KEY_COUNT],
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            effect: 0,
            color: [255, 0, 0],
            custom: false,
            keys: [[0; 3]; KEY_COUNT],
        }
    }
}

impl Panel {
    pub fn show(&mut self, ui: &mut egui::Ui, worker: &Worker) {
        ui.heading("Lighting");
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.custom, false, "Effects");
            ui.selectable_value(&mut self.custom, true, "Per-key");
        });
        ui.add_space(12.0);
        if self.custom {
            self.show_keys(ui);
        } else {
            ui.horizontal(|ui| {
                ui.label("Effect");
                egui::ComboBox::from_id_salt("hero-effect")
                    .selected_text(EFFECTS[self.effect])
                    .show_ui(ui, |ui| {
                        for (index, &name) in EFFECTS.iter().enumerate() {
                            ui.selectable_value(&mut self.effect, index, name);
                        }
                    });
                ui.label("Color");
                ui.color_edit_button_srgb(&mut self.color);
            });
        }

        ui.add_space(12.0);
        if ui.button("Apply lighting").clicked() {
            let setting = if self.custom {
                Lighting::PerKey(Box::new(self.keys))
            } else {
                Lighting::Effect {
                    id: self.effect as u8 + 1,
                    color: self.color,
                }
            };
            worker.send(Request::SetHeroLighting(setting));
        }
    }

    fn show_keys(&mut self, ui: &mut egui::Ui) {
        ui.label("Choose a color, then click keys to paint. Apply sends the full layout.");
        ui.horizontal(|ui| {
            ui.color_edit_button_srgb(&mut self.color);
            if ui.button("Fill all").clicked() {
                self.keys.fill(self.color);
            }
            if ui.button("Clear").clicked() {
                self.keys.fill([0; 3]);
            }
        });
        ui.add_space(8.0);

        let mut colors = self.keys.iter_mut();
        for row in KEY_ROWS {
            ui.horizontal(|ui| {
                for &(_, label) in *row {
                    let color = colors.next().expect("one color per key");
                    let fill = egui::Color32::from_rgb(color[0], color[1], color[2]);
                    let [r, g, b] = *color;
                    let luminance = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b);
                    let text = if luminance < 128_000 {
                        egui::Color32::WHITE
                    } else {
                        egui::Color32::BLACK
                    };
                    let button = egui::Button::new(egui::RichText::new(label).color(text))
                        .fill(fill)
                        .min_size(egui::vec2(32.0, 30.0));
                    if ui.add(button).clicked() {
                        *color = self.color;
                    }
                }
            });
        }
    }
}
