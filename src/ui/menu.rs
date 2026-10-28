//! Верхнее меню (новый `egui::MenuBar` из egui 0.32+).
//! Меню закрываются автоматически после клика по пункту действия.

use eframe::egui;

use crate::app::{FocusMode, GravityApp};
use crate::physics::{Command, Scenario};

pub fn show(ui: &mut egui::Ui, app: &mut GravityApp) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("Файл", |ui| {
            ui.label(egui::RichText::new("Загрузить сценарий").weak());
            for scenario in Scenario::ALL {
                if ui.button(scenario.name()).clicked() {
                    app.reset(scenario);
                }
            }
            ui.separator();
            if ui.button("Очистить холст").clicked() {
                app.physics.send(Command::Clear);
                app.trails.clear();
                app.selected_id = None;
            }
            ui.separator();
            if ui.button("Повтор последнего лога").clicked() {
                app.start_replay_latest();
            }
            if ui.button("Повтор из файла…").clicked() {
                app.load_dialog = true;
                app.replay_error = None;
            }
            ui.separator();
            if ui.button("Выход").clicked() {
                ui.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });

        ui.menu_button("Вид", |ui| {
            ui.checkbox(&mut app.show_left, "Левая панель");
            ui.checkbox(&mut app.show_right, "Правая панель");
            ui.separator();
            ui.checkbox(&mut app.view3d, "3D-режим (перспектива)");
            ui.checkbox(&mut app.show_prediction, "Предсказание орбиты");
            ui.checkbox(&mut app.show_trails, "Следы орбит");
            ui.checkbox(&mut app.show_labels, "Подписи тел");
            ui.checkbox(&mut app.show_forces, "Векторы сил");
            ui.checkbox(&mut app.show_field_heatmap, "Тепловая карта поля");
            if app.show_field_heatmap {
                egui::ComboBox::from_id_salt("menu_field_kind")
                    .selected_text(app.heatmap.kind.name())
                    .show_ui(ui, |ui| {
                        for kind in crate::ui::heatmap::FieldKind::ALL {
                            ui.selectable_value(&mut app.heatmap.kind, kind, kind.name());
                        }
                    });
                ui.add(egui::Slider::new(&mut app.heatmap.res, 16..=128).text("Разрешение поля"));
                ui.checkbox(&mut app.heatmap.contours, "Изолинии");
            }
            ui.checkbox(&mut app.show_lagrange, "Точки Лагранжа L1–L5");
            ui.checkbox(&mut app.show_hill, "Сфера Хилла");
            ui.checkbox(&mut app.show_well3d, "3D-колодец");
            ui.separator();
            ui.checkbox(&mut app.auto_center, "Автоцентр (не давать улетать)");
            ui.checkbox(&mut app.auto_fit, "Авто-масштаб (вписать тела)");
            if ui.button("Центр: система").clicked() {
                app.focus_system_once();
            }
            if ui.button("Центр: выбор").clicked() {
                app.focus_selection_once();
            }
            ui.menu_button("Слежение", |ui| {
                for mode in FocusMode::ALL {
                    ui.selectable_value(&mut app.focus_mode, mode, mode.name());
                }
            });
            ui.separator();
            if ui.button("Сбросить камеру").clicked() {
                app.camera = Default::default();
                app.cam3d = Default::default();
                app.focus_mode = FocusMode::Free;
            }
        });

        ui.menu_button("Симуляция", |ui| {
            let running = app.physics.is_running();
            let label = if running {
                "Пауза (Space)"
            } else {
                "Запустить (Space)"
            };
            if ui.button(label).clicked() {
                app.toggle_run();
            }
            if ui.button("Один шаг").clicked() {
                app.physics.send(Command::StepOnce);
            }
            if ui.button("⏪ Обратить время (R)").clicked() {
                app.physics.send(Command::ReverseVelocities);
            }
            ui.separator();
            if ui.button("Перезапустить сценарий").clicked() {
                app.reset(app.scenario);
            }
        });

        ui.menu_button("Настройки", |ui| {
            if ui
                .checkbox(&mut app.dark_theme, "Тёмная тема")
                .changed()
            {
                app.apply_theme(ui.ctx());
            }
            ui.separator();
            ui.checkbox(&mut app.show_fps, "Счётчик FPS");
        });

        ui.menu_button("Справка", |ui| {
            ui.label("Гравитационный симулятор (заготовка интерфейса).");
            ui.separator();
            ui.label("Управление:");
            ui.label("• ЛКМ — применить инструмент");
            ui.label("• ПКМ / drag — панорама камеры");
            ui.label("• Колесо — зум к курсору");
            ui.label("• Space — пауза/запуск");
            ui.label("• Вид → Точки Лагранжа / Сфера Хилла");
            ui.label("• Тепловая карта: поле, изолинии и разрешение — слева");
            ui.label("• Физика считается в отдельном потоке");
        });
    });
}
