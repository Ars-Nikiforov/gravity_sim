//! Методы-действия [`GravityApp`]: команды физике, пресеты, фокус камеры,
//! логирование и управление повтором. Тела окон — в [`super::gallery`]
//! и [`super::windows`].

use std::path::Path;

use eframe::egui;

use crate::io::logging::{LogMode, Logger};
use crate::io::replay::{self, ReplayPlayer};
use crate::physics::{Body, Command, Integrator, Scenario, Vec3};
use crate::ui::canvas::Camera3D;
use crate::ui::Camera;

use super::{barycenter, barycenter_filtered, FocusMode, GravityApp};

impl GravityApp {
    /// Отправить текущие UI-параметры в поток физики.
    pub fn send_params(&self) {
        self.physics.send(Command::SetParams(self.params));
    }

    pub fn toggle_run(&self) {
        self.physics.set_running(!self.physics.is_running());
    }

    /// Перезапустить симуляцию с выбранным сценарием.
    pub fn reset(&mut self, scenario: Scenario) {
        self.scenario = scenario;
        self.physics.send(Command::Reset(scenario));
        self.trails.clear();
        self.prediction = None;
        self.orbits.clear();
        self.orbits_tick = 0;
        self.selection.clear();
        self.selected_id = None;
        self.camera = Camera::default();
        self.cam3d = Camera3D::default();
        self.focus_mode = FocusMode::Free;
        self.frame_pending = true;
        self.orbit_host = None;
    }

    /// Загрузить пресет: применить рекомендованные параметры интегрирования
    /// и кадрировать сцену после появления первых тел.
    pub fn load_preset(&mut self, scenario: Scenario) {
        if let Some(rec) = scenario.info().recommended {
            self.params.integrator = rec.integrator;
            self.params.dt = rec.dt;
            self.params.softening = rec.softening;
            self.params.speed = rec.speed;
            self.send_params();
        }
        self.reset(scenario);
    }

    /// Загрузить произвольный набор тел (генератор систем): умеренные
    /// параметры интегрирования, смена поколения состояния.
    pub fn load_custom_bodies(&mut self, bodies: Vec<Body>) {
        self.params.integrator = Integrator::SymplecticEuler;
        self.params.dt = 0.008;
        self.params.softening = 4.0;
        self.params.speed = 1.0;
        self.send_params();
        self.physics.send(Command::ReplaceState(bodies));
        self.trails.clear();
        self.prediction = None;
        self.orbits.clear();
        self.orbits_tick = 0;
        self.selection.clear();
        self.selected_id = None;
        self.orbit_host = None;
        self.frame_pending = true;
    }

    /// Разово переместить камеру в точку мира.
    pub fn focus_point_once(&mut self, p: Vec3) {
        self.camera.center = p.xy();
        self.cam3d.target = p;
        self.focus_mode = FocusMode::Free;
    }

    /// Разово центрировать камеру на барицентре всей системы.
    pub fn focus_system_once(&mut self) {
        let bodies = self.physics.snapshot().bodies.clone();
        self.focus_point_once(barycenter(&bodies));
    }

    /// Разово центрировать камеру на барицентре выделения (или выбранного тела).
    pub fn focus_selection_once(&mut self) {
        let bodies = self.physics.snapshot().bodies.clone();
        let p = if self.selection.is_empty() {
            self.selected_id
                .and_then(|id| bodies.iter().find(|b| b.id == id))
                .map(|b| b.pos)
        } else {
            barycenter_filtered(&bodies, |b| self.selection.contains(&b.id))
        };
        if let Some(p) = p {
            self.focus_point_once(p);
        }
    }

    pub fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_visuals(if self.dark_theme {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
    }

    // ── Логирование ─────────────────────────────────────────────
    pub fn start_logging(&mut self) {
        self.stop_logging();
        match Logger::start(&self.params, self.log_mode) {
            Ok(logger) => {
                let sender = logger.sender();
                let (mode, every) = (self.log_mode, self.log_every);
                self.physics.send(Command::SetLogging {
                    mode,
                    every,
                    sender,
                });
                self.logger = Some(logger);
            }
            Err(e) => {
                self.replay_error = Some(format!("лог не запущен: {e}"));
            }
        }
    }

    pub fn stop_logging(&mut self) {
        self.physics.send(Command::SetLogging {
            mode: LogMode::Off,
            every: 1,
            sender: None,
        });
        if let Some(mut logger) = self.logger.take() {
            logger.stop();
        }
    }

    // ── Повтор по логу ──────────────────────────────────────────
    pub fn start_replay(&mut self, path: &Path) {
        match ReplayPlayer::load(path) {
            Ok(mut player) => {
                // На время повтора глушим живую физику (и запоминаем, вернём).
                self.replay_return_running = self.physics.is_running();
                if self.replay_return_running {
                    self.physics.set_running(false);
                }
                player.playing = true;
                self.replay = Some(player);
                self.trails.clear();
                self.prediction = None;
                self.selection.clear();
                self.selected_id = None;
                self.replay_error = None;
                self.load_dialog = false;
                self.frame_pending = true;
            }
            Err(e) => {
                self.replay_error = Some(format!("{}: {e}", path.display()));
            }
        }
    }

    pub fn start_replay_latest(&mut self) {
        if let Some(latest) = replay::latest_log(Path::new("logs")) {
            self.load_path = latest.display().to_string();
            self.start_replay(&latest);
        } else {
            self.replay_error = Some("каталог logs/ пуст — логов ещё нет".into());
            self.load_dialog = true;
        }
    }

    pub fn exit_replay(&mut self) {
        self.replay = None;
        self.trails.clear();
        self.prediction = None;
        self.selection.clear();
        self.selected_id = None;
        if self.replay_return_running {
            self.physics.set_running(true);
        }
    }

    /// Продолжить живую симуляцию с текущей отметки повтора.
    pub fn continue_from_replay(&mut self) {
        if let Some(rp) = &self.replay {
            self.physics
                .send(Command::ReplaceState(rp.current().bodies.clone()));
        }
        self.replay = None;
        self.trails.clear();
        self.prediction = None;
        self.selection.clear();
        self.selected_id = None;
        self.physics.set_running(true);
    }

}
