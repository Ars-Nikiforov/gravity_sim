//! Состояние приложения и общий каркас интерфейса.
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
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::logging::{LogMode, Logger};
use crate::physics::{
    Body, Command, PhysicsWorker, PresetCategory, Scenario, SimParams, Stability, Vec3,
};
use crate::replay::{self, ReplayPlayer};
use crate::ui::canvas::Camera3D;
use crate::ui::heatmap::HeatmapState;
use crate::ui::{Camera, canvas, left_panel, menu, right_panel};

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
        self.prediction = None;
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

    /// Окно галереи пресетов (плавающее, с описаниями и устойчивостью).
    fn show_gallery_window(&mut self, ctx: &egui::Context) {
        if !self.show_gallery {
            return;
        }
        let mut open = true;
        let mut load: Option<Scenario> = None;
        egui::Window::new("Галерея пресетов")
            .open(&mut open)
            .default_width(620.0)
            .default_height(540.0)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Готовые конфигурации: от точных хореографий до иерархических \
                         систем. Устойчивость указана по литературе и критериям Хилла.",
                    )
                    .small()
                    .weak(),
                );
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for category in PresetCategory::ALL {
                        ui.add_space(8.0);
                        ui.heading(category.name());
                        ui.label(egui::RichText::new(category.hint()).small().weak());
                        for scenario in Scenario::in_category(category) {
                            let info = scenario.info();
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.strong(info.name);
                                    let color = match info.stability {
                                        Stability::Stable => egui::Color32::from_rgb(120, 235, 160),
                                        Stability::LongLived => {
                                            egui::Color32::from_rgb(150, 210, 255)
                                        }
                                        Stability::WeaklyUnstable => {
                                            egui::Color32::from_rgb(255, 200, 110)
                                        }
                                        Stability::Demo => egui::Color32::from_rgb(210, 170, 255),
                                    };
                                    ui.label(
                                        egui::RichText::new(info.stability.label())
                                            .small()
                                            .color(color),
                                    )
                                    .on_hover_text(info.stability.hint());
                                    if ui.button("Загрузить").clicked() {
                                        load = Some(scenario);
                                    }
                                });
                                ui.label(egui::RichText::new(info.description).small());
                                if let Some(rec) = info.recommended {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Рекомендуется: {} · dt {:.3} · ε² {:.1} · ×{:.1}",
                                            rec.integrator.name(),
                                            rec.dt,
                                            rec.softening,
                                            rec.speed
                                        ))
                                        .small()
                                        .weak(),
                                    );
                                }
                            });
                        }
                    }
                });
            });
        if !open {
            self.show_gallery = false;
        }
        if let Some(scenario) = load {
            self.load_preset(scenario);
            self.show_gallery = false;
        }
    }

    /// Окно управления повтором (плавающее поверх холста).
    fn show_replay_window(&mut self, ctx: &egui::Context) {
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
    fn show_load_dialog(&mut self, ctx: &egui::Context) {
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
