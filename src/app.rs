//! Состояние приложения и общий каркас интерфейса.
//!
//! Макет (сверху вниз, слева направо):
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ Меню: Файл | Вид | Симуляция | Настройки …   │
//! ├────────┬─────────────────────────┬───────────┤
//! │ Левая  │      Дисплей планет     │  Правая   │
//! │ панель │      (canvas, зум,      │  панель   │
//! │ (инст- │       панорама)         │ (инспек-  │
//! │ румент)│                         │  тор)     │
//! └────────┴─────────────────────────┴───────────┘
//! ```

use std::collections::{HashMap, VecDeque};

use eframe::egui;

use crate::physics::{Command, PhysicsWorker, Scenario, SimParams, Vec2};
use crate::ui::{canvas, left_panel, menu, right_panel, Camera};

/// Активный инструмент для левой кнопки мыши на холсте.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Клик — выбор тела, drag — панорама камеры.
    Select,
    /// Клик — добавить тело с орбитальной скоростью.
    AddBody,
    /// Клик по телу — удалить его.
    DeleteBody,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Выбор",
            Tool::AddBody => "Добавить тело",
            Tool::DeleteBody => "Удалить тело",
        }
    }

    pub const ALL: [Tool; 3] = [Tool::Select, Tool::AddBody, Tool::DeleteBody];
}

/// Параметры тела, создаваемого инструментом «Добавить тело».
#[derive(Clone, Copy, Debug)]
pub struct NewBodyParams {
    pub mass: f64,
    pub radius: f64,
    pub color: [u8; 3],
}

pub struct GravityApp {
    pub physics: PhysicsWorker,
    /// Редактируемая копия параметров: слайдеры меняют её, затем уходит команда.
    pub params: SimParams,
    pub scenario: Scenario,
    pub tool: Tool,
    pub new_body: NewBodyParams,

    pub show_left: bool,
    pub show_right: bool,
    pub show_trails: bool,
    pub show_labels: bool,
    pub show_fps: bool,
    pub dark_theme: bool,

    pub selected_id: Option<u64>,
    pub camera: Camera,
    /// Следы орбит — чисто визуальная история на стороне UI.
    pub trails: HashMap<u64, VecDeque<Vec2>>,
    pub fps: f32,
}

impl GravityApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let params = SimParams::default();
        let scenario = Scenario::SolarSystem;
        let physics = PhysicsWorker::spawn(cc.egui_ctx.clone(), params, scenario);
        Self {
            physics,
            params,
            scenario,
            tool: Tool::Select,
            new_body: NewBodyParams {
                mass: 5.0,
                radius: 3.0,
                color: [120, 200, 255],
            },
            show_left: true,
            show_right: true,
            show_trails: true,
            show_labels: false,
            show_fps: true,
            dark_theme: true,
            selected_id: None,
            camera: Camera::default(),
            trails: HashMap::new(),
            fps: 0.0,
        }
    }

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
        self.selected_id = None;
        self.camera = Camera::default();
    }

    pub fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_visuals(if self.dark_theme {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
    }
}

impl eframe::App for GravityApp {
    /// Новый API egui 0.34+: вместо `update(ctx)` метод `ui(&mut Ui)`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Счётчик FPS (EMA) — виден эффект асинхронности: интерфейс не тормозит,
        // даже если шаг физики стал тяжёлым.
        let frame_dt = ui.ctx().input(|i| i.unstable_dt).max(1e-6);
        self.fps = self.fps * 0.9 + (1.0 / frame_dt) * 0.1;

        // Пробел — пауза/продолжить.
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_run();
        }

        // Порядок добавления панелей важен: каждая занимает свою часть области.
        egui::Panel::top("menu_bar").show(ui, |ui| menu::show(ui, self));

        if self.show_left {
            egui::Panel::left("left_panel")
                .resizable(true)
                .default_size(280.0)
                .min_size(220.0)
                .max_size(450.0)
                .show(ui, |ui| left_panel::show(ui, self));
        }

        if self.show_right {
            egui::Panel::right("right_panel")
                .resizable(true)
                .default_size(300.0)
                .min_size(240.0)
                .max_size(460.0)
                .show(ui, |ui| right_panel::show(ui, self));
        }

        egui::CentralPanel::default_margins().show(ui, |ui| canvas::show(ui, self));
    }
}
