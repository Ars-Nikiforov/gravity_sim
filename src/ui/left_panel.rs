//! Левая панель — инструментарий: выбор инструмента, параметры
//! создаваемого тела и управление ходом времени.

use eframe::egui;

use crate::app::{GravityApp, Tool};

pub fn show(ui: &mut egui::Ui, app: &mut GravityApp) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(8.0);
        ui.heading("Инструменты");
        ui.separator();

        // ── Инструмент ──────────────────────────────────────────────
        ui.group(|ui| {
            ui.label("Активный инструмент:");
            for tool in Tool::ALL {
                ui.radio_value(&mut app.tool, tool, tool.name());
            }
            match app.tool {
                Tool::Select => ui.label(
                    egui::RichText::new("Клик — выбор тела, перетаскивание — панорама")
                        .small()
                        .weak(),
                ),
                Tool::AddBody => ui.label(
                    egui::RichText::new("Клик по холсту — новое тело на круговой орбите")
                        .small()
                        .weak(),
                ),
                Tool::DeleteBody => {
                    ui.label(egui::RichText::new("Клик по телу — удалить").small().weak())
                }
            }
        });

        // ── Параметры нового тела ───────────────────────────────────
        if app.tool == Tool::AddBody {
            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label("Новое тело:");
                ui.add(
                    egui::Slider::new(&mut app.new_body.mass, 0.1..=2000.0)
                        .logarithmic(true)
                        .text("Масса"),
                );
                ui.add(
                    egui::Slider::new(&mut app.new_body.radius, 0.5..=20.0)
                        .text("Радиус"),
                );
                ui.horizontal(|ui| {
                    ui.label("Цвет:");
                    ui.color_edit_button_srgb(&mut app.new_body.color);
                });
            });
        }

        // ── Управление временем ─────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Ход времени:");
            let speed_changed = ui
                .add(
                    egui::Slider::new(&mut app.params.speed, 0.0..=10.0)
                        .text("Скорость, ×"),
                )
                .changed();
            if speed_changed {
                app.send_params();
            }

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let running = app.physics.is_running();
                let run_label = if running { "Пауза" } else { "Пуск" };
                if ui.button(run_label).clicked() {
                    app.toggle_run();
                }
                if ui.button("Шаг").clicked() {
                    app.physics.send(crate::physics::Command::StepOnce);
                }
                if ui.button("Перезапуск").clicked() {
                    app.reset(app.scenario);
                }
            });
            let running = app.physics.is_running();
            ui.label(
                egui::RichText::new(if running {
                    "Статус: идёт расчёт"
                } else {
                    "Статус: пауза"
                })
                .small()
                .weak(),
            );
        });

        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Пробел — пауза/пуск. Колесо мыши — зум.")
                .small()
                .weak(),
        );
    });
}
