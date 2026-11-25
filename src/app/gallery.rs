//! Окно галереи пресетов: карточки с миниатюрами, настройки генератора
//! стабильных систем и отрисовка превью.

use std::collections::HashMap;

use eframe::egui;

use crate::physics::{Body, PresetCategory, Scenario, StableSystemSpec, Stability, Vec3};

use super::GravityApp;

impl GravityApp {
    /// Случайный сид для генератора (без внешнего crate rand).
    fn random_seed() -> u64 {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678);
        let mut x = n ^ 0x9E37_79B9_7F4A_7C15;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }

    /// Окно галереи пресетов (плавающее, с описаниями и устойчивостью).
    pub(super) fn show_gallery_window(&mut self, ctx: &egui::Context) {
        if !self.show_gallery {
            return;
        }
        let mut open = true;
        let mut load: Option<Scenario> = None;
        let mut generated: Option<Vec<Body>> = None;
        egui::Window::new("Галерея пресетов")
            .open(&mut open)
            .default_width(620.0)
            .default_height(540.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Готовые конфигурации: от точных хореографий до иерархических \
                         систем. Устойчивость указана по литературе и критериям Хилла.",
                    )
                    .small()
                    .weak(),
                );
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for category in PresetCategory::ALL {
                        ui.add_space(8.0);
                        ui.heading(category.name());
                        ui.label(egui::RichText::new(category.hint()).small().weak());
                        for scenario in Scenario::in_category(category) {
                            let info = scenario.info();
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    Self::draw_thumbnail(ui, &mut self.thumbnails, scenario);
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.strong(info.name);
                                            let color = match info.stability {
                                                Stability::Stable => {
                                                    egui::Color32::from_rgb(120, 235, 160)
                                                }
                                                Stability::LongLived => {
                                                    egui::Color32::from_rgb(150, 210, 255)
                                                }
                                                Stability::WeaklyUnstable => {
                                                    egui::Color32::from_rgb(255, 200, 110)
                                                }
                                                Stability::Demo => {
                                                    egui::Color32::from_rgb(210, 170, 255)
                                                }
                                            };
                                            ui.label(
                                                egui::RichText::new(info.stability.label())
                                                    .small()
                                                    .color(color),
                                            )
                                            .on_hover_text(info.stability.hint());
                                            if ui.button("Загрузить").clicked() {
                                                load = Some(scenario);
                                            }
                                        });
                                        ui.label(egui::RichText::new(info.description).small());
                                        if let Some(rec) = info.recommended {
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "Рекомендуется: {} · dt {:.3} · ε² {:.1} · ×{:.1}",
                                                    rec.integrator.name(),
                                                    rec.dt,
                                                    rec.softening,
                                                    rec.speed
                                                ))
                                                .small()
                                                .weak(),
                                            );
                                        }
                                    });
                                });
                            });
                        }
                    }

                    // ── Генератор стабильных систем ─────────────
                    ui.add_space(12.0);
                    ui.separator();
                    ui.heading("Генератор систем");
                    ui.label(
                        egui::RichText::new(
                            "Планеты разносятся на k взаимных радиусов Хилла \
                             (критерий Гладмана 2√3 ≈ 3.46) — такие системы \
                             устойчивы на длинных горизонтах.",
                        )
                        .small()
                        .weak(),
                    );
                    ui.add_space(4.0);
                    if ui.button("🎲 Случайная стабильная система").clicked() {
                        let seed = Self::random_seed();
                        self.gen_spec = Self::random_spec(seed);
                        generated =
                            Some(crate::physics::presets::generate_stable(
                                &self.gen_spec,
                                self.params.g,
                            ));
                    }
                    ui.add_space(4.0);
                    egui::Grid::new("gen-grid")
                        .num_columns(2)
                        .spacing([10.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Планет:");
                            ui
                                .add(
                                    egui::DragValue::new(&mut self.gen_spec.planets)
                                        .range(1..=20)
                                        .suffix(" шт"),
                                );
                            ui.end_row();

                            ui.label("Масса звезды:");
                            ui
                                .add(
                                    egui::Slider::new(
                                        &mut self.gen_spec.star_mass,
                                        100.0..=5000.0,
                                    )
                                    .logarithmic(true),
                                );
                            ui.end_row();

                            ui.label("Массы планет:");
                            ui.horizontal(|ui| {
                                ui
                                    .add(
                                        egui::DragValue::new(&mut self.gen_spec.mass_lo)
                                            .speed(0.05)
                                            .range(0.01..=self.gen_spec.mass_hi),
                                    );
                                ui.label("…");
                                ui
                                    .add(
                                        egui::DragValue::new(&mut self.gen_spec.mass_hi)
                                            .speed(0.05)
                                            .range(self.gen_spec.mass_lo..=100.0),
                                    );
                            });
                            ui.end_row();

                            ui.label("Теснота k:");
                            ui
                                .add(
                                    egui::Slider::new(&mut self.gen_spec.k, 3.6..=8.0)
                                        .text("×R_H"),
                                )
                                .on_hover_text("Меньше — плотнее и рискованнее, больше — просторнее и надёжнее");
                            ui.end_row();

                            ui.label("Первая орбита:");
                            ui
                                .add(
                                    egui::Slider::new(&mut self.gen_spec.start_a, 30.0..=200.0),
                                );
                            ui.end_row();

                            ui.label("Эксцентриситет ≤");
                            ui
                                .add(
                                    egui::Slider::new(&mut self.gen_spec.ecc_max, 0.0..=0.1),
                                );
                            ui.end_row();

                            ui.label("Наклонения ≤");
                            ui
                                .add(
                                    egui::Slider::new(
                                        &mut self.gen_spec.incl_max,
                                        0.0..=0.5,
                                    )
                                    .text("рад"),
                                );
                            ui.end_row();

                            ui.label("Наклон сцены:");
                            ui
                                .add(
                                    egui::Slider::new(&mut self.gen_spec.tilt, 0.0..=1.2)
                                        .text("рад"),
                                );
                            ui.end_row();

                            ui.label("Сид:");
                            ui.horizontal(|ui| {
                                ui
                                    .add(
                                        egui::DragValue::new(&mut self.gen_spec.seed)
                                            .speed(1),
                                    );
                                if ui.button("🔁").on_hover_text("Новый сид").clicked() {
                                    self.gen_spec.seed = Self::random_seed();
                                    generated =
                                        Some(crate::physics::presets::generate_stable(
                                            &self.gen_spec,
                                            self.params.g,
                                        ));
                                }
                            });
                            ui.end_row();
                        });
                    ui.add_space(6.0);
                    if ui
                        .add(
                            egui::Button::new("⚙ Сгенерировать и загрузить").min_size(
                                egui::vec2(240.0, 32.0),
                            ),
                        )
                        .clicked()
                    {
                        generated = Some(crate::physics::presets::generate_stable(
                            &self.gen_spec,
                            self.params.g,
                        ));
}
                    ui.add_space(4.0);
                });
            });
        if !open {
            self.show_gallery = false;
        }
        if let Some(scenario) = load {
            self.load_preset(scenario);
            self.show_gallery = false;
        }
        if let Some(bodies) = generated {
            self.load_custom_bodies(bodies);
            self.show_gallery = false;
        }
    }

    /// Случайная, но заведомо устойчивая спецификация: k ≥ 4.6 (запас
    /// относительно критерия Гладмана), умеренные эксцентриситеты.
    fn random_spec(seed: u64) -> StableSystemSpec {
        // Вспомогательная функция вместо замыкания: Rng и так мутабельный.
        fn plan(rng: &mut crate::physics::vec::Rng, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * rng.f64()
        }
        let mut rng = crate::physics::vec::Rng::new(seed | 1);
        let star = plan(&mut rng, 800.0, 2600.0);
        let planets = 4 + (rng.f64() * 7.0) as usize;
        StableSystemSpec {
            star_mass: star,
            planets,
            mass_lo: plan(&mut rng, 0.05, 0.3),
            mass_hi: plan(&mut rng, 0.5, 3.0),
            k: plan(&mut rng, 4.6, 6.5),
            start_a: plan(&mut rng, 60.0, 140.0),
            ecc_max: plan(&mut rng, 0.0, 0.03),
            incl_max: plan(&mut rng, 0.0, 0.08),
            tilt: plan(&mut rng, 0.0, 0.9),
            seed,
        }
    }

    /// Миниатюра пресета: вид сверху на сгенерированные тела, вписанные
    /// в прямоугольник. Тела генерируются лениво и кэшируются (сид
    /// фиксированный — картинка детерминированная).
    fn draw_thumbnail(
        ui: &mut egui::Ui,
        cache: &mut HashMap<Scenario, Vec<(Vec3, f64, [u8; 3])>>,
        scenario: Scenario,
    ) {
        const SIZE: egui::Vec2 = egui::Vec2::new(96.0, 64.0);
        const MAX_BODIES: usize = 400;
        let (rect, resp) = ui.allocate_exact_size(SIZE, egui::Sense::hover());
        let bodies = cache.entry(scenario).or_insert_with(|| {
            crate::physics::presets::generate(scenario, 1.0)
                .into_iter()
                .map(|b| (b.pos, b.radius, b.color))
                .collect()
        });

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, egui::Color32::from_rgb(9, 11, 22));

        let shown = &bodies[..bodies.len().min(MAX_BODIES)];
        let count_label = if shown.is_empty() {
            Some("пусто".to_string())
        } else {
            let mut min = (f64::INFINITY, f64::INFINITY);
            let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for (pos, _, _) in shown {
                min.0 = min.0.min(pos.x);
                min.1 = min.1.min(pos.y);
                max.0 = max.0.max(pos.x);
                max.1 = max.1.max(pos.y);
            }
            let span = (
                (max.0 - min.0).max(1.0),
                (max.1 - min.1).max(1.0),
            );
            let margin = 8.0;
            let scale = (((SIZE.x - margin * 2.0) as f64) / span.0)
                .min(((SIZE.y - margin * 2.0) as f64) / span.1)
                .max(0.01) as f32;
            let center = ((min.0 + max.0) * 0.5, (min.1 + max.1) * 0.5);
            let c = rect.center();
            for (pos, radius, color) in shown {
                let px = c.x + (((pos.x - center.0) * scale as f64) as f32);
                let py = c.y - (((pos.y - center.1) * scale as f64) as f32);
                let r = ((radius * scale as f64) as f32).clamp(1.2, 7.0);
                painter.circle_filled(
                    egui::pos2(px, py),
                    r,
                    egui::Color32::from_rgb(color[0], color[1], color[2]),
                );
            }
            Some(format!("{} тел", bodies.len()))
        };
        if let Some(label) = count_label {
            resp.on_hover_text(label);
        }
    }

}
