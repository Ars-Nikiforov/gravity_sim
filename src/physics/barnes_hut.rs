//! Метод Barnes-Hut: октодерево и монопольное приближение O(N log N).
//!
//! Идея в трёх шагах:
//! 1. **Дерево.** То же плоское октодерево, что в мультипольном методе
//!    (`multipole.rs`): тела в одном массиве `order`, листья — непрерывные
//!    отрезки, разбиение counting sort. Для BH хватает монополя: у узла
//!    хранятся суммарная масса и центр масс.
//! 2. **Дерево взаимодействий.** Для каждого листа один раз строится список
//!    далёких узлов и список ближних тел — как в FMM. Критерий раскрытия —
//!    классический Barnes-Hut `s < θ·d`: сторона узла `s = 2·half` меньше
//!    θ·(расстояние до цели). Цели листа группируются, поэтому критерий
//!    считается консервативно — от центра листа с запасом на его описанную
//!    сферу: если `s ≤ θ·(|com − c_листа| − r_листа)`, то условие верно для
//!    *каждой* цели листа.
//! 3. **Сумма.** Далёкий узел — монополь от его центра масс (со сглаживанием,
//!     чтобы поле было непрерывным на границе ближней/дальней зоны), ближние
//!     тела — точный попарный расчёт.
//!
//! Дерево и списки переиспользуются между стадиями RK4: внутри шага источники
//! неизменны, кэш сверяет позиции и массы. Обход по листьям даёт ту же
//! экономию, что и в FMM: вклад далёкого узла загружается один раз на лист.
//!
//! Все операции детерминированы (порядок тел и октантов фиксирован), поэтому
//! одинаковая конфигурация даёт бит-в-бит одинаковый результат.

use super::multipole::{MAX_LEAF, MAX_DEPTH, NO_NODE, Region, SQRT3, bounds, octant};
use super::profiler::{self, Profiler};
use super::sim::Body;
use super::vec::Vec3;

/// Результат расчёта Barnes-Hut.
pub struct BarnesHutResult {
    pub acc: Vec<Vec3>,
    pub regions: Vec<Region>,
}

/// Узел плоского октодерева (только монополь).
#[derive(Clone, Copy, Debug)]
struct Node {
    center: Vec3,
    half: f64,
    /// Суммарная масса (уже умножена на `g`).
    mass: f64,
    /// Центр масс.
    com: Vec3,
    children: [u32; 8],
    /// Отрезок `body_start..body_start + body_len` в массиве `order`;
    /// у внутренних узлов `body_len == 0`.
    body_start: u32,
    body_len: u32,
}

impl Node {
    #[inline]
    fn is_leaf(&self) -> bool {
        self.body_len > 0
    }

    /// Монопольное ускорение в точке `p` (со сглаживанием).
    #[inline]
    fn accel(&self, p: Vec3, softening: f64) -> Vec3 {
        let d = p - self.com;
        let r2 = d.len2() + softening;
        d * (self.mass / (r2 * r2.sqrt()))
    }
}

/// Переиспользуемый решатель: дерево, списки взаимодействий и буферы живут
/// между шагами, поэтому после прогрева аллокаций на узел нет.
#[derive(Default)]
pub struct BarnesHutSolver {
    nodes: Vec<Node>,
    /// Тела в порядке листьев (лист — непрерывный отрезок).
    order: Vec<u32>,
    /// Буфер для counting sort при разбиении узла.
    scratch: Vec<u32>,
    /// Листья в порядке обхода.
    leaf_nodes: Vec<u32>,
    /// Далёкие узлы для каждого листа.
    far_lists: Vec<Vec<u32>>,
    /// Ближние тела для каждого листа.
    near_lists: Vec<Vec<u32>>,
    /// Позиции, массы и `g·m` источников на текущем вызове.
    src_pos: Vec<Vec3>,
    src_mass: Vec<f64>,
    src_gm: Vec<f64>,
    /// Признак, что дерево и списки соответствуют последним источникам.
    cache_valid: bool,
    /// Листья последнего дерева (для отрисовки областей).
    regions: Vec<Region>,
    acc: Vec<Vec3>,
    /// Стек обхода дерева (переиспользуется).
    stack: Vec<u32>,
    /// Профилировщик этапов: дерево, списки, вычисление.
    #[allow(dead_code)]
    profiler: Profiler,
}

impl BarnesHutSolver {
    /// Ускорения `positions` от тел `bodies` методом Barnes-Hut.
    ///
    /// * `theta` — угол раскрытия: меньше — точнее и дороже.
    pub fn accel(
        &mut self,
        bodies: &[Body],
        positions: &[Vec3],
        g: f64,
        softening: f64,
        theta: f64,
    ) -> BarnesHutResult {
        self.acc.clear();
        self.acc.resize(positions.len(), Vec3::ZERO);
        let theta = theta.clamp(0.02, 1.5);
        let softening = softening.max(0.0);

        if bodies.is_empty() {
            self.regions.clear();
            self.cache_valid = false;
            return self.take_result();
        }

        // RK4 двигает только пробные точки: источники внутри шага неизменны,
        // поэтому дерево и списки строятся один раз на шаг.
        if !self.cache_matches(bodies, g) {
            let ((), tree_ms) = profiler::timed(|| {
                self.prepare_sources(bodies, g);
                self.build(bodies, g);
            });
            self.profiler.record("BH: дерево", tree_ms);
            let ((), lists_ms) = profiler::timed(|| self.build_interaction_lists(theta));
            self.profiler.record("BH: списки", lists_ms);
            self.cache_valid = true;
        }

        // Основной путь: обход по листьям, как в FMM.
        let ((), force_ms) = profiler::timed(|| {
            self.sum_leaves(positions, softening);
        });
        self.profiler.record("BH: расчёт сил", force_ms);

        // Точки сверх числа тел (нестандартный вызов) считаем точечно.
        for i in bodies.len()..positions.len() {
            self.acc[i] = self.direct_sum(positions[i], i, softening);
        }

        self.take_result()
    }

    /// Совпадают ли источники с закэшированными.
    fn cache_matches(&self, bodies: &[Body], g: f64) -> bool {
        self.cache_valid
            && bodies.len() == self.src_pos.len()
            && bodies
                .iter()
                .zip(self.src_pos.iter())
                .all(|(b, p)| b.pos == *p)
            && bodies
                .iter()
                .zip(self.src_mass.iter())
                .all(|(b, m)| b.mass == *m)
            && self.src_gm.first().is_some_and(|&gm| gm == g * bodies[0].mass)
    }

    /// Забрать результат, оставив буферы решателю (пустые, но с ёмкостью).
    fn take_result(&mut self) -> BarnesHutResult {
        BarnesHutResult {
            acc: std::mem::take(&mut self.acc),
            regions: self.regions.clone(),
        }
    }

    fn prepare_sources(&mut self, bodies: &[Body], g: f64) {
        self.src_pos.clear();
        self.src_pos.extend(bodies.iter().map(|b| b.pos));
        self.src_mass.clear();
        self.src_mass.extend(bodies.iter().map(|b| b.mass));
        self.src_gm.clear();
        self.src_gm.extend(bodies.iter().map(|b| g * b.mass));
    }

    // ── Построение дерева ──────────────────────────────────────────

    fn build(&mut self, bodies: &[Body], g: f64) {
        self.nodes.clear();
        self.leaf_nodes.clear();
        self.regions.clear();
        self.order.clear();
        self.order.extend(0..bodies.len() as u32);
        self.scratch.resize(bodies.len(), 0);

        let (center, half) = bounds(bodies);
        self.build_node(bodies, 0, bodies.len(), center, half, 0, MAX_LEAF);

        // Области листьев — для отрисовки на холсте.
        for &node_id in &self.leaf_nodes {
            let node = self.nodes[node_id as usize];
            self.regions.push(Region {
                center: node.center,
                half: node.half,
                mass: node.mass,
                bodies: node.body_len,
            });
        }

        // Масса узлов сразу включает g: дальняя часть складывается без
        // умножения на каждой цели. Области сохранены с физической массой.
        if g != 1.0 {
            for node in &mut self.nodes {
                node.mass *= g;
            }
        }
    }

    /// Рекурсивно построить узел и вернуть его индекс.
    fn build_node(
        &mut self,
        bodies: &[Body],
        lo: usize,
        len: usize,
        center: Vec3,
        half: f64,
        depth: u32,
        max_leaf: usize,
    ) -> u32 {
        let idx = self.nodes.len() as u32;
        self.nodes.push(Node {
            center,
            half,
            mass: 0.0,
            com: Vec3::ZERO,
            children: [NO_NODE; 8],
            body_start: 0,
            body_len: 0,
        });

        // Разбиение по октантам counting sort: без аллокаций на узел.
        let mut counts = [0usize; 8];
        for k in lo..lo + len {
            counts[octant(bodies[self.order[k] as usize].pos, center)] += 1;
        }
        let degenerate = counts.iter().any(|&c| c == len);

        if len > max_leaf && depth < MAX_DEPTH && !degenerate {
            let mut offsets = [0usize; 8];
            let mut running = 0usize;
            for k in 0..8 {
                offsets[k] = running;
                running += counts[k];
            }
            let mut cursor = offsets;
            for k in lo..lo + len {
                let b = self.order[k];
                let o = octant(bodies[b as usize].pos, center);
                self.scratch[lo + cursor[o]] = b;
                cursor[o] += 1;
            }
            self.order[lo..lo + len].copy_from_slice(&self.scratch[lo..lo + len]);

            let child_half = half * 0.5;
            for o in 0..8 {
                if counts[o] == 0 {
                    continue;
                }
                let child_center = Vec3::new(
                    center.x + if o & 1 != 0 { child_half } else { -child_half },
                    center.y + if o & 2 != 0 { child_half } else { -child_half },
                    center.z + if o & 4 != 0 { child_half } else { -child_half },
                );
                let child = self.build_node(
                    bodies,
                    lo + offsets[o],
                    counts[o],
                    child_center,
                    child_half,
                    depth + 1,
                    max_leaf,
                );
                self.nodes[idx as usize].children[o] = child;
            }

            // Центр масс родителя из детей.
            let children = self.nodes[idx as usize].children;
            let mut mass = 0.0;
            let mut com = Vec3::ZERO;
            for &ch in &children {
                if ch == NO_NODE {
                    continue;
                }
                let child = self.nodes[ch as usize];
                mass += child.mass;
                com = com + child.com * child.mass;
            }
            let node = &mut self.nodes[idx as usize];
            node.mass = mass;
            node.com = if mass > 0.0 {
                com * (1.0 / mass)
            } else {
                center
            };
            return idx;
        }

        // Лист: масса и центр масс прямо по телам.
        let mut mass = 0.0;
        let mut com = Vec3::ZERO;
        for k in lo..lo + len {
            let b = &bodies[self.order[k] as usize];
            mass += b.mass;
            com = com + b.pos * b.mass;
        }
        let node = &mut self.nodes[idx as usize];
        node.mass = mass;
        node.com = if mass > 0.0 {
            com * (1.0 / mass)
        } else {
            center
        };
        node.body_start = lo as u32;
        node.body_len = len as u32;
        self.leaf_nodes.push(idx);
        idx
    }

    /// Дерево взаимодействий: для каждого листа — далёкие узлы и ближние тела.
    /// Критерий Barnes-Hut, консервативный вариант: гарантированно верен для
    /// всех целей листа (см. шапку модуля).
    fn build_interaction_lists(&mut self, theta: f64) {
        let leaves = self.leaf_nodes.len();
        self.far_lists.resize_with(leaves, Vec::new);
        self.near_lists.resize_with(leaves, Vec::new);

        let Self {
            nodes,
            order,
            leaf_nodes,
            far_lists,
            near_lists,
            stack,
            ..
        } = self;

        for slot in 0..leaves {
            let leaf_id = leaf_nodes[slot];
            let leaf = &nodes[leaf_id as usize];
            let far = &mut far_lists[slot];
            let near = &mut near_lists[slot];
            far.clear();
            near.clear();
            stack.clear();
            stack.push(0);

            while let Some(node_id) = stack.pop() {
                let node = &nodes[node_id as usize];
                if node_id != leaf_id && bh_separated(node, leaf.center, leaf.half, theta) {
                    far.push(node_id);
                } else if node.is_leaf() {
                    let start = node.body_start as usize;
                    near.extend_from_slice(&order[start..start + node.body_len as usize]);
                } else {
                    for &child in &node.children {
                        if child != NO_NODE {
                            stack.push(child);
                        }
                    }
                }
            }
        }
    }

    // ── Вычисление ускорений ───────────────────────────────────────

    /// Основной проход: цели сгруппированы по листьям, монополь далёкого узла
    /// загружается один раз и применяется ко всем телам листа.
    fn sum_leaves(&mut self, positions: &[Vec3], softening: f64) {
        let Self {
            nodes,
            order: body_order,
            leaf_nodes,
            far_lists,
            near_lists,
            src_pos,
            src_gm,
            acc,
            ..
        } = self;
        let n = positions.len();

        for slot in 0..leaf_nodes.len() {
            let leaf = nodes[leaf_nodes[slot] as usize];
            let start = leaf.body_start as usize;
            let targets = &body_order[start..start + leaf.body_len as usize];

            for &node_id in &far_lists[slot] {
                let node = nodes[node_id as usize];
                for &i in targets {
                    let i = i as usize;
                    if i >= n {
                        continue;
                    }
                    acc[i] = acc[i] + node.accel(positions[i], softening);
                }
            }

            for &j in &near_lists[slot] {
                let j = j as usize;
                let sp = src_pos[j];
                let gm = src_gm[j];
                for &i in targets {
                    let i = i as usize;
                    if i == j || i >= n {
                        continue;
                    }
                    let d = sp - positions[i];
                    let r2 = d.len2() + softening;
                    acc[i] = acc[i] + d * (gm / (r2 * r2.sqrt()));
                }
            }
        }
    }

    /// Точный расчёт по всем телам — запасной путь для точек вне дерева.
    fn direct_sum(&self, p: Vec3, target: usize, softening: f64) -> Vec3 {
        let mut a = Vec3::ZERO;
        for (j, &pos) in self.src_pos.iter().enumerate() {
            if j == target {
                continue;
            }
            let d = pos - p;
            let r2 = d.len2() + softening;
            a = a + d * (self.src_gm[j] / (r2 * r2.sqrt()));
        }
        a
    }
}

/// Критерий раскрытия Barnes-Hut для листа-цели: сторона узла `s` меньше
/// θ·(расстояние от центра масс узла до ближайшей возможной цели листа).
/// Ближайшая цель листа не ближе `|com − c_листа| − r_листа`, где
/// `r_листа = half_листа·√3` — радиус описанной сферы куба листа.
#[inline]
fn bh_separated(node: &Node, leaf_center: Vec3, leaf_half: f64, theta: f64) -> bool {
    let s = node.half * 2.0;
    let r_leaf = leaf_half * SQRT3;
    let d = (node.com - leaf_center).len() - r_leaf;
    s <= theta * d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::vec::Rng;

    const G: f64 = 1.0;
    const SOFT: f64 = 1.0;

    fn cluster(n: usize) -> Vec<Body> {
        let mut rng = Rng::new(0x1234_5678_9ABC_DEF0);
        (0..n)
            .map(|i| Body {
                id: i as u64,
                mass: 0.5 + rng.f64() * 4.0,
                pos: Vec3::new(
                    (rng.f64() - 0.5) * 400.0,
                    (rng.f64() - 0.5) * 400.0,
                    (rng.f64() - 0.5) * 120.0,
                ),
                vel: Vec3::ZERO,
                radius: 1.0,
                color: [200, 200, 200],
            })
            .collect()
    }

    fn brute(bodies: &[Body], positions: &[Vec3], g: f64, softening: f64) -> Vec<Vec3> {
        positions
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let mut a = Vec3::ZERO;
                for (j, b) in bodies.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let d = b.pos - p;
                    let r2 = d.len2() + softening;
                    a = a + d * (g * b.mass / (r2 * r2.sqrt()));
                }
                a
            })
            .collect()
    }

    fn relative_error(exact: &[Vec3], approx: &[Vec3]) -> f64 {
        let num: f64 = exact
            .iter()
            .zip(approx.iter())
            .map(|(e, a)| (*e - *a).len())
            .fold(0.0, f64::max);
        let den: f64 = exact.iter().map(|e| e.len()).fold(0.0, f64::max);
        num / den.max(1e-12)
    }

    // Пороги подобраны по замерам на равномерном облаке (худшее тело,
    // макс-норма): монопольный BH сходится как θ², но при больших θ ошибка
    // на «размазанных» конфигурациях ощутима — на системах с доминирующей
    // массой (планетные пресеты) она существенно меньше.
    #[test]
    fn bh_matches_brute_force() {
        let bodies = cluster(1500);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let exact = brute(&bodies, &positions, G, SOFT);
        let approx = BarnesHutSolver::default()
            .accel(&bodies, &positions, G, SOFT, 0.1)
            .acc;
        let err = relative_error(&exact, &approx);
        assert!(err < 0.03, "ошибка BH θ=0.1 слишком велика: {err:.4}");
    }

    #[test]
    fn small_theta_is_accurate() {
        let bodies = cluster(800);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let exact = brute(&bodies, &positions, G, SOFT);
        let approx = BarnesHutSolver::default()
            .accel(&bodies, &positions, G, SOFT, 0.05)
            .acc;
        let err = relative_error(&exact, &approx);
        assert!(err < 0.005, "ошибка BH θ=0.05 слишком велика: {err:.4}");
    }

    #[test]
    fn error_falls_with_theta() {
        let bodies = cluster(800);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let exact = brute(&bodies, &positions, G, SOFT);
        let mut previous = f64::INFINITY;
        for theta in [0.6, 0.3, 0.1] {
            let approx = BarnesHutSolver::default()
                .accel(&bodies, &positions, G, SOFT, theta)
                .acc;
            let err = relative_error(&exact, &approx);
            assert!(
                err < previous,
                "θ={theta} не точнее предыдущего: {err:.4} ≥ {previous:.4}"
            );
            previous = err;
        }
    }

    /// Сквозной тест: шаг симуляции алгоритмом Barnes-Hut держит луну на орбите
    /// и отдаёт области дерева для отрисовки.
    #[test]
    fn bh_step_produces_regions_and_keeps_moon_orbit() {
        let params = crate::physics::SimParams {
            algorithm: crate::physics::Algorithm::BarnesHut,
            integrator: crate::physics::Integrator::SymplecticEuler,
            dt: 0.008,
            softening: 1.0,
            bh_theta: 0.3,
            ..Default::default()
        };
        let mut sim =
            crate::physics::sim::Simulation::new(params, crate::physics::Scenario::EarthMoon);
        let planet = sim.bodies()[1].pos;
        let moon = sim.bodies()[2].pos;
        let r0 = (moon - planet).len();
        for _ in 0..600 {
            sim.step();
        }
        assert!(!sim.regions().is_empty(), "BH не отдал области");
        assert!(sim.bodies().iter().all(|b| b.pos.is_finite()));
        let planet = sim.bodies()[1].pos;
        let moon = sim.bodies()[2].pos;
        let r1 = (moon - planet).len();
        assert!(
            (r1 / r0 - 1.0).abs() < 0.15,
            "луна ушла с орбиты: {r0} -> {r1}"
        );
    }

    #[test]
    fn result_is_deterministic() {
        let bodies = cluster(600);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let a = BarnesHutSolver::default().accel(&bodies, &positions, G, SOFT, 0.6);
        let b = BarnesHutSolver::default().accel(&bodies, &positions, G, SOFT, 0.6);
        for (x, y) in a.acc.iter().zip(b.acc.iter()) {
            assert_eq!(x, y, "одинаковый вход дал разный результат");
        }
    }

    #[test]
    fn reuse_matches_fresh_solver() {
        let bodies = cluster(700);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let mut reused = BarnesHutSolver::default();
        reused.accel(&bodies, &positions, G, SOFT, 0.5);
        let second = reused.accel(&bodies, &positions, G, SOFT, 0.5);
        let fresh = BarnesHutSolver::default().accel(&bodies, &positions, G, SOFT, 0.5);
        for (x, y) in second.acc.iter().zip(fresh.acc.iter()) {
            assert_eq!(x, y, "кэшированное дерево изменило результат");
        }
    }

    #[test]
    fn leaves_export_regions() {
        let bodies = cluster(400);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let res = BarnesHutSolver::default().accel(&bodies, &positions, G, SOFT, 0.5);
        let leaves: u32 = res.regions.iter().map(|r| r.bodies).sum();
        assert_eq!(leaves as usize, bodies.len(), "листы не покрывают все тела");
    }

    #[test]
    fn empty_input_is_safe() {
        let res = BarnesHutSolver::default().accel(&[], &[], G, SOFT, 0.5);
        assert!(res.acc.is_empty());
        assert!(res.regions.is_empty());
    }

    #[test]
    fn single_body_is_safe() {
        let bodies = vec![Body {
            id: 1,
            mass: 5.0,
            pos: Vec3::new(10.0, 20.0, 30.0),
            vel: Vec3::ZERO,
            radius: 1.0,
            color: [255, 255, 255],
        }];
        let positions = vec![bodies[0].pos];
        let res = BarnesHutSolver::default().accel(&bodies, &positions, G, SOFT, 0.5);
        assert!(res.acc[0].is_finite());
    }

    /// Замер: Barnes-Hut против точного расчёта (`--ignored --nocapture`).
    #[test]
    #[ignore = "бенчмарк"]
    fn benchmark_bh_against_brute_force() {
        for n in [1000, 2000, 4000, 8000] {
            let bodies = cluster(n);
            let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();

            let t = std::time::Instant::now();
            let _ = brute(&bodies, &positions, G, SOFT);
            let brute_ms = t.elapsed().as_secs_f64() * 1000.0;

            let t = std::time::Instant::now();
            let _ = BarnesHutSolver::default().accel(&bodies, &positions, G, SOFT, 0.6);
            let bh_ms = t.elapsed().as_secs_f64() * 1000.0;

            println!("N={n}: brute {brute_ms:.1} мс, BH {bh_ms:.1} мс ({:.2}×)", {
                brute_ms / bh_ms
            });
        }
    }

    /// Замер точного расчёта в движке (Algorithm::BruteForce) —
    /// `cargo test --release benchmark_sim_brute_step -- --ignored --nocapture`.
    #[test]
    #[ignore = "бенчмарк"]
    fn benchmark_sim_brute_step() {
        use crate::physics::sim::{Algorithm, Simulation, SimParams};
        for n in [1000, 4000, 8000] {
            let bodies = cluster(n);
            let params = SimParams {
                algorithm: Algorithm::BruteForce,
                softening: SOFT,
                ..Default::default()
            };
            let mut sim = Simulation::new(params, crate::physics::Scenario::RandomCloud);
            sim.replace_bodies(bodies);
            // прогрев
            sim.step();
            let t = std::time::Instant::now();
            for _ in 0..5 {
                sim.step();
            }
            println!(
                "движок BruteForce N={n}: {:.1} мс/шаг",
                t.elapsed().as_secs_f64() * 1000.0 / 5.0
            );
        }
    }
}
