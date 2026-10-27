//! Верхнее меню (новый `egui::MenuBar` из egui 0.32+).
//! Меню закрываются автоматически после клика по пункту действия.

use eframe::egui;

use crate::app::GravityApp;
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
            if ui.button("Выход").clicked() {
                ui.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });

        ui.menu_button("Вид", |ui| {
            ui.checkbox(&mut app.show_left, "Левая панель");
            ui.checkbox(&mut app.show_right, "Правая панель");
            ui.separator();
            ui.checkbox(&mut app.show_trails, "Следы орбит");
            ui.checkbox(&mut app.show_labels, "Подписи тел");
            ui.separator();
            if ui.button("Сбросить камеру").clicked() {
                app.camera = Default::default();
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
            ui.label("• Физика считается в отдельном потоке");
        });
    });
}
