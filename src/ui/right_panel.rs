//! Правая панель — инспектор: статистика симуляции, настройки физики,
//! список тел и свойства выбранного тела.

use eframe::egui;

use crate::app::GravityApp;
use crate::physics::{Algorithm, Command};
use crate::ui::overlays;

pub fn show(ui: &mut egui::Ui, app: &mut GravityApp) {
    let snapshot = app.physics.snapshot();

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(8.0);
        ui.heading("Инспектор");
        ui.separator();

        // ── Статистика ──────────────────────────────────────────────
        ui.group(|ui| {
            ui.label("Статистика:");
            egui::Grid::new("stats")
                .num_columns(2)
                .spacing([20.0, 4.0])
                .show(ui, |ui| {
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

                    if app.params.algorithm == Algorithm::FastMultipole {
                        ui.label("Областей FMM:");
                        ui.label(snapshot.regions.len().to_string());
                        ui.end_row();
                    }

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

                    ui.label("Фокус камеры:");
                    ui.label(app.focus_mode.name());
                    ui.end_row();

                    ui.label("Выделено тел:");
                    ui.label(app.selection.len().to_string());
                    ui.end_row();
                });
        });

        // ── Точки Лагранжа и сфера Хилла ────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Точки Лагранжа:");
            match overlays::dominant_pair(&snapshot.bodies) {
                Some(pair) => {
                    let lag = overlays::lagrange(&pair);
                    egui::Grid::new("lagrange_info")
                        .num_columns(2)
                        .spacing([16.0, 3.0])
                        .show(ui, |ui| {
                            ui.label("Пара:");
                            ui.label(format!("#{} ↔ #{}", pair.id1, pair.id2));
                            ui.end_row();
                            ui.label("μ = m₂/(m₁+m₂):");
                            ui.label(format!("{:.5}", lag.mu));
                            ui.end_row();
                            ui.label("R Хилла:");
                            ui.label(format!("{:.2}", lag.hill2));
                            ui.end_row();
                            ui.label("L1 от вторичного:");
                            ui.label(format!("{:.2}", (lag.l1 - pair.p2).len()));
                            ui.end_row();
                            ui.label("L2 от вторичного:");
                            ui.label(format!("{:.2}", (lag.l2 - pair.p2).len()));
                            ui.end_row();
                        });
                    ui.separator();
                    ui.label(egui::RichText::new("Вставить тело из левой панели в точку:").small());
                    ui.horizontal_wrapped(|ui| {
                        for (label, p) in [
                            ("L1", lag.l1),
                            ("L2", lag.l2),
                            ("L3", lag.l3),
                            ("L4", lag.l4),
                            ("L5", lag.l5),
                        ] {
                            let help = "Скорость: v барицентра + ω × (r − r_барицентра)";
                            if ui.button(label).on_hover_text(help).clicked() {
                                let nb = app.new_body;
                                let vel = overlays::lagrange_velocity(&pair, p);
                                app.physics.send(Command::AddBodyState {
                                    mass: nb.mass,
                                    radius: nb.radius,
                                    color: nb.color,
                                    pos: p,
                                    vel,
                                });
                            }
                        }
                    });
                    if !app.show_lagrange && !app.show_hill {
                        ui.label(
                            egui::RichText::new(
                                "Показ точек/сферы Хилла — в левой панели или меню «Вид».",
                            )
                            .small()
                            .weak(),
                        );
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new("Нужно минимум два тела с массой")
                            .small()
                            .weak(),
                    );
                }
            }
            ui.label(
                egui::RichText::new("Круговая ограниченная задача трёх тел")
                    .small()
                    .weak(),
            );
        });

        // ── Параметры физики ────────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Физика:");
            let mut changed = false;
            for algorithm in Algorithm::ALL {
                changed |= ui
                    .radio_value(&mut app.params.algorithm, algorithm, algorithm.name())
                    .on_hover_text(algorithm.hint())
                    .changed();
            }
            ui.add_space(4.0);
            changed |= ui
                .add(egui::Slider::new(&mut app.params.g, 0.0..=50.0).text("Гравитация G"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut app.params.dt, 0.001..=0.05).text("Шаг dt"))
                .changed();
            changed |= ui
                .add(
                    egui::Slider::new(&mut app.params.softening, 0.0..=100.0)
                        .text("Сглаживание ε²"),
                )
                .changed();
            ui.add_space(4.0);
            ui.label("Интегратор:");
            for integrator in crate::physics::Integrator::ALL {
                changed |= ui
                    .radio_value(&mut app.params.integrator, integrator, integrator.name())
                    .changed();
            }
            if app.params.algorithm == Algorithm::FastMultipole {
                ui.separator();
                ui.label("Мультипольное разложение:");
                changed |= ui
                    .add(egui::Slider::new(&mut app.params.fmm_theta, 0.2..=1.0).text("Угол θ"))
                    .on_hover_text("Меньше θ — больше областей раскрывается, точнее и дороже.")
                    .changed();
                ui.horizontal(|ui| {
                    ui.label("Порядок:");
                    for (order, name) in [(0u8, "Монополь"), (1, "Диполь"), (2, "Квадруполь")]
                    {
                        changed |= ui
                            .radio_value(&mut app.params.fmm_order, order, name)
                            .changed();
                    }
                });
                ui.checkbox(&mut app.show_regions, "Показать области на холсте");
                ui.label(
                    egui::RichText::new(format!(
                        "Областей в последнем шаге: {}",
                        snapshot.regions.len()
                    ))
                    .small()
                    .weak(),
                );
            }
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
                            app.selection.clear();
                            app.selection.insert(body.id);
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
                            ui.label(format!(
                                "({:7.1}, {:7.1}, {:7.1})",
                                body.pos.x, body.pos.y, body.pos.z
                            ));
                            ui.end_row();
                            ui.label("Скорость:");
                            ui.label(format!(
                                "({:6.2}, {:6.2}, {:6.2})  |v| = {:5.2}",
                                body.vel.x,
                                body.vel.y,
                                body.vel.z,
                                body.vel.len()
                            ));
                            ui.end_row();
                            ui.label("Радиус:");
                            ui.label(format!("{:.1}", body.radius));
                            ui.end_row();
                        });
                    ui.horizontal(|ui| {
                        if ui.button("Навести камеру").clicked() {
                            app.focus_point_once(body.pos);
                        }
                        if ui.button("Следить").clicked() {
                            app.selected_id = Some(body.id);
                            app.focus_mode = crate::app::FocusMode::FollowSelected;
                        }
                    });
                    if ui.button("Удалить тело").clicked() {
                        app.physics.send(Command::RemoveBody(body.id));
                        app.selection.remove(&body.id);
                        app.selected_id = None;
                    }
                }
                None => {
                    ui.label(
                        egui::RichText::new(
                            "Тело не выбрано.\nКликните по телу на холсте или в списке.",
                        )
                        .small()
                        .weak(),
                    );
                }
            }
        });

        // ── Детерминированность ─────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Детерминированность:");
            ui.label(format!(
                "КС: {:016X}",
                crate::app::state_checksum(&snapshot.bodies)
            ));
            ui.label(
                egui::RichText::new(
                    "Одинаковые запуски дают одинаковые контрольные суммы \
                     (порядок обхода фиксирован, без плавающего параллелизма).",
                )
                .small()
                .weak(),
            );
        });
    });
}
