//! Чистое ядро симуляции: тела, параметры, шаг интегрирования и сценарии.
//! Ничего не знает ни про egui, ни про потоки — ими обёртки выше.

use std::time::Instant;

use super::vec::{Rng, Vec3};

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
}

impl Algorithm {
    pub fn name(self) -> &'static str {
        match self {
            Algorithm::BruteForce => "Brute Force O(N²)",
            Algorithm::BarnesHut => "Barnes-Hut O(N log N)",
            Algorithm::FixedOrbits => "Fixed Orbits (Euler)",
            Algorithm::GridCells => "Сетка областей ~O(N)",
        }
    }

    pub const ALL: [Algorithm; 4] = [
        Algorithm::BruteForce,
        Algorithm::BarnesHut,
        Algorithm::FixedOrbits,
        Algorithm::GridCells,
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Scenario {
    /// Звезда и планеты на почти круговых орбитах.
    SolarSystem,
    /// Две звезды в паре плюс планеты вокруг пары.
    BinaryStar,
    /// Диск из мелких тел (протопланетное облако).
    RandomCloud,
    /// Планеты разнесены по критерию взаимных радиусов Хилла.
    StableSystem,
}

impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Scenario::SolarSystem => "Солнечная система",
            Scenario::BinaryStar => "Двойная звезда",
            Scenario::RandomCloud => "Протопланетное облако",
            Scenario::StableSystem => "Стабильная система (Хилл)",
        }
    }

    pub const ALL: [Scenario; 4] = [
        Scenario::SolarSystem,
        Scenario::BinaryStar,
        Scenario::RandomCloud,
        Scenario::StableSystem,
    ];
}

pub struct Simulation {
    bodies: Vec<Body>,
    next_id: u64,
    params: SimParams,
    tick: u64,
    sim_time: f64,
    /// Экспоненциальное среднее длительности шага, мс.
    step_ms: f32,
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
        };
        sim.reset(scenario);
        sim
    }

    pub fn reset(&mut self, scenario: Scenario) {
        self.bodies = generate_scenario(scenario, self.params.g);
        self.next_id = self.bodies.iter().map(|b| b.id).max().unwrap_or(0) + 1;
        self.tick = 0;
        self.sim_time = 0.0;
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
        let started = Instant::now();
        match self.params.algorithm {
            Algorithm::BruteForce => self.integrate(false),
            // TODO(#barnes-hut): настоящее квадродерево. Пока используется точный
            // расчёт, чтобы заготовка вела себя честно на всех трёх режимах.
            Algorithm::BarnesHut => self.integrate(false),
            Algorithm::GridCells => self.integrate(true),
            Algorithm::FixedOrbits => self.step_fixed_orbits(),
        }
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        self.step_ms = if self.step_ms == 0.0 {
            ms
        } else {
            self.step_ms * 0.9 + ms * 0.1
        };
        self.sim_time += self.params.dt;
        self.tick += 1;
    }

    /// Шаг с текущим интегратором; `use_grid` — сеточное приближение сил.
    fn integrate(&mut self, use_grid: bool) {
        let positions: Vec<Vec3> = self.bodies.iter().map(|b| b.pos).collect();
        match self.params.integrator {
            Integrator::SymplecticEuler => {
                let acc = if use_grid {
                    super::grid::accel_grid(
                        &self.bodies,
                        &positions,
                        self.params.g,
                        self.params.softening,
                    )
                } else {
                    self.accel_brute_at(&positions)
                };
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

                let accel = |p: &[Vec3]| -> Vec<Vec3> {
                    if use_grid {
                        super::grid::accel_grid(&self.bodies, p, self.params.g, self.params.softening)
                    } else {
                        self.accel_brute_at(p)
                    }
                };

                let a1 = accel(&pos);
                let mut p2 = pos.clone();
                for i in 0..pos.len() {
                    p2[i] = pos[i] + vel[i] * (dt * 0.5);
                }
                let a2 = accel(&p2);
                let v2: Vec<Vec3> = (0..pos.len())
                    .map(|i| vel[i] + a1[i] * (dt * 0.5))
                    .collect();
                let mut p3 = pos.clone();
                for i in 0..pos.len() {
                    p3[i] = pos[i] + v2[i] * (dt * 0.5);
                }
                let a3 = accel(&p3);
                let v3: Vec<Vec3> = (0..pos.len())
                    .map(|i| vel[i] + a2[i] * (dt * 0.5))
                    .collect();
                let mut p4 = pos.clone();
                for i in 0..pos.len() {
                    p4[i] = pos[i] + v3[i] * dt;
                }
                let a4 = accel(&p4);
                let v4: Vec<Vec3> = (0..pos.len())
                    .map(|i| vel[i] + a3[i] * dt)
                    .collect();

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

/// Добавить тело с автоинкрементом id.
fn push(
    bodies: &mut Vec<Body>,
    next_id: &mut u64,
    mass: f64,
    pos: Vec3,
    vel: Vec3,
    radius: f64,
    color: [u8; 3],
) {
    bodies.push(Body {
        id: *next_id,
        mass,
        pos,
        vel,
        radius,
        color,
    });
    *next_id += 1;
}

/// Ортонормированный базис плоскости с нормалью `normal`.
fn orbit_basis(normal: Vec3) -> (Vec3, Vec3) {
    let n = normal.normalized();
    let n = if n.len() < 1e-9 { Vec3::new(0.0, 0.0, 1.0) } else { n };
    let helper = if n.z.abs() < 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = helper.cross(n).normalized();
    let v = n.cross(u).normalized();
    (u, v)
}

/// Нормаль орбитальной плоскости по наклонению `incl` и долготе узла `node`.
fn normal_from(incl: f64, node: f64) -> Vec3 {
    Vec3::new(
        incl.sin() * node.cos(),
        incl.sin() * node.sin(),
        incl.cos(),
    )
}

/// Состояние на кеплеровской орбите (старт из апоцентра, vis-viva).
/// `a` — большая полуось, `e` — эксцентриситет, `phase` — истинная аномалия.
fn orbit_state(g: f64, m: f64, a: f64, e: f64, normal: Vec3, phase: f64) -> (Vec3, Vec3) {
    let (u, v) = orbit_basis(normal);
    let a = a.max(1e-6);
    let e = e.clamp(0.0, 0.9);
    let r_apo = a * (1.0 + e);
    // v² = GM/a · (1−e)/(1+e) в апоцентре.
    let v_apo = (g * m * (1.0 - e) / (a * (1.0 + e))).sqrt();
    let (sp, cp) = phase.sin_cos();
    let pos = (u * cp + v * sp) * r_apo;
    let vel = (u * (-sp) + v * cp) * v_apo;
    (pos, vel)
}

/// Наклонить всю систему как единое целое (не меняет взаимную динамику).
fn tilt_bodies(bodies: &mut [Body], rx: f64, ry: f64) {
    for b in bodies {
        b.pos = b.pos.rotate_x(rx).rotate_y(ry);
        b.vel = b.vel.rotate_x(rx).rotate_y(ry);
    }
}

fn generate_scenario(scenario: Scenario, g: f64) -> Vec<Body> {
    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    let mut bodies = Vec::new();
    let mut next_id = 1u64;

    // Компенсация импульса: самый массивный объект получает противо-импульс,
    // чтобы центр масс не уплывал из кадра.
    fn balance_momentum(bodies: &mut Vec<Body>) {
        let Some(i) = bodies
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.mass.total_cmp(&b.mass))
            .map(|(i, _)| i)
        else {
            return;
        };
        let (m_total_star, p) = {
            let star = &bodies[i];
            let m = star.mass;
            let p: Vec3 = bodies
                .iter()
                .filter(|b| b.id != star.id)
                .map(|b| b.vel * b.mass)
                .fold(Vec3::ZERO, |s, v| s + v);
            (m, p)
        };
        bodies[i].vel = p * (-1.0 / m_total_star);
    }

    const PLANET_COLORS: [[u8; 3]; 8] = [
        [110, 170, 255], [80, 220, 160], [240, 120, 110],
        [230, 180, 90], [200, 140, 255], [120, 230, 230],
        [255, 150, 190], [190, 210, 230],
    ];

    match scenario {
        Scenario::SolarSystem => {
            let star_mass = 1000.0;
            push(&mut bodies, &mut next_id, star_mass, Vec3::ZERO, Vec3::ZERO, 14.0, [255, 200, 60]);
            for i in 0..10 {
                let r = 70.0 + i as f64 * 42.0 + rng.f64() * 18.0;
                let mass = 0.5 + rng.f64() * 8.0;
                // Небольшие взаимные наклонения (до ~4°) — сценарий перестаёт
                // быть плоским, но остаётся динамически почти копланарным.
                let incl = (rng.f64() - 0.5) * 0.14;
                let node = rng.f64() * std::f64::consts::TAU;
                let phase = rng.f64() * std::f64::consts::TAU;
                let e = rng.f64() * 0.02;
                let (pos, vel) = orbit_state(g, star_mass, r, e, normal_from(incl, node), phase);
                push(&mut bodies, &mut next_id, mass, pos, vel,
                     1.6 + mass * 0.22, PLANET_COLORS[i % PLANET_COLORS.len()]);
            }
            balance_momentum(&mut bodies);
            // Небольшой общий наклон плоскости — даже «плоская» солнечная
            // система выглядит объёмной относительно горизонтальной сетки.
            tilt_bodies(&mut bodies, 0.30, 0.12);
        }
        Scenario::BinaryStar => {
            let m = 800.0;
            let d = 140.0;
            // Плоскость пары наклонена — сценарий сразу читается как 3D.
            let normal = normal_from(0.55, 0.9);
            let (u, tangent) = orbit_basis(normal);
            // Круговая орбита двух равных масс вокруг общего центра:
            // v = sqrt(G·m / (2d)).
            let v = (g * m / (2.0 * d)).sqrt();
            push(&mut bodies, &mut next_id, m, u * (-d / 2.0), tangent * v, 11.0, [255, 170, 70]);
            push(&mut bodies, &mut next_id, m, u * (d / 2.0), tangent * (-v), 10.0, [150, 200, 255]);
            for i in 0..8 {
                let r = 220.0 + i as f64 * 55.0 + rng.f64() * 20.0;
                let incl = 0.55 + (rng.f64() - 0.5) * 0.22;
                let node = 0.9 + (rng.f64() - 0.5) * 0.4;
                let phase = rng.f64() * std::f64::consts::TAU;
                let e = rng.f64() * 0.02;
                let (pos, vel) = orbit_state(g, 2.0 * m, r, e, normal_from(incl, node), phase);
                push(&mut bodies, &mut next_id, 0.5 + rng.f64() * 4.0, pos, vel,
                     1.6 + rng.f64() * 1.4, [200, 220, 240]);
            }
            balance_momentum(&mut bodies);
        }
        Scenario::RandomCloud => {
            let n = 200;
            let disk_r = 320.0;
            for _ in 0..n {
                // Толстый диск: радиус в плоскости + заметный разброс по z,
                // чтобы облако было объёмным, а не плоским.
                let r = (30.0f64 * 30.0 + (disk_r * disk_r - 30.0 * 30.0) * rng.f64()).sqrt();
                let a = rng.f64() * std::f64::consts::TAU;
                let z = (rng.f64() - 0.5) * r * 0.7;
                let pos = Vec3::new(r * a.cos(), r * a.sin(), z);
                let mass = 0.1 + rng.f64() * 2.0;
                // Скорость задаётся ниже по круговой формуле.
                push(&mut bodies, &mut next_id, mass, pos, Vec3::ZERO,
                     0.9 + mass * 0.3,
                     [
                         120 + (rng.f64() * 100.0) as u8,
                         160 + (rng.f64() * 80.0) as u8,
                         255,
                     ]);
            }
            // Центральная протозвезда, чтобы облако стягивалось в систему.
            push(&mut bodies, &mut next_id, 900.0, Vec3::ZERO, Vec3::ZERO, 12.0, [255, 210, 90]);
            // Круговые скорости относительно массы внутри орбиты.
            // Снимок позиций/масс, чтобы не заимствовать bodies дважды.
            let layout: Vec<(Vec3, f64)> = bodies.iter().map(|b| (b.pos, b.mass)).collect();
            for b in &mut bodies {
                if b.mass > 100.0 {
                    continue;
                }
                let rho = b.pos.xy().len().max(1.0);
                let m_in: f64 = layout
                    .iter()
                    .filter(|(p, _)| p.xy().len() < rho)
                    .map(|(_, m)| m)
                    .sum();
                let v = orbital_speed(g, m_in, rho);
                let tangent = b.pos.xy().perp() * (v / rho);
                b.vel = tangent.to3();
                // Небольшая вертикальная составляющая — облако «дышит» в 3D.
                b.vel.z = (rng.f64() - 0.5) * v * 0.12;
            }
            balance_momentum(&mut bodies);
        }
        Scenario::StableSystem => {
            // Генератор долгосрочно стабильных систем по критерию взаимных
            // радиусов Хилла (Gladman 1993; Chambers, Wetherill & Boss 1996):
            // разнесение соседних орбит Δ > 2√3 ≈ 3.46 взаимных радиусов
            // Хилла — граница хилл-устойчивости для круговых копланарных
            // орбит; берём запас K = 5 и крошечные эксцентриситеты.
            const HILL_K: f64 = 5.0;
            let star_mass = 2000.0;
            push(&mut bodies, &mut next_id, star_mass, Vec3::ZERO, Vec3::ZERO, 14.0, [255, 210, 80]);

            let mut a = 70.0; // первая большая полуось
            let mut prev_a = 0.0;
            let mut prev_mass = 0.0;
            for i in 0..8 {
                let mass = 0.05 + rng.f64() * 0.45; // ≤ 2.5·10⁻⁴ M★ — возмущения малы
                if i > 0 {
                    // Взаимный радиус Хилла соседней пары: r_H = ((m1+m2)/3M)^⅓·(a1+a2)/2
                    let r_h = ((mass + prev_mass) / (3.0 * star_mass)).powf(1.0 / 3.0)
                        * (a + prev_a)
                        / 2.0;
                    let next_a = prev_a + HILL_K * r_h;
                    prev_a = a;
                    a = next_a;
                } else {
                    prev_a = a;
                }
                // Малый эксцентриситет; старт из апоцентра (vis-viva):
                // v² = GM/a · (1−e)/(1+e) при r = a(1+e).
                let e = rng.f64() * 0.02;
                let phase = rng.f64() * std::f64::consts::TAU;
                let (pos, vel) = orbit_state(
                    g,
                    star_mass,
                    a,
                    e,
                    Vec3::new(0.0, 0.0, 1.0),
                    phase,
                );
                push(&mut bodies, &mut next_id, mass, pos, vel,
                     1.5 + mass * 0.8,
                     PLANET_COLORS[(i + 3) % PLANET_COLORS.len()]);
                prev_mass = mass;
            }
            balance_momentum(&mut bodies);
            // Наклоняем всю плоскость как единое целое: взаимная динамика
            // (и хилл-устойчивость) не меняется, но 3D-камера видит объём.
            tilt_bodies(&mut bodies, 0.42, 0.22);
        }
    }
    bodies
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
