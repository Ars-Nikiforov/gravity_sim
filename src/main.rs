//! Гравитационный симулятор — заготовка интерфейса.
//!
//! Архитектура:
//! * `physics` — движок и отдельный поток расчёта (не знает про egui);
//! * `logging` — асинхронная запись симуляции в JSONL-лог;
//! * `replay` — повтор симуляции по логу;
//! * `ui` — меню, панели и дисплей;
//! * `app` — состояние и сборка интерфейса.
//!
//! UI и физика асинхронны: интерфейс всегда рисует последний готовый
//! снапшот и просыпается по `request_repaint` от потока физики.

mod app;
mod logging;
mod physics;
mod replay;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("Гравитационный симулятор"),
        ..Default::default()
    };

    eframe::run_native(
        "grav_sim",
        options,
        Box::new(|cc| Ok(Box::new(app::GravityApp::new(cc)))),
    )
}
