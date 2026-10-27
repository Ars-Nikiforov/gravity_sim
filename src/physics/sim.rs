//! Чистое ядро симуляции: тела, параметры, шаг интегрирования и сценарии.
//! Ничего не знает ни про egui, ни про потоки — ими обёртки выше.

use std::time::Instant;

use super::vec::{Rng, Vec2};

#[derive(Clone, Debug)]
pub struct Body {
    pub id: u64,
    pub mass: f64,
    pub pos: Vec2,
    pub vel: Vec2,
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
}

impl Algorithm {
    pub fn name(self) -> &'static str {
        match self {
            Algorithm::BruteForce => "Brute Force O(N²)",
            Algorithm::BarnesHut => "Barnes-Hut O(N log N)",
            Algorithm::FixedOrbits => "Fixed Orbits (Euler)",
        }
    }

    pub const ALL: [Algorithm; 3] = [
        Algorithm::BruteForce,
        Algorithm::BarnesHut,
        Algorithm::FixedOrbits,
    ];
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
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            g: 1.0,
            dt: 0.016,
            speed: 1.0,
            softening: 4.0,
            algorithm: Algorithm::BruteForce,
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
}

impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Scenario::SolarSystem => "Солнечная система",
            Scenario::BinaryStar => "Двойная звезда",
            Scenario::RandomCloud => "Протопланетное облако",
        }
    }

    pub const ALL: [Scenario; 3] = [
        Scenario::SolarSystem,
        Scenario::BinaryStar,
        Scenario::RandomCloud,
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
    /// Черновой буфер ускорений, чтобы не аллоцировать на каждом шаге.
    acc: Vec<Vec2>,
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
            acc: Vec::new(),
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

    /// Добавить тело с круговой орбитальной скоростью вокруг самого массивного.
    pub fn add_body_orbiting(&mut self, mass: f64, radius: f64, color: [u8; 3], pos: Vec2) {
        let vel = self
            .heaviest()
            .map(|i| {
                let star = &self.bodies[i];
                let rel = pos - star.pos;
                let r = rel.len().max(1e-9);
                let v = (self.params.g * star.mass / r).sqrt();
                rel.perp() * (v / r) + star.vel
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

    /// Один шаг интегрирования текущим алгоритмом.
    pub fn step(&mut self) {
        let started = Instant::now();
        match self.params.algorithm {
            Algorithm::BruteForce => self.step_brute_force(),
            // TODO(#barnes-hut): настоящее квадродерево. Пока используется точный
            // расчёт, чтобы заготовка вела себя честно на всех трёх режимах.
            Algorithm::BarnesHut => self.step_brute_force(),
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

    /// O(N²), полуявный метод Эйлера (симплектический — орбиты не разваливаются).
    fn step_brute_force(&mut self) {
        let n = self.bodies.len();
        self.acc.clear();
        self.acc.resize(n, Vec2::ZERO);

        for i in 0..n {
            let pi = self.bodies[i].pos;
            let mut a = Vec2::ZERO;
            for j in 0..n {
                if i == j {
                    continue;
                }
                let bj = &self.bodies[j];
                let d = bj.pos - pi;
                let r2 = d.len2() + self.params.softening;
                // d / (r² + eps)^{3/2} * G * m_j
                a += d * (self.params.g * bj.mass / (r2 * r2.sqrt()));
            }
            self.acc[i] = a;
        }

        let dt = self.params.dt;
        for (b, a) in self.bodies.iter_mut().zip(&self.acc) {
            b.vel += *a * dt;
            b.pos += b.vel * dt;
        }
    }

    /// Кеплеровские круговые орбиты вокруг самой массивной звезды, O(N).
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
            // ω = sqrt(G·M / r³); поворачиваем радиус-вектор на ω·dt.
            let omega = (g * m_central / (rel.len2() * r)).sqrt();
            let ang = omega * dt;
            let (s, c) = ang.sin_cos();
            b.pos = center + Vec2::new(rel.x * c - rel.y * s, rel.x * s + rel.y * c);
            // Синхронизируем скорость, чтобы переключение алгоритма не давало рывка.
            b.vel = (b.pos - center).perp() * (omega / (b.pos - center).len().max(1e-9));
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
    pos: Vec2,
    vel: Vec2,
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
            let p: Vec2 = bodies
                .iter()
                .filter(|b| b.id != star.id)
                .map(|b| b.vel * b.mass)
                .fold(Vec2::ZERO, |s, v| s + v);
            (m, p)
        };
        bodies[i].vel = p * (-1.0 / m_total_star);
    }

    match scenario {
        Scenario::SolarSystem => {
            let star_mass = 1000.0;
            push(&mut bodies, &mut next_id, star_mass, Vec2::ZERO, Vec2::ZERO, 14.0, [255, 200, 60]);
            const PLANET_COLORS: [[u8; 3]; 8] = [
                [110, 170, 255], [80, 220, 160], [240, 120, 110],
                [230, 180, 90], [200, 140, 255], [120, 230, 230],
                [255, 150, 190], [190, 210, 230],
            ];
            for i in 0..10 {
                let r = 70.0 + i as f64 * 42.0 + rng.f64() * 18.0;
                let pos = rng.ring(r, r);
                let mass = 0.5 + rng.f64() * 8.0;
                let v = orbital_speed(g, star_mass, r);
                let tangent = pos.perp() * (1.0 / r);
                push(&mut bodies, &mut next_id, mass, pos, tangent * v,
                     1.6 + mass * 0.22, PLANET_COLORS[i % PLANET_COLORS.len()]);
            }
            balance_momentum(&mut bodies);
        }
        Scenario::BinaryStar => {
            let m = 800.0;
            let d = 140.0;
            // Круговая орбита двух равных масс вокруг общего центра:
            // v = sqrt(G·m / (2d)).
            let v = (g * m / (2.0 * d)).sqrt();
            push(&mut bodies, &mut next_id, m, Vec2::new(-d / 2.0, 0.0), Vec2::new(0.0, v), 11.0, [255, 170, 70]);
            push(&mut bodies, &mut next_id, m, Vec2::new(d / 2.0, 0.0), Vec2::new(0.0, -v), 10.0, [150, 200, 255]);
            for i in 0..8 {
                let r = 220.0 + i as f64 * 55.0 + rng.f64() * 20.0;
                let pos = rng.ring(r, r);
                let vc = orbital_speed(g, 2.0 * m, r);
                let tangent = pos.perp() * (1.0 / r);
                push(&mut bodies, &mut next_id, 0.5 + rng.f64() * 4.0, pos, tangent * vc,
                     1.6 + rng.f64() * 1.4, [200, 220, 240]);
            }
            balance_momentum(&mut bodies);
        }
        Scenario::RandomCloud => {
            let n = 200;
            let disk_r = 320.0;
            for _ in 0..n {
                let pos = rng.ring(30.0, disk_r);
                let mass = 0.1 + rng.f64() * 2.0;
                // Скорость задаётся ниже по круговой формуле.
                push(&mut bodies, &mut next_id, mass, pos, Vec2::ZERO,
                     0.9 + mass * 0.3,
                     [
                         120 + (rng.f64() * 100.0) as u8,
                         160 + (rng.f64() * 80.0) as u8,
                         255,
                     ]);
            }
            // Центральная протозвезда, чтобы облако стягивалось в систему.
            push(&mut bodies, &mut next_id, 900.0, Vec2::ZERO, Vec2::ZERO, 12.0, [255, 210, 90]);
            // Круговые скорости относительно массы внутри орбиты.
            // Снимок позиций/масс, чтобы не заимствовать bodies дважды.
            let layout: Vec<(Vec2, f64)> = bodies.iter().map(|b| (b.pos, b.mass)).collect();
            for b in &mut bodies {
                if b.mass > 100.0 {
                    continue;
                }
                let r = b.pos.len().max(1.0);
                let m_in: f64 = layout
                    .iter()
                    .filter(|(p, _)| p.len() < r)
                    .map(|(_, m)| m)
                    .sum();
                let v = orbital_speed(g, m_in, r);
                b.vel = b.pos.perp() * (v / r);
            }
            balance_momentum(&mut bodies);
        }
    }
    bodies
}
