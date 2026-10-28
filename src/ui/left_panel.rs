//! Левая панель — инструментарий: выбор инструмента, параметры
//! создаваемого тела и управление ходом времени.

use eframe::egui;

use crate::app::{ColorMode, FocusMode, GravityApp, Tool};

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
                Tool::Orbit => ui.label(
                    egui::RichText::new(
                        "1) клик по планете-хозяину\n\
                         2) клик в стороне — тело на устойчивой орбите",
                    )
                    .small()
                    .weak(),
                ),
                Tool::DeleteBody => {
                    ui.label(egui::RichText::new("Клик по телу — удалить").small().weak())
                }
            }
        });

        // ── Параметры нового тела ───────────────────────────────────
        if app.tool == Tool::AddBody || app.tool == Tool::Orbit {
            ui.add_space(6.0);
            ui.group(|ui| {
                ui.label(if app.tool == Tool::Orbit {
                    "Новое тело на орбите:"
                } else {
                    "Новое тело:"
                });
                ui.add(
                    egui::Slider::new(&mut app.new_body.mass, 0.1..=2000.0)
                        .logarithmic(true)
                        .text("Масса"),
                );
                ui.add(egui::Slider::new(&mut app.new_body.radius, 0.5..=20.0).text("Радиус"));
                ui.horizontal(|ui| {
                    ui.label("Цвет:");
                    ui.color_edit_button_srgb(&mut app.new_body.color);
                });
                if app.tool == Tool::Orbit {
                    match app.orbit_host.and_then(|id| {
                        app.physics
                            .snapshot()
                            .bodies
                            .iter()
                            .find(|b| b.id == id)
                            .map(|b| (b.id, b.mass))
                    }) {
                        Some((id, mass)) => {
                            ui.label(
                                egui::RichText::new(format!("Хозяин: #{id} (m = {mass:.1})"))
                                    .small()
                                    .weak(),
                            );
                            if ui.button("Сбросить хозяина").clicked() {
                                app.orbit_host = None;
                            }
                        }
                        None => {
                            ui.label(
                                egui::RichText::new("Хозяин не выбран — кликните по планете")
                                    .small()
                                    .weak(),
                            );
                        }
                    }
                }
            });
        }

        // ── Управление временем ─────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Ход времени:");
            let speed_changed = ui
                .add(egui::Slider::new(&mut app.params.speed, 0.0..=10.0).text("Скорость, ×"))
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
            if ui.button("⏪ Обратить время (R)").clicked() {
                app.physics.send(crate::physics::Command::ReverseVelocities);
            }
        });

        // ── Визуализация ────────────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Визуализация:");
            ui.checkbox(&mut app.view3d, "3D-режим (перспектива)");
            if app.view3d {
                ui.label(
                    egui::RichText::new("ЛКМ — орбита камеры,\nПКМ — панорама, колесо — зум")
                        .small()
                        .weak(),
                );
            }
            ui.checkbox(&mut app.show_prediction, "Предсказание орбиты");
            if app.show_prediction {
                ui.label(
                    egui::RichText::new("Пунктир — будущее выбранного тела")
                        .small()
                        .weak(),
                );
            }
            ui.checkbox(&mut app.show_forces, "Векторы сил");
            ui.checkbox(&mut app.show_field_heatmap, "Тепловая карта поля");
            if app.show_field_heatmap {
                ui.horizontal(|ui| {
                    ui.label("Поле:");
                    egui::ComboBox::from_id_salt("field_kind")
                        .selected_text(app.heatmap.kind.name())
                        .show_ui(ui, |ui| {
                            for kind in crate::ui::heatmap::FieldKind::ALL {
                                ui.selectable_value(&mut app.heatmap.kind, kind, kind.name());
                            }
                        });
                });
                ui.add(egui::Slider::new(&mut app.heatmap.res, 16..=128).text("Разрешение"))
                    .on_hover_text(
                        "Число ячеек по длинной стороне. Больше — детальнее, но дороже.",
                    );
                ui.checkbox(&mut app.heatmap.contours, "Изолинии уровней");
                ui.label(
                    egui::RichText::new("Контраст подстраивается по перцентилям")
                        .small()
                        .weak(),
                );
            }
            ui.checkbox(&mut app.show_lagrange, "Точки Лагранжа L1–L5");
            ui.checkbox(&mut app.show_hill, "Сфера Хилла");
            if app.show_lagrange || app.show_hill {
                ui.label(
                    egui::RichText::new("Считается для двух самых массивных тел")
                        .small()
                        .weak(),
                );
            }
            ui.checkbox(&mut app.show_well3d, "3D-колодец (угол, в 2D)");
            if app.params.algorithm == crate::physics::Algorithm::FastMultipole {
                ui.checkbox(&mut app.show_regions, "Показывать области FMM");
            }
            ui.horizontal(|ui| {
                ui.label("Раскраска:");
                egui::ComboBox::from_id_salt("color_mode")
                    .selected_text(app.color_mode.name())
                    .show_ui(ui, |ui| {
                        for mode in ColorMode::ALL {
                            ui.selectable_value(&mut app.color_mode, mode, mode.name());
                        }
                    });
            });
        });

        // ── Пресеты ─────────────────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Пресеты:");
            if ui.button("🖼 Галерея пресетов…").clicked() {
                app.show_gallery = true;
            }
            ui.label(
                egui::RichText::new("22 устойчивые конфигурации по категориям")
                    .small()
                    .weak(),
            );
        });

        // ── Камера и слежение ───────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Камера и слежение:");
            ui.checkbox(&mut app.auto_center, "Автоцентр (не давать улетать)");
            ui.checkbox(&mut app.auto_fit, "Авто-масштаб (вписать тела)");
            ui.horizontal(|ui| {
                if ui.button("Центр: система").clicked() {
                    app.focus_system_once();
                }
                if ui.button("Центр: выбор").clicked() {
                    app.focus_selection_once();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Режим:");
                egui::ComboBox::from_id_salt("focus_mode")
                    .selected_text(app.focus_mode.name())
                    .show_ui(ui, |ui| {
                        for mode in FocusMode::ALL {
                            ui.selectable_value(&mut app.focus_mode, mode, mode.name());
                        }
                    });
            });
            if ui.button("Сбросить камеру").clicked() {
                app.camera = Default::default();
                app.cam3d = Default::default();
                app.focus_mode = FocusMode::Free;
                app.frame_pending = true;
            }
            ui.label(
                egui::RichText::new(
                    "Двойной клик по телу — навести камеру.\n\
                     Shift+ЛКМ — рамка выделения области.",
                )
                .small()
                .weak(),
            );
        });

        // ── Логирование ─────────────────────────────────────────────
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.label("Логирование:");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("log_mode")
                    .selected_text(app.log_mode.name())
                    .show_ui(ui, |ui| {
                        for mode in [
                            crate::logging::LogMode::Off,
                            crate::logging::LogMode::Deltas,
                            crate::logging::LogMode::Full,
                        ] {
                            ui.selectable_value(&mut app.log_mode, mode, mode.name());
                        }
                    });
            });
            ui.add(
                egui::Slider::new(&mut app.log_every, 1..=100u64)
                    .logarithmic(true)
                    .text("Каждые N тиков"),
            );
            ui.horizontal(|ui| {
                if app.logger.is_some() {
                    if ui.button("⏹ Стоп").clicked() {
                        app.stop_logging();
                    }
                    if let Some(logger) = &app.logger {
                        ui.label(
                            egui::RichText::new(format!("записей: {}", logger.record_count()))
                                .small()
                                .weak(),
                        );
                    }
                } else if ui.button("⏺ Запись").clicked() {
                    app.start_logging();
                }
            });
            if let Some(logger) = &app.logger {
                ui.label(
                    egui::RichText::new(format!("{}", logger.file.display()))
                        .small()
                        .weak(),
                );
            }
            ui.separator();
            ui.label("Повтор:");
            ui.horizontal(|ui| {
                if ui.button("Последний лог").clicked() {
                    app.start_replay_latest();
                }
                if ui.button("Файл…").clicked() {
                    app.load_dialog = true;
                    app.replay_error = None;
                }
            });
            if let Some(err) = &app.replay_error {
                if app.replay.is_none() {
                    ui.label(
                        egui::RichText::new(err)
                            .small()
                            .color(egui::Color32::LIGHT_RED),
                    );
                }
            }
        });

        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Пробел — пауза/пуск. R — обратить время. Колесо мыши — зум.")
                .small()
                .weak(),
        );
    });
}
