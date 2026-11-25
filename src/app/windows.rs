//! Плавающие окна повтора лога: управление проигрыванием и диалог
//! загрузки JSONL-файла.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::io::replay;

use super::GravityApp;

impl GravityApp {
    /// Окно управления повтором (плавающее поверх холста).
    pub(super) fn show_replay_window(&mut self, ctx: &egui::Context) {
        if self.replay.is_none() {
            return;
        }
        let mut exit = false;
        let mut resume = false;
        egui::Window::new("Повтор лога")
            .id(egui::Id::unique("replay-window"))
            .collapsible(false)
            .show(ctx, |ui| {
                let Some(rp) = &mut self.replay else { return };
                ui.label(format!(
                    "Файл: {}",
                    rp.file
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                ));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button(if rp.playing { "Пауза" } else { "Пуск" })
                        .clicked()
                    {
                        rp.playing = !rp.playing;
                    }
                    ui.add(egui::Slider::new(&mut rp.speed, 0.1..=8.0).text("Скорость"));
                });
                let max = rp.len().saturating_sub(1);
                if ui
                    .add(egui::Slider::new(&mut rp.idx, 0..=max).text("Кадр"))
                    .changed()
                {
                    rp.note_scrub();
                }
                let frame = rp.current();
                ui.label(format!(
                    "Кадров: {} · тик {} · t = {:.1}",
                    rp.len(),
                    frame.tick,
                    frame.sim_time
                ));
                if rp.skipped_lines > 0 {
                    ui.label(
                        egui::RichText::new(format!("Пропущено битых строк: {}", rp.skipped_lines))
                            .small()
                            .weak(),
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Продолжить симуляцию здесь").clicked() {
                        resume = true;
                    }
                    if ui.button("Выйти из повтора").clicked() {
                        exit = true;
                    }
                });
            });
        if exit {
            self.exit_replay();
        }
        if resume {
            self.continue_from_replay();
        }
    }

    /// Диалог загрузки лога по пути.
    pub(super) fn show_load_dialog(&mut self, ctx: &egui::Context) {
        if !self.load_dialog {
            return;
        }
        let mut do_load = false;
        egui::Window::new("Повтор из файла")
            .open(&mut self.load_dialog)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Путь к JSONL-логу:");
                ui.text_edit_singleline(&mut self.load_path);
                if let Some(latest) = replay::latest_log(Path::new("logs")) {
                    if ui
                        .button(format!("Последний: {}", latest.display()))
                        .clicked()
                    {
                        self.load_path = latest.display().to_string();
                    }
                }
                if ui.button("Загрузить").clicked() {
                    do_load = true;
                }
                if let Some(err) = &self.replay_error {
                    ui.label(egui::RichText::new(err).color(egui::Color32::LIGHT_RED));
                }
            });
        if do_load && !self.load_path.trim().is_empty() {
            let path = PathBuf::from(self.load_path.trim());
            self.start_replay(&path);
        }
    }
}
