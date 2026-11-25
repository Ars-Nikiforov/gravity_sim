//! Состояние приложения и общий каркас интерфейса.
//!
//! Модуль разбит по обязанностям:
//! * этот файл — структура [`GravityApp`], перечисления инструментов и
//!   режимов, конструирование, каркас кадров `eframe::App` и свободные
//!   функции-хелперы (барицентр, контрольная сумма);
//! * [`actions`] — методы-действия: параметры, пресеты, фокус камеры,
//!   логирование, повтор;
//! * [`gallery`] — окно галереи пресетов, миниатюры, генератор систем;
//! * [`windows`] — плавающие окна повтора и загрузки лога.
//!
//! Макет (сверху вниз, слева направо):
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ Меню: Файл | Вид | Симуляция | Настройки …   │
//! ├────────┬─────────────────────────┬───────────┤
//! │ Левая  │      Дисплей планет     │  Правая   │
//! │ панель │      (canvas, зум,      │  панель   │
//! │ (инст- │       панорама, силы,   │ (инспек-  │
//! │ румент)│       поле, 3D-колодец) │  тор)     │
//! └────────┴─────────────────────────┴───────────┘
//! ```

use std::collections::{HashMap, HashSet, VecDeque};

use eframe::egui;

use crate::io::logging::{LogMode, Logger};
use crate::io::replay::ReplayPlayer;
use crate::physics::{
    Body, Command, PhysicsWorker, Scenario, SimParams, StableSystemSpec, Vec3,
};
use crate::ui::canvas::Camera3D;
use crate::ui::heatmap::HeatmapState;
use crate::ui::{Camera, canvas, left_panel, menu, right_panel};

mod actions;
mod gallery;
mod windows;

/// Активный инструмент для левой кнопки мыши на холсте.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Клик — выбор тела, drag — панорама камеры.
    Select,
    /// Клик — добавить тело с орбитальной скоростью.
    AddBody,
    /// Быстрая орбита: первый клик — планета-хозяин, второй — тело на
    /// устойчивой круговой орбите вокруг неё.
    Orbit,
    /// Клик по телу — удалить его.
    DeleteBody,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Выбор",
            Tool::AddBody => "Добавить тело",
            Tool::Orbit => "Быстрая орбита",
            Tool::DeleteBody => "Удалить тело",
        }
    }

    pub const ALL: [Tool; 4] = [Tool::Select, Tool::AddBody, Tool::Orbit, Tool::DeleteBody];
}

/// Раскраска тел на холсте.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Исходные цвета тел.
    Normal,
    /// По модулю гравитационного ускорения (лог-шкала, тепло).
    ByGravity,
}

impl ColorMode {
    pub fn name(self) -> &'static str {
        match self {
            ColorMode::Normal => "Обычная",
            ColorMode::ByGravity => "По тяготению",
        }
    }

    pub const ALL: [ColorMode; 2] = [ColorMode::Normal, ColorMode::ByGravity];
}

/// Режим слежения камеры за точкой мира.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FocusMode {
    /// Камера не двигается сама.
    #[default]
    Free,
    /// Центр — выбранное тело.
    FollowSelected,
    /// Центр — барицентр всей системы.
    FollowSystem,
    /// Центр — барицентр выделенной области (рамкой).
    FollowSelection,
}

impl FocusMode {
    pub fn name(self) -> &'static str {
        match self {
            FocusMode::Free => "Свободная камера",
            FocusMode::FollowSelected => "Следить за выбранным телом",
            FocusMode::FollowSystem => "Следить за системой",
            FocusMode::FollowSelection => "Следить за выделением",
        }
    }

    pub const ALL: [FocusMode; 4] = [
        FocusMode::Free,
        FocusMode::FollowSelected,
        FocusMode::FollowSystem,
        FocusMode::FollowSelection,
    ];
}

/// Параметры тела, создаваемого инструментом «Добавить тело».
#[derive(Clone, Copy, Debug)]
pub struct NewBodyParams {
    pub mass: f64,
    pub radius: f64,
    pub color: [u8; 3],
}

/// Состояние 3D-колодца (поворот мышью + авто-вращение).
pub struct WellState {
    pub yaw: f32,
    /// Момент последнего взаимодействия — после паузы возобновляем авто-вращение.
    pub last_drag: f64,
}

impl Default for WellState {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            last_drag: f64::NEG_INFINITY,
        }
    }
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

    // ── Визуализация поля и сил ──────────────────────────────────
    pub show_forces: bool,
    pub show_field_heatmap: bool,
    /// Настройки тепловой карты: поле, разрешение, изолинии + кэш.
    pub heatmap: HeatmapState,
    /// Оверлеи небесной механики.
    pub show_lagrange: bool,
    pub show_hill: bool,
    pub show_well3d: bool,
    pub color_mode: ColorMode,
    pub well: WellState,
    /// 3D-режим: перспективная орбитальная камера вместо вида сверху.
    pub view3d: bool,
    pub cam3d: Camera3D,
    /// Слежение камеры и авто-подстройка масштаба.
    pub focus_mode: FocusMode,
    pub auto_center: bool,
    pub auto_fit: bool,
    /// Мультивыбор рамкой: id тел, попавших в область.
    pub selection: HashSet<u64>,
    /// Начало рамки выделения (экранные координаты) и режим добавления.
    pub box_select_start: Option<egui::Pos2>,
    pub box_select_additive: bool,
    /// Поколение снапшота: при смене сбрасываем следы/предсказание.
    pub last_epoch: u64,
    /// Предсказание траектории выбранного тела (траектория-призрак).
    pub show_prediction: bool,
    pub prediction: Option<canvas::Prediction>,
    /// Орбиты-предсказания для всех тел (кнопка в панели визуализации).
    pub show_orbits: bool,
    /// Сливать орбиты мелких тел в скоплениях: рисовать только для
    /// самых массивных, иначе сотни путей сливаются в шум.
    pub merge_small_orbits: bool,
    /// Кэш орбит-предсказаний: (id тела, путь) + тик пересчёта.
    pub orbits: Vec<(u64, Vec<Vec3>)>,
    pub orbits_tick: u64,

    // ── Логирование и повтор ─────────────────────────────────────
    pub log_mode: LogMode,
    /// Период записи: каждый N-й тик.
    pub log_every: u64,
    pub logger: Option<Logger>,
    pub replay: Option<ReplayPlayer>,
    /// Была ли симуляция запущена до входа в повтор.
    replay_return_running: bool,
    pub load_dialog: bool,
    pub load_path: String,
    pub replay_error: Option<String>,

    pub selected_id: Option<u64>,
    pub camera: Camera,
    /// Следы орбит — чисто визуальная история на стороне UI.
    pub trails: HashMap<u64, VecDeque<Vec3>>,
    pub fps: f32,

    // ── Новые возможности ────────────────────────────────────────
    /// Окно галереи пресетов.
    pub show_gallery: bool,
    /// Кадрировать сцену на ближайшем кадре (после сброса/загрузки пресета).
    pub frame_pending: bool,
    /// Выбранная планета-хозяин для инструмента «Быстрая орбита».
    pub orbit_host: Option<u64>,
    /// Показывать листья мультипольного разбиения.
    pub show_regions: bool,
    /// Кэш миниатюр пресетов: сгенерированные тела (позиция, радиус, цвет).
    pub thumbnails: HashMap<Scenario, Vec<(Vec3, f64, [u8; 3])>>,
    /// Спецификация генератора стабильных систем (окно галереи).
    pub gen_spec: StableSystemSpec,
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
            show_forces: false,
            show_field_heatmap: false,
            heatmap: HeatmapState::default(),
            show_lagrange: false,
            show_hill: false,
            show_well3d: false,
            color_mode: ColorMode::Normal,
            well: WellState::default(),
            view3d: false,
            cam3d: Camera3D::default(),
            focus_mode: FocusMode::Free,
            auto_center: false,
            auto_fit: false,
            selection: HashSet::new(),
            box_select_start: None,
            box_select_additive: false,
            last_epoch: 0,
            show_prediction: true,
            prediction: None,
            show_orbits: false,
            merge_small_orbits: true,
            orbits: Vec::new(),
            orbits_tick: 0,
            log_mode: LogMode::Deltas,
            log_every: 10,
            logger: None,
            replay: None,
            replay_return_running: false,
            load_dialog: false,
            load_path: String::new(),
            replay_error: None,
            selected_id: None,
            camera: Camera::default(),
            trails: HashMap::new(),
            fps: 0.0,
            show_gallery: false,
            frame_pending: true,
            orbit_host: None,
            show_regions: false,
            thumbnails: HashMap::new(),
            gen_spec: StableSystemSpec::default(),
        }
    }

}

/// Контрольная сумма состояния (FNV-1a по отсортированным телам) —
/// численное доказательство детерминированности: одинаковые запуски
/// дают одинаковую цепочку сумм.
pub fn state_checksum(bodies: &[Body]) -> u64 {
    let mut sorted: Vec<&Body> = bodies.iter().collect();
    sorted.sort_unstable_by_key(|b| b.id);
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in sorted {
        for bytes in [
            b.id.to_le_bytes(),
            b.mass.to_bits().to_le_bytes(),
            b.pos.x.to_bits().to_le_bytes(),
            b.pos.y.to_bits().to_le_bytes(),
            b.pos.z.to_bits().to_le_bytes(),
            b.vel.x.to_bits().to_le_bytes(),
            b.vel.y.to_bits().to_le_bytes(),
            b.vel.z.to_bits().to_le_bytes(),
        ] {
            for byte in bytes {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    hash
}

/// Барицентр всех тел (по массе). Если тел нет — начало координат.
pub fn barycenter(bodies: &[Body]) -> Vec3 {
    barycenter_filtered(bodies, |_| true).unwrap_or(Vec3::ZERO)
}

/// Барицентр подмножества тел, прошедших фильтр.
pub fn barycenter_filtered(bodies: &[Body], keep: impl Fn(&Body) -> bool) -> Option<Vec3> {
    let mut mass = 0.0;
    let mut sum = Vec3::ZERO;
    for b in bodies.iter().filter(|b| keep(*b)) {
        if b.mass.is_finite() && b.mass > 0.0 {
            mass += b.mass;
            sum += b.pos * b.mass;
        }
    }
    if mass > 1e-12 && sum.is_finite() {
        Some(sum * (1.0 / mass))
    } else {
        None
    }
}


impl eframe::App for GravityApp {
    /// Новый API egui 0.34+: вместо `update(ctx)` метод `ui(&mut Ui)`.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Счётчик FPS (EMA) — виден эффект асинхронности: интерфейс не тормозит,
        // даже если шаг физики стал тяжёлым.
        let frame_dt = ui.ctx().input(|i| i.unstable_dt).max(1e-6);
        self.fps = self.fps * 0.9 + (1.0 / frame_dt) * 0.1;

        // Пробел — пауза/продолжить, R — обратить время вспять.
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            self.toggle_run();
        }
        if ui.ctx().input(|i| i.key_pressed(egui::Key::R)) {
            self.physics.send(Command::ReverseVelocities);
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

        // Плавающие окна поверх всего.
        self.show_replay_window(ui.ctx());
        self.show_load_dialog(ui.ctx());
        self.show_gallery_window(ui.ctx());
    }
}
