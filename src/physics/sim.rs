//! Чистое ядро симуляции: тела, параметры, шаг интегрирования и сценарии.
//! Ничего не знает ни про egui, ни про потоки — ими обёртки выше.

use std::time::Instant;

use super::profiler::StageStat;
use super::vec::Vec3;

#[derive(Clone, Debug)]
pub struct Body {
    pub id: u64,
    pub mass: f64,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Радиус в мировых единицах (масштабируется камерой при отрисовке).
    pub radius: f64,
    /// Цвет тела RGB.
    pub color: [u8; 3],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Algorithm {
    #[default]
    BruteForce,
    #[allow(clippy::upper_case_acronyms)]
    BarnesHut,
    FixedOrbits,
    /// Разбиение на области: близко — точно, далеко — приближённо.
    GridCells,
    /// Настоящий мультипольный метод (FMM-подобный): октодерево,
    /// монополь/диполь/квадруполь и локальные разложения.
    FastMultipole,
}

impl Algorithm {
    pub fn name(self) -> &'static str {
        match self {
            Algorithm::BruteForce => "Brute Force O(N²)",
            Algorithm::BarnesHut => "Barnes-Hut O(N log N)",
            Algorithm::FixedOrbits => "Fixed Orbits (Euler)",
            Algorithm::GridCells => "Сетка областей ~O(N)",
            Algorithm::FastMultipole => "Мультипольный FMM O(N log N)",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Algorithm::BruteForce => "Точный попарный расчёт. Эталон точности.",
            Algorithm::BarnesHut => "Заготовка: пока считается точным попарным расчётом.",
            Algorithm::FixedOrbits => "Кеплеровские орбиты вокруг самого массивного тела, O(N).",
            Algorithm::GridCells => {
                "Равномерная сетка: ближние ячейки точно, дальние — точечной массой."
            }
            Algorithm::FastMultipole => {
                "Октодерево + дерево взаимодействий: моменты и списки листьев \
                 переиспользуются между стадиями RK4. Выигрывает примерно с 1500 тел."
            }
        }
    }

    pub const ALL: [Algorithm; 5] = [
        Algorithm::BruteForce,
        Algorithm::BarnesHut,
        Algorithm::FixedOrbits,
        Algorithm::GridCells,
        Algorithm::FastMultipole,
    ];
}

/// Численный интегратор. Симплектический Эйлер — исходный (быстрый,
/// устойчивый на орбитах). RK4 — «полиномиальный» режим 4-го порядка:
/// точнее на тот же шаг, но 4 расчёта сил на шаг.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Integrator {
    #[default]
    SymplecticEuler,
    PolynomialRK4,
}

impl Integrator {
    pub fn name(self) -> &'static str {
        match self {
            Integrator::SymplecticEuler => "Симплектический Эйлер",
            Integrator::PolynomialRK4 => "Полиномиальный RK4",
        }
    }

    pub const ALL: [Integrator; 2] = [Integrator::SymplecticEuler, Integrator::PolynomialRK4];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimParams {
    pub g: f64,
    /// Шаг интегрирования в секундах симуляции.
    pub dt: f64,
    /// Множитель скорости: сколько сим-секунд проходит за секунду реального времени.
    pub speed: f64,
    /// Плавающее ограничение (softening), прибавляется к r² — гасит взрывы на близких сближениях.
    pub softening: f64,
    pub algorithm: Algorithm,
    pub integrator: Integrator,
    /// Угол раскрытия мультиполя: меньше — точнее и дороже.
    pub fmm_theta: f64,
    /// Порядок мультипольного разложения: 0 — монополь, 1 — диполь, 2 — квадруполь.
    pub fmm_order: u8,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            g: 1.0,
            dt: 0.016,
            speed: 1.0,
            softening: 4.0,
            algorithm: Algorithm::BruteForce,
            integrator: Integrator::SymplecticEuler,
            fmm_theta: 0.55,
            fmm_order: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    // ── Хореографии ────────────────────────────────────────────
    FigureEight,
    FigureEightPlanet,
    // ── Планетные системы ──────────────────────────────────────
    SolarSystem,
    StableSystem,
    HillCompact,
    HillGrand,
    KeplerLike,
    TrappistLike,
    PulsarPlanets,
    GasGiants,
    // ── Спутники и кольца ──────────────────────────────────────
    EarthMoon,
    MoonSystem,
    StableRing,
    // ── Кратные системы ────────────────────────────────────────
    BinaryStar,
    BinaryUnequal,
    HierarchicalTriple,
    HierarchicalQuad,
    CircumbinaryPlanets,
    StypePlanet,
    PlutoCharon,
    // ── Лагранж ─────────────────────────────────────────────────
    JupiterTrojans,
    // ── Облака ──────────────────────────────────────────────────
    RandomCloud,
}

impl Scenario {
    pub const ALL: [Scenario; 22] = [
        Scenario::FigureEight,
        Scenario::FigureEightPlanet,
        Scenario::SolarSystem,
        Scenario::StableSystem,
        Scenario::HillCompact,
        Scenario::HillGrand,
        Scenario::KeplerLike,
        Scenario::TrappistLike,
        Scenario::PulsarPlanets,
        Scenario::GasGiants,
        Scenario::EarthMoon,
        Scenario::MoonSystem,
        Scenario::StableRing,
        Scenario::BinaryStar,
        Scenario::BinaryUnequal,
        Scenario::HierarchicalTriple,
        Scenario::HierarchicalQuad,
        Scenario::CircumbinaryPlanets,
        Scenario::StypePlanet,
        Scenario::PlutoCharon,
        Scenario::JupiterTrojans,
        Scenario::RandomCloud,
    ];

    /// Все пресеты выбранной категории.
    pub fn in_category(category: super::presets::PresetCategory) -> impl Iterator<Item = Scenario> {
        Scenario::ALL
            .into_iter()
            .filter(move |s| s.info().category == category)
    }
}

pub struct Simulation {
    bodies: Vec<Body>,
    next_id: u64,
    params: SimParams,
    tick: u64,
    sim_time: f64,
    /// Экспоненциальное среднее длительности шага, мс.
    step_ms: f32,
    /// Листья октодерева последнего мультипольного шага (для отрисовки областей).
    last_regions: Vec<super::multipole::Region>,
    /// Переиспользуемый FMM-решатель: дерево и списки взаимодействий живут
    /// между шагами, поэтому после прогрева нет аллокаций на узел.
    multipole: super::multipole::MultipoleSolver,
    /// Профилировщик этапов шага (дерево, списки, силы, интегрирование).
    profiler: super::profiler::Profiler,
}

/// Способ расчёта ускорений в пределах шага.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForceMode {
    Brute,
    Grid,
    Multipole,
}

impl Simulation {
    pub fn new(params: SimParams, scenario: Scenario) -> Self {
        let mut sim = Self {
            bodies: Vec::new(),
            next_id: 1,
            params,
            tick: 0,
            sim_time: 0.0,
            step_ms: 0.0,
            last_regions: Vec::new(),
            multipole: super::multipole::MultipoleSolver::default(),
            profiler: super::profiler::Profiler::new(),
        };
        sim.reset(scenario);
        sim
    }

    pub fn reset(&mut self, scenario: Scenario) {
        self.bodies = super::presets::generate(scenario, self.params.g);
        self.next_id = self.bodies.iter().map(|b| b.id).max().unwrap_or(0) + 1;
        self.tick = 0;
        self.sim_time = 0.0;
        self.step_ms = 0.0;
        self.last_regions.clear();
    }

    pub fn params(&self) -> SimParams {
        self.params
    }

    pub fn set_params(&mut self, params: SimParams) {
        self.params = params;
    }

    pub fn bodies(&self) -> &[Body] {
        &self.bodies
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn sim_time(&self) -> f64 {
        self.sim_time
    }

    pub fn step_ms(&self) -> f32 {
        self.step_ms
    }

    /// Листья последнего мультипольного разбиения (пусто для других алгоритмов).
    pub fn regions(&self) -> &[super::multipole::Region] {
        &self.last_regions
    }

    /// Статистика этапов профилировщика последнего шага.
    pub fn profiler_stages(&self) -> &[StageStat] {
        self.profiler.stages()
    }

    /// Индекс самого массивного тела.
    pub fn heaviest(&self) -> Option<usize> {
        self.bodies
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.mass.total_cmp(&b.mass))
            .map(|(i, _)| i)
    }

    pub fn remove_body(&mut self, id: u64) {
        self.bodies.retain(|b| b.id != id);
    }

    /// Удалить все тела (пустой холст, параметры сохраняются).
    pub fn clear(&mut self) {
        self.bodies.clear();
        self.tick = 0;
        self.sim_time = 0.0;
        self.step_ms = 0.0;
    }

    pub fn set_body_mass(&mut self, id: u64, mass: f64) {
        if let Some(b) = self.bodies.iter_mut().find(|b| b.id == id) {
            b.mass = mass;
        }
    }

    /// Добавить тело с круговой орбитальной скоростью вокруг самого массивного
    /// (в плоскости xy; z сохраняется, чтобы клик в 3D-режиме на плоскости работал).
    pub fn add_body_orbiting(&mut self, mass: f64, radius: f64, color: [u8; 3], pos: Vec3) {
        let vel = self
            .heaviest()
            .map(|i| {
                let star = &self.bodies[i];
                let rel_xy = (pos - star.pos).xy();
                let r = rel_xy.len().max(1e-9);
                let v = (self.params.g * star.mass / r).sqrt();
                let v3 = rel_xy.perp().to3() * (v / r) + star.vel;
                v3
            })
            .unwrap_or_default();
        self.bodies.push(Body {
            id: self.next_id,
            mass,
            pos,
            vel,
            radius,
            color,
        });
        self.next_id += 1;
    }

    /// Добавить тело с явно заданной скоростью (вставка в точку Лагранжа,
    /// быстрая стабильная орбита, ручное состояние).
    pub fn add_body_state(&mut self, mass: f64, radius: f64, color: [u8; 3], pos: Vec3, vel: Vec3) {
        self.bodies.push(Body {
            id: self.next_id,
            mass,
            pos,
            vel,
            radius,
            color,
        });
        self.next_id += 1;
    }

    /// Обратить время: гравитация симметрична по времени, поэтому разворот
    /// всех скоростей заставляет систему retraces свою траекторию
    /// (точно для обратимых интеграторов, с малым дрейфом для Эйлера).
    pub fn reverse_velocities(&mut self) {
        for b in &mut self.bodies {
            b.vel = -b.vel;
        }
    }

    /// Полная замена состояния (используется повтором лога:
    /// «продолжить симуляцию с отметки»).
    pub fn replace_bodies(&mut self, bodies: Vec<Body>) {
        self.bodies = bodies;
        self.next_id = self.bodies.iter().map(|b| b.id).max().unwrap_or(0) + 1;
    }

    /// Один шаг интегрирования текущим алгоритмом.
    pub fn step(&mut self) {
        self.profiler.measure("шаг физики", || {
            let started = Instant::now();
            match self.params.algorithm {
                Algorithm::BruteForce | Algorithm::BarnesHut => {
                    self.integrate(ForceMode::Brute);
                }
                Algorithm::GridCells => self.integrate(ForceMode::Grid),
                Algorithm::FastMultipole => self.integrate(ForceMode::Multipole),
                Algorithm::FixedOrbits => {
                    self.last_regions.clear();
                    self.step_fixed_orbits();
                }
            }
            let ms = started.elapsed().as_secs_f32() * 1000.0;
            self.step_ms = if self.step_ms == 0.0 {
                ms
            } else {
                self.step_ms * 0.9 + ms * 0.1
            };
            self.sim_time += self.params.dt;
            self.tick += 1;
        });
    }

    /// Ускорения в заданных позициях выбранным способом.
    fn accel_all(
        &mut self,
        pos: &[Vec3],
        mode: ForceMode,
        regions: &mut Vec<super::multipole::Region>,
    ) -> Vec<Vec3> {
        match mode {
            ForceMode::Brute => self.accel_brute_at(pos),
            ForceMode::Grid => {
                super::grid::accel_grid(&self.bodies, pos, self.params.g, self.params.softening)
            }
            ForceMode::Multipole => {
                let result = self.multipole.accel(
                    &self.bodies,
                    pos,
                    self.params.g,
                    self.params.softening,
                    self.params.fmm_theta,
                    self.params.fmm_order,
                );
                *regions = result.regions;
                result.acc
            }
        }
    }

    /// Шаг с текущим интегратором.
    fn integrate(&mut self, mode: ForceMode) {
        let positions: Vec<Vec3> = self.bodies.iter().map(|b| b.pos).collect();
        let mut regions = Vec::new();
        let acc = self.profiler.measure("ускорения", || {
            self.accel_all(&positions, mode, &mut regions)
        });
        self.profiler.measure("интегрирование", || {
            match self.params.integrator {
                Integrator::SymplecticEuler => {
                    let dt = self.params.dt;
                    for (b, a) in self.bodies.iter_mut().zip(acc) {
                        b.vel += a * dt;
                        b.pos += b.vel * dt;
                    }
                }
                Integrator::PolynomialRK4 => {
                    // Классический RK4 для системы y = (pos, vel), y' = (vel, a(pos)).
                    let dt = self.params.dt;
                    let mut pos = positions;
                    let mut vel: Vec<Vec3> = self.bodies.iter().map(|b| b.vel).collect();

                    let a1 = acc;
                    let mut p2 = pos.clone();
                    for i in 0..pos.len() {
                        p2[i] = pos[i] + vel[i] * (dt * 0.5);
                    }
                    let a2 = self.accel_all(&p2, mode, &mut regions);
                    let v2: Vec<Vec3> = (0..pos.len())
                        .map(|i| vel[i] + a1[i] * (dt * 0.5))
                        .collect();
                    let mut p3 = pos.clone();
                    for i in 0..pos.len() {
                        p3[i] = pos[i] + v2[i] * (dt * 0.5);
                    }
                    let a3 = self.accel_all(&p3, mode, &mut regions);
                    let v3: Vec<Vec3> = (0..pos.len())
                        .map(|i| vel[i] + a2[i] * (dt * 0.5))
                        .collect();
                    let mut p4 = pos.clone();
                    for i in 0..pos.len() {
                        p4[i] = pos[i] + v3[i] * dt;
                    }
                    let a4 = self.accel_all(&p4, mode, &mut regions);
                    let v4: Vec<Vec3> = (0..pos.len()).map(|i| vel[i] + a3[i] * dt).collect();

                    for i in 0..pos.len() {
                        let sixth = dt / 6.0;
                        pos[i] += (vel[i] + v2[i] * 2.0 + v3[i] * 2.0 + v4[i]) * sixth;
                        vel[i] += (a1[i] + a2[i] * 2.0 + a3[i] * 2.0 + a4[i]) * sixth;
                    }
                    for (b, i) in self.bodies.iter_mut().zip(0..pos.len()) {
                        b.pos = pos[i];
                        b.vel = vel[i];
                    }
                }
            }
        });
        if mode == ForceMode::Multipole {
            self.last_regions = regions;
        } else {
            self.last_regions.clear();
        }
    }

    /// Ускорения всех тел в произвольных позициях, точный O(N²) расчёт.
    fn accel_brute_at(&self, pos: &[Vec3]) -> Vec<Vec3> {
        let n = self.bodies.len();
        let mut acc = vec![Vec3::ZERO; n];
        for i in 0..n {
            let pi = pos[i];
            let mut a = Vec3::ZERO;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let bj = &self.bodies[j];
                let d = pos[j] - pi;
                let r2 = d.len2() + self.params.softening;
                // d / (r² + eps)^{3/2} * G * m_j
                a += d * (self.params.g * bj.mass / (r2 * r2.sqrt()));
            }
            acc[i] = a;
        }
        acc
    }

    /// Кеплеровские круговые орбиты вокруг самой массивной звезды, O(N).
    /// Работает в 3D: плоскость орбиты каждого тела задаётся его моментом
    /// импульса `rel × vel`, поворот — вокруг этой нормали (Родриг).
    fn step_fixed_orbits(&mut self) {
        let Some(heaviest) = self.heaviest() else {
            return;
        };
        let (center, m_central) = {
            let c = &self.bodies[heaviest];
            (c.pos, c.mass)
        };
        let g = self.params.g;
        let dt = self.params.dt;
        for b in &mut self.bodies {
            let rel = b.pos - center;
            let r = rel.len();
            if r < 1e-9 {
                continue;
            }
            let omega = (g * m_central / (r * r * r)).sqrt();
            let ang = omega * dt;
            let (s, c) = ang.sin_cos();
            // Нормаль орбитальной плоскости; при вырожденном моменте — z.
            let mut normal = rel.cross(b.vel).normalized();
            if normal.len() < 1e-9 {
                normal = Vec3::new(0.0, 0.0, 1.0);
            }
            // rel ⊥ normal, поэтому в формуле Родрига остаются два слагаемых.
            let rotated = rel * c + normal.cross(rel) * s;
            b.pos = center + rotated;
            // Касательная скорость: n × r, по модулю ω·r.
            b.vel = normal.cross(rotated) * omega;
        }
    }
}

/// Круговая орбитальная скорость вокруг массы `m` на расстоянии `r`.
pub fn orbital_speed(g: f64, m: f64, r: f64) -> f64 {
    (g * m / r).sqrt()
}

/// Готовая орбита для быстрой вставки тела: положение, скорость, радиус
/// и нормаль плоскости (для превью).
#[derive(Clone, Copy, Debug)]
pub struct OrbitPlan {
    pub pos: Vec3,
    pub vel: Vec3,
    pub radius: f64,
    pub normal: Vec3,
}

/// Подобрать устойчивую круговую орбиту вокруг `host` через точку `click`.
///
/// Радиус ограничен сферой Хилла хозяина (`0.05…0.45 R_H`) относительно
/// самого массивного соседа — это тот же запас, что используют генераторы
/// пресетов. Скорость считается по сумме масс `m_host + m_new` и добавляется
/// к скорости хозяина, поэтому тело сразу оказывается на орбите, а не падает
/// на него. Если хозяин — самое массивное тело системы, ограничения по Хиллу
/// нет и радиус берётся из клика.
pub fn orbit_plan(
    bodies: &[Body],
    host: usize,
    click: Vec3,
    mass: f64,
    g: f64,
) -> Option<OrbitPlan> {
    let h = bodies.get(host)?;
    if !(h.mass > 0.0) || !h.pos.is_finite() || !(g > 0.0) {
        return None;
    }

    // Сосед, относительно которого считается сфера Хилла, и плоскость
    // орбиты хозяина (нормаль его момента импульса вокруг соседа).
    let companion = bodies
        .iter()
        .enumerate()
        .filter(|(i, b)| *i != host && b.mass > 0.0)
        .max_by(|a, b| a.1.mass.total_cmp(&b.1.mass));
    let mut normal = Vec3::new(0.0, 0.0, 1.0);
    let mut hill = f64::INFINITY;
    if let Some((_, c)) = companion {
        let rel = h.pos - c.pos;
        let vrel = h.vel - c.vel;
        let n = rel.cross(vrel).normalized();
        if n.len() > 0.5 {
            normal = n;
        }
        let sep = rel.len();
        if sep > 1e-9 && c.mass > 0.0 {
            hill = sep * (h.mass / (3.0 * c.mass)).cbrt();
        }
    }
    if !hill.is_finite() || hill <= 0.0 {
        hill = f64::INFINITY;
    }

    // Клик — направление орбиты; вырожденный случай — произвольный базис.
    let mut offset = click - h.pos;
    offset = offset - normal * offset.dot(normal);
    let mut dir = offset.normalized();
    if dir.len() < 0.5 {
        let helper = if normal.z.abs() < 0.9 {
            Vec3::new(0.0, 0.0, 1.0)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        dir = helper.cross(normal).normalized();
    }

    let r_click = offset.len();
    let r_min = (h.radius * 2.5).max(3.0);
    let r_max = if hill.is_finite() {
        hill * 0.45
    } else {
        f64::INFINITY
    };
    let r_min = if r_max.is_finite() {
        r_min.min(r_max * 0.8)
    } else {
        r_min
    };
    let radius = r_click.max(r_min).min(r_max).max(1e-6);

    // Прямое движение: нормаль орбиты хозяина × радиус-вектор.
    let tangent = normal.cross(dir).normalized();
    let v = orbital_speed(g, h.mass + mass.max(0.0), radius);
    Some(OrbitPlan {
        pos: h.pos + dir * radius,
        vel: h.vel + tangent * v,
        radius,
        normal,
    })
}

/// Предсказание траектории выбранного тела: состояние копируется и
/// интегрируется вперёд симплектическим Эйлером с укрупнённым шагом
/// («траектория-призрак»; возмущениями от будущего приближённого расчёта
/// пренебрегаем — предсказание переоценивается по мере хаоса).
pub fn predict_path(
    bodies: &[Body],
    params: SimParams,
    focus: usize,
    steps: usize,
    dt: f64,
) -> Vec<Vec3> {
    if bodies.is_empty() {
        return Vec::new();
    }
    let mut p = params;
    p.dt = dt;
    // Для небольших систем сохраняем тот же интегратор, что и в основной
    // симуляции; для больших — дешёвый Эйлер (предсказание всё равно «призрак»).
    p.integrator = if bodies.len() <= 64 {
        params.integrator
    } else {
        Integrator::SymplecticEuler
    };
    p.algorithm = if bodies.len() > 150 {
        Algorithm::GridCells
    } else {
        Algorithm::BruteForce
    };
    let mut sim = Simulation {
        bodies: bodies.to_vec(),
        next_id: bodies.iter().map(|b| b.id).max().unwrap_or(0) + 1,
        params: p,
        tick: 0,
        sim_time: 0.0,
        step_ms: 0.0,
        last_regions: Vec::new(),
        multipole: super::multipole::MultipoleSolver::default(),
        profiler: super::profiler::Profiler::new(),
    };
    let mut path = Vec::with_capacity(steps);
    for _ in 0..steps {
        sim.step();
        if let Some(b) = sim.bodies.get(focus) {
            path.push(b.pos);
        }
    }
    path
}
