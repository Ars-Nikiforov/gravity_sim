//! Правая панель — инспектор: статистика симуляции, настройки физики,
//! список тел и свойства выбранного тела.

use eframe::egui;

use crate::app::GravityApp;
use crate::physics::{Algorithm, Command};

pub fn show(ui: &mut egui::Ui, app: &mut GravityApp) {
    let snapshot = app.physics.snapshot();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(8.0);
        ui.heading("Инспектор");
        ui.separator();

        // ── Статистика ──────────────────────────────────────────────
        ui.group(|ui| {
            ui.label("Статистика:");
            egui::Grid::new("stats").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
                ui.label("FPS интерфейса:");
                ui.label(format!("{:5.1}", app.fps));
                ui.end_row();

                ui.label("Шагов физики/с:");
                let steps_per_sec = if app.params.dt > 0.0 {
                    app.params.speed / app.params.dt
                } else {
                    0.0
                };
                ui.label(format!("{:5.0}", steps_per_sec));
                ui.end_row();

                ui.label("Шаг физики, мс:");
                ui.label(format!("{:5.2}", snapshot.step_ms));
                ui.end_row();

                ui.label("Тел:");
                ui.label(snapshot.bodies.len().to_string());
                ui.end_row();

                ui.label("Тик:");
                ui.label(snapshot.tick.to_string());
                ui.end_row();

                ui.label("Время симуляции:");
                ui.label(format!("{:8.1}", snapshot.sim_time));
                ui.end_row();

                ui.label("Алгоритм:");
                ui.label(app.params.algorithm.name());
                ui.end_row();

                ui.label("Состояние:");
                ui.label(if snapshot.running {
                    "расчёт"
                } else {
                    "пауза"
                });
                ui.end_row();
            });
        });

        // ── Параметры физики ────────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Физика:");
            let mut changed = false;
            for algorithm in Algorithm::ALL {
                changed |= ui
                    .radio_value(&mut app.params.algorithm, algorithm, algorithm.name())
                    .changed();
            }
            ui.add_space(4.0);
            changed |= ui
                .add(egui::Slider::new(&mut app.params.g, 0.0..=50.0).text("Гравитация G"))
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut app.params.dt, 0.001..=0.05)
                        .text("Шаг dt"),
                )
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut app.params.softening, 0.0..=100.0)
                        .text("Сглаживание ε²"),
                )
                .changed();
            if changed {
                app.send_params();
            }
        });

        // ── Список тел ──────────────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label(format!("Тела ({}):", snapshot.bodies.len()));
            egui::ScrollArea::vertical()
                .max_height(220.0)
                .show(ui, |ui| {
                    for body in &snapshot.bodies {
                        let selected = app.selected_id == Some(body.id);
                        let label = if body.mass >= 100.0 {
                            format!("★ #{}  m = {:.0}", body.id, body.mass)
                        } else {
                            format!("  #{}  m = {:.1}", body.id, body.mass)
                        };
                        let row = egui::RichText::new(label).color(egui::Color32::from_rgb(
                            body.color[0],
                            body.color[1],
                            body.color[2],
                        ));
                        if ui.selectable_label(selected, row).clicked() {
                            app.selected_id = Some(body.id);
                        }
                    }
                });
        });

        // ── Выбранное тело ──────────────────────────────────────────
        ui.add_space(6.0);
        let selected = snapshot
            .bodies
            .iter()
            .find(|b| Some(b.id) == app.selected_id)
            .cloned();
        ui.group(|ui| {
            ui.label("Свойства тела:");
            match selected {
                Some(body) => {
                    let mut mass = body.mass;
                    if ui
                        .add(
                            egui::Slider::new(&mut mass, 0.1..=3000.0)
                                .logarithmic(true)
                                .text("Масса"),
                        )
                        .changed()
                    {
                        app.physics.send(Command::SetBodyMass(body.id, mass));
                    }
                    egui::Grid::new("body_props")
                        .num_columns(2)
                        .spacing([20.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Позиция:");
                            ui.label(format!("({:7.1}, {:7.1})", body.pos.x, body.pos.y));
                            ui.end_row();
                            ui.label("Скорость:");
                            ui.label(format!(
                                "({:6.2}, {:6.2})  |v| = {:5.2}",
                                body.vel.x,
                                body.vel.y,
                                body.vel.len()
                            ));
                            ui.end_row();
                            ui.label("Радиус:");
                            ui.label(format!("{:.1}", body.radius));
                            ui.end_row();
                        });
                    if ui.button("Удалить тело").clicked() {
                        app.physics.send(Command::RemoveBody(body.id));
                        app.selected_id = None;
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new("Тело не выбрано.\nКликните по телу на холсте или в списке.")
                            .small()
                            .weak(),
                    );
                }
            }
        });
    });
}
