//! Настоящий мультипольный метод (FMM): октодерево, моменты и дерево
//! взаимодействий.
//!
//! Идея в трёх шагах:
//! 1. **Дерево.** Детерминированное плоское октодерево (`Vec<Node>`, дети —
//!    индексы, а не `Box`). Тела хранятся в одном массиве `order`, листья
//!    занимают в нём непрерывные отрезки; разбиение — counting sort по
//!    октантам, поэтому на узел не выделяется ни одного `Vec`.
//! 2. **Моменты.** Для каждого узла считаются монополь `M`, диполь `Q` и
//!    квадруполь `T` относительно центра узла; моменты родителя собираются
//!    из детей (M2M-пересчёт со сдвигом `t = c_child − c_parent`).
//! 3. **Дерево взаимодействий.** Для каждого листа один раз строится список
//!    далёких узлов (с ними считается разложение) и список ближних тел
//!    (точный расчёт). Списки переиспользуют память между шагами.
//!
//! Разложение узла относительно центра `c` для точки `x` (`d = x − c`):
//!
//! ```text
//! Φ(x) = −G [ M/r + (d·Q)/r³ + (3(d·T d) − tr(T) r²) / (2 r⁵) + … ]
//! a = −∇Φ = G [ −M d/r³
//!                + Q/r³ − 3 (d·Q) d/r⁵
//!                + 3 T d/r⁵ + (3/2) tr(T) d/r⁵ − (15/2) (d·T d) d/r⁷ ].
//! ```
//!
//! Ряд вычисляется прямо в точке пробной частицы. Это эквивалентно
//! локальному разложению около точки, но без отдельной L2L-стадии — зато
//! удобно для RK4, где положения пробных частиц не совпадают с положениями
//! источников: списки взаимодействий строятся по листьям тел, а ряд
//! считается в фактической точке.
//!
//! Критерий раскрытия — по описанным сферам узлов:
//! `|c_a − c_b|² > (r_a + r_b)² / θ²`, где `r = half·√3` (сравниваются
//! квадраты — `sqrt` на каждом узле обхода не нужен). При `θ ≤ 1` сферы
//! расходятся, поэтому вклад от узла всегда считается снаружи.
//!
//! ## Что даёт оптимизация
//!
//! * дерево и списки строятся **один раз на шаг**: RK4 двигает только пробные
//!   точки, а источники внутри шага неизменны (кэш сверяет позиции и массы);
//! * обход идёт **по листьям**: разложение далёкого узла загружается один раз
//!   и применяется ко всем целям листа;
//! * моменты сразу масштабируются на `g`, ближние тела хранят готовые `g·m`;
//! * на узел не выделяется память: дерево плоское, разбиение — counting sort.
//!
//! Замеры (`cargo test --release benchmark_ -- --ignored --nocapture`):
//! обход по листьям в 1.3–1.6 раза быстрее старого рекурсивного обхода для
//! каждой цели; против прямого расчёта FMM выигрывает примерно с 1500–2000
//! тел (4096 тел — около 2.7×, 8192 — около 4.7× для шага RK4).
//!
//! Все операции детерминированы: порядок тел и октантов фиксирован,
//! обход стека — тоже, поэтому одинаковая конфигурация даёт бит-в-бит
//! одинаковый результат.

use super::sim::Body;
use super::vec::Vec3;

/// Максимум тел в листе.
const MAX_LEAF: usize = 8;
/// Максимальная глубина дерева — защита от вырожденных распределений.
const MAX_DEPTH: u32 = 14;
/// Сентинел «узла нет» / «листа нет».
const NO_NODE: u32 = u32::MAX;
const SQRT3: f64 = 1.732_050_807_568_877_2;

/// Лист октодерева — область, которую можно показать на холсте.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub center: Vec3,
    /// Половина стороны куба.
    pub half: f64,
    pub mass: f64,
    pub bodies: u32,
}

/// Результат мультипольного расчёта.
pub struct MultipoleResult {
    pub acc: Vec<Vec3>,
    pub regions: Vec<Region>,
}

/// Симметричная матрица вторых моментов (без множителя 1/2).
#[derive(Clone, Copy, Debug, Default)]
struct Quad {
    xx: f64,
    yy: f64,
    zz: f64,
    xy: f64,
    xz: f64,
    yz: f64,
}

impl Quad {
    #[inline]
    fn add(self, other: Self) -> Self {
        Self {
            xx: self.xx + other.xx,
            yy: self.yy + other.yy,
            zz: self.zz + other.zz,
            xy: self.xy + other.xy,
            xz: self.xz + other.xz,
            yz: self.yz + other.yz,
        }
    }

    /// `T + t⊗Q + Q⊗t + m t⊗t` — сдвиг моментов на вектор `t`.
    #[inline]
    fn shifted(self, t: Vec3, dipole: Vec3, mass: f64) -> Self {
        Self {
            xx: self.xx + 2.0 * t.x * dipole.x + mass * t.x * t.x,
            yy: self.yy + 2.0 * t.y * dipole.y + mass * t.y * t.y,
            zz: self.zz + 2.0 * t.z * dipole.z + mass * t.z * t.z,
            xy: self.xy + t.x * dipole.y + dipole.x * t.y + mass * t.x * t.y,
            xz: self.xz + t.x * dipole.z + dipole.x * t.z + mass * t.x * t.z,
            yz: self.yz + t.y * dipole.z + dipole.y * t.z + mass * t.y * t.z,
        }
    }

    #[inline]
    fn trace(self) -> f64 {
        self.xx + self.yy + self.zz
    }

    #[inline]
    fn mul(self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.xx * v.x + self.xy * v.y + self.xz * v.z,
            self.xy * v.x + self.yy * v.y + self.yz * v.z,
            self.xz * v.x + self.yz * v.y + self.zz * v.z,
        )
    }

    #[inline]
    fn dot(self, v: Vec3) -> f64 {
        v.dot(self.mul(v))
    }

    #[inline]
    fn scaled(self, k: f64) -> Self {
        Self {
            xx: self.xx * k,
            yy: self.yy * k,
            zz: self.zz * k,
            xy: self.xy * k,
            xz: self.xz * k,
            yz: self.yz * k,
        }
    }
}

/// Узел плоского октодерева. Дети — индексы в общем массиве.
#[derive(Clone, Copy, Debug)]
struct Node {
    center: Vec3,
    half: f64,
    mass: f64,
    dipole: Vec3,
    quad: Quad,
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

    /// Ускорение в точке `p` от всего узла (ряд до `order`).
    #[inline]
    fn accel(&self, p: Vec3, order: u8) -> Vec3 {
        let d = p - self.center;
        let r2 = d.len2().max(1e-12);
        let inv_r = 1.0 / r2.sqrt();
        let inv_r2 = inv_r * inv_r;
        let inv_r3 = inv_r * inv_r2;
        let mut a = d * (-self.mass * inv_r3);

        if order >= 1 {
            let dq = d.dot(self.dipole);
            a = a + self.dipole * inv_r3 - d * (3.0 * dq * inv_r3 * inv_r2);
        }
        if order >= 2 {
            let inv_r5 = inv_r3 * inv_r2;
            let inv_r7 = inv_r5 * inv_r2;
            let q = self.quad.dot(d);
            a = a + self.quad.mul(d) * (3.0 * inv_r5);
            a = a + d * (1.5 * self.quad.trace() * inv_r5);
            a = a - d * (7.5 * q * inv_r7);
        }
        a
    }
}

/// Переиспользуемый решатель: владеет деревом, списками взаимодействий и
/// буферами, поэтому после прогрева не выделяет память на каждом шаге.
#[derive(Default)]
pub struct MultipoleSolver {
    nodes: Vec<Node>,
    /// Тела в порядке листьев (лист — непрерывный отрезок).
    order: Vec<u32>,
    /// Буфер для counting sort при разбиении узла.
    scratch: Vec<u32>,
    /// Листья в порядке обхода.
    leaf_nodes: Vec<u32>,
    /// Номер листа по индексу узла (NO_NODE для внутренних).
    node_slot: Vec<u32>,
    /// Лист, в котором лежит тело.
    leaf_of_body: Vec<u32>,
    /// Дерево взаимодействий: далёкие узлы для каждого листа.
    far_lists: Vec<Vec<u32>>,
    /// Дерево взаимодействий: ближние тела для каждого листа.
    near_lists: Vec<Vec<u32>>,
    /// Позиции и `g·m` источников на текущем вызове.
    src_pos: Vec<Vec3>,
    src_mass: Vec<f64>,
    src_gm: Vec<f64>,
    /// Признак, что дерево и списки соответствуют последним источникам.
    cache_valid: bool,
    cached_g: f64,
    cached_theta: f64,
    /// Вместимость листа (0 — значение по умолчанию). Меняется в замерах.
    leaf_capacity: usize,
    /// Стек обхода дерева (переиспользуется).
    stack: Vec<u32>,
    acc: Vec<Vec3>,
    regions: Vec<Region>,
}

impl MultipoleSolver {
    /// Ускорения `positions` от тел `bodies`.
    ///
    /// * `theta` — угол раскрытия: меньше — точнее и дороже;
    /// * `order` — порядок разложения: 0 — монополь, 1 — диполь, 2 — квадруполь.
    pub fn accel(
        &mut self,
        bodies: &[Body],
        positions: &[Vec3],
        g: f64,
        softening: f64,
        theta: f64,
        order: u8,
    ) -> MultipoleResult {
        self.acc.clear();
        self.acc.resize(positions.len(), Vec3::ZERO);

        let theta = theta.clamp(0.2, 1.0);
        let order = order.min(2);
        let softening = softening.max(0.0);

        if bodies.is_empty() {
            self.regions.clear();
            self.cache_valid = false;
            return self.take_result();
        }

        // RK4 двигает только пробные точки: источники внутри шага неизменны.
        // Поэтому дерево, моменты и списки строятся один раз на шаг.
        if !self.cache_matches(bodies, g, theta) {
            self.prepare_sources(bodies, g);
            self.build(bodies, g);
            self.build_interaction_lists(theta);
            self.cached_g = g;
            self.cached_theta = theta;
            self.cache_valid = true;
        }

        // Основной путь: обход по листьям. Далёкий узел загружается один раз
        // и применяется ко всем целям листа, ближнее тело — тоже.
        self.sum_leaves(positions, softening, order);

        // Точки сверх числа тел (нестандартный вызов) считаем точечно.
        for i in bodies.len()..positions.len() {
            let p = positions[i];
            let node = self.locate(p);
            let slot = self.node_slot[node as usize];
            self.acc[i] = if slot != NO_NODE {
                self.sum_slot(slot, p, i, softening, order)
            } else {
                self.direct_sum(p, i, softening)
            };
        }

        self.take_result()
    }

    /// Совпадают ли источники и параметры разбиения с закэшированными.
    fn cache_matches(&self, bodies: &[Body], g: f64, theta: f64) -> bool {
        self.cache_valid
            && self.cached_g == g
            && self.cached_theta == theta
            && bodies.len() == self.src_pos.len()
            && bodies
                .iter()
                .zip(self.src_pos.iter())
                .all(|(b, p)| b.pos == *p)
            && bodies
                .iter()
                .zip(self.src_mass.iter())
                .all(|(b, m)| b.mass == *m)
    }

    /// Забрать результат, оставив буферы решателю (пустые, но с ёмкостью).
    fn take_result(&mut self) -> MultipoleResult {
        MultipoleResult {
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
        let max_leaf = if self.leaf_capacity == 0 {
            MAX_LEAF
        } else {
            self.leaf_capacity
        };
        self.build_node(bodies, 0, bodies.len(), center, half, 0, max_leaf);

        // Номера листьев и привязка тел.
        self.node_slot.clear();
        self.node_slot.resize(self.nodes.len(), NO_NODE);
        self.leaf_of_body.clear();
        self.leaf_of_body.resize(bodies.len(), NO_NODE);
        for (slot, &node_id) in self.leaf_nodes.iter().enumerate() {
            self.node_slot[node_id as usize] = slot as u32;
            let node = self.nodes[node_id as usize];
            let start = node.body_start as usize;
            for k in start..start + node.body_len as usize {
                self.leaf_of_body[self.order[k] as usize] = slot as u32;
            }
            self.regions.push(Region {
                center: node.center,
                half: node.half,
                mass: node.mass,
                bodies: node.body_len,
            });
        }

        // Моменты сразу включают g: дальняя часть затем складывается без
        // умножения на каждой цели. Области уже сохранены с физической массой.
        if g != 1.0 {
            for node in &mut self.nodes {
                node.mass *= g;
                node.dipole = node.dipole * g;
                node.quad = node.quad.scaled(g);
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
            dipole: Vec3::ZERO,
            quad: Quad::default(),
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

            // M2M: моменты родителя из моментов детей.
            let children = self.nodes[idx as usize].children;
            let mut mass = 0.0;
            let mut dipole = Vec3::ZERO;
            let mut quad = Quad::default();
            for &ch in &children {
                if ch == NO_NODE {
                    continue;
                }
                let child = self.nodes[ch as usize];
                let t = child.center - center;
                mass += child.mass;
                dipole = dipole + child.dipole + t * child.mass;
                quad = quad.add(child.quad.shifted(t, child.dipole, child.mass));
            }
            let node = &mut self.nodes[idx as usize];
            node.mass = mass;
            node.dipole = dipole;
            node.quad = quad;
            return idx;
        }

        // Лист: моменты прямо по телам.
        let mut mass = 0.0;
        let mut dipole = Vec3::ZERO;
        let mut quad = Quad::default();
        for k in lo..lo + len {
            let b = &bodies[self.order[k] as usize];
            let s = b.pos - center;
            mass += b.mass;
            dipole = dipole + s * b.mass;
            quad = quad.add(Quad {
                xx: b.mass * s.x * s.x,
                yy: b.mass * s.y * s.y,
                zz: b.mass * s.z * s.z,
                xy: b.mass * s.x * s.y,
                xz: b.mass * s.x * s.z,
                yz: b.mass * s.y * s.z,
            });
        }
        let node = &mut self.nodes[idx as usize];
        node.mass = mass;
        node.dipole = dipole;
        node.quad = quad;
        node.body_start = lo as u32;
        node.body_len = len as u32;
        self.leaf_nodes.push(idx);
        idx
    }

    /// Дерево взаимодействий: для каждого листа — далёкие узлы и ближние тела.
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
                if node_id != leaf_id && well_separated(node, leaf.center, leaf.half, theta) {
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

    /// Лист, содержащий точку (спуск по октантам). Нужен, только если точек
    /// больше, чем тел; в обычном расчёте используется `leaf_of_body`.
    fn locate(&self, p: Vec3) -> u32 {
        let mut node_id = 0u32;
        loop {
            let node = self.nodes[node_id as usize];
            if node.is_leaf() {
                return node_id;
            }
            let child = node.children[octant(p, node.center)];
            if child == NO_NODE {
                return node_id;
            }
            node_id = child;
        }
    }

    // ── Вычисление ускорений ───────────────────────────────────────

    /// Основной проход: цели сгруппированы по листьям, поэтому разложение
    /// далёкого узла вычисляется из регистров и раскладывается сразу на все
    /// тела листа, а не перезагружается для каждой цели.
    fn sum_leaves(&mut self, positions: &[Vec3], softening: f64, order: u8) {
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
                    acc[i] = acc[i] + node.accel(positions[i], order);
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

    /// Одна произвольная точка — запасной путь для вызовов, где целей больше,
    /// чем тел (в симуляции не используется).
    fn sum_slot(&self, slot: u32, p: Vec3, target: usize, softening: f64, order: u8) -> Vec3 {
        let mut a = Vec3::ZERO;
        for &node_id in &self.far_lists[slot as usize] {
            a = a + self.nodes[node_id as usize].accel(p, order);
        }

        for &j in &self.near_lists[slot as usize] {
            let j = j as usize;
            if j == target {
                continue;
            }
            let d = self.src_pos[j] - p;
            let r2 = d.len2() + softening;
            a = a + d * (self.src_gm[j] / (r2 * r2.sqrt()));
        }
        a
    }

    /// Точный расчёт по всем телам — запасной путь, если точка вне дерева.
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

/// Ускорения `positions` от тел `bodies` мультипольным методом.
///
/// Удобная обёртка над [`MultipoleSolver`] для разовых вызовов; в цикле
/// симуляции лучше держать собственный решатель, чтобы не строить дерево
/// и списки заново с нуля.
#[allow(dead_code)]
pub fn accel_multipole(
    bodies: &[Body],
    positions: &[Vec3],
    g: f64,
    softening: f64,
    theta: f64,
    order: u8,
) -> MultipoleResult {
    MultipoleSolver::default().accel(bodies, positions, g, softening, theta, order)
}

#[inline]
fn octant(p: Vec3, center: Vec3) -> usize {
    let mut k = 0;
    if p.x >= center.x {
        k |= 1;
    }
    if p.y >= center.y {
        k |= 2;
    }
    if p.z >= center.z {
        k |= 4;
    }
    k
}

/// Критерий раскрытия по описанным сферам: сферы узлов должны разойтись.
/// Сравниваем квадраты расстояний — `sqrt` на каждом узле обхода не нужен.
#[inline]
fn well_separated(node: &Node, center: Vec3, half: f64, theta: f64) -> bool {
    let ra = node.half * SQRT3;
    let rb = half * SQRT3;
    let reach = ra + rb;
    (node.center - center).len2() > reach * reach / (theta * theta)
}

fn bounds(bodies: &[Body]) -> (Vec3, f64) {
    let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for b in bodies {
        min.x = min.x.min(b.pos.x);
        min.y = min.y.min(b.pos.y);
        min.z = min.z.min(b.pos.z);
        max.x = max.x.max(b.pos.x);
        max.y = max.y.max(b.pos.y);
        max.z = max.z.max(b.pos.z);
    }
    let size = max - min;
    let center = (min + max) * 0.5;
    let half = size.x.max(size.y).max(size.z) * 0.5;
    (center, half.max(1e-6))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::vec::Rng;

    fn cluster(n: usize) -> Vec<Body> {
        let mut rng = Rng::new(0xDEAD_BEEF_1234_5678);
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
        let scale = exact.iter().fold(0.0_f64, |m, a| m.max(a.len())).max(1e-9);
        exact
            .iter()
            .zip(approx.iter())
            .fold(0.0_f64, |m, (a, b)| m.max((*a - *b).len() / scale))
    }

    #[test]
    fn node_expansion_is_accurate() {
        let bodies = vec![
            Body {
                id: 0,
                mass: 2.0,
                pos: Vec3::new(10.0, 0.0, 0.0),
                vel: Vec3::ZERO,
                radius: 1.0,
                color: [0, 0, 0],
            },
            Body {
                id: 1,
                mass: 5.0,
                pos: Vec3::new(-4.0, 7.0, 2.0),
                vel: Vec3::ZERO,
                radius: 1.0,
                color: [0, 0, 0],
            },
        ];
        let mut solver = MultipoleSolver::default();
        solver.build(&bodies, 1.0);
        let node = solver.nodes[0];
        let p = Vec3::new(100.0, 7.0, -3.0);
        let mut exact = Vec3::ZERO;
        for b in &bodies {
            let d = b.pos - p;
            let r2 = d.len2();
            exact = exact + d * (b.mass / (r2 * r2.sqrt()));
        }
        let mut worst = f64::INFINITY;
        for order in 0..=2 {
            let approx = node.accel(p, order);
            worst = worst.min((approx - exact).len());
        }
        assert!(worst < 1e-5, "квадрупольное разложение неточно: {worst}");
    }

    #[test]
    fn multipole_matches_brute_force() {
        let bodies = cluster(240);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let exact = brute(&bodies, &positions, 1.0, 4.0);
        let approx = accel_multipole(&bodies, &positions, 1.0, 4.0, 0.5, 2);
        let worst = relative_error(&exact, &approx.acc);
        assert!(
            worst < 0.02,
            "относительная ошибка FMM слишком велика: {worst}"
        );
        assert!(!approx.regions.is_empty());
    }

    #[test]
    fn accuracy_holds_for_all_orders_and_angles() {
        let bodies = cluster(400);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let exact = brute(&bodies, &positions, 1.0, 4.0);
        // Ошибка должна монотонно падать с порядком при одном угле.
        let mut previous = f64::INFINITY;
        for order in 0..=2 {
            let approx = accel_multipole(&bodies, &positions, 1.0, 4.0, 0.4, order);
            let err = relative_error(&exact, &approx.acc);
            assert!(
                err <= previous + 1e-9,
                "порядок {order} не точнее предыдущего: {err} > {previous}"
            );
            previous = err;
        }
        // И с ростом угла точность не должна катастрофически падать.
        for theta in [0.2, 0.4, 0.7, 1.0] {
            let approx = accel_multipole(&bodies, &positions, 1.0, 4.0, theta, 2);
            let err = relative_error(&exact, &approx.acc);
            assert!(err < 0.08, "θ = {theta}: слишком большая ошибка {err}");
        }
    }

    fn collect_bodies(solver: &MultipoleSolver, node_id: u32, out: &mut Vec<u32>) {
        let node = solver.nodes[node_id as usize];
        if node.is_leaf() {
            let start = node.body_start as usize;
            out.extend_from_slice(&solver.order[start..start + node.body_len as usize]);
        } else {
            for &child in &node.children {
                if child != NO_NODE {
                    collect_bodies(solver, child, out);
                }
            }
        }
    }

    #[test]
    fn interaction_lists_cover_every_body_once() {
        let bodies = cluster(500);
        let mut solver = MultipoleSolver::default();
        solver.prepare_sources(&bodies, 1.0);
        solver.build(&bodies, 1.0);
        solver.build_interaction_lists(0.5);

        let total: f64 = bodies.iter().map(|b| b.mass).sum();
        for slot in 0..solver.leaf_nodes.len() {
            let mut mass_near = 0.0;
            let mut mass_far = 0.0;
            let mut seen = vec![false; bodies.len()];

            for &j in &solver.near_lists[slot] {
                let j = j as usize;
                assert!(!seen[j], "тело {j} попало в ближний список дважды");
                seen[j] = true;
                mass_near += bodies[j].mass;
            }
            let mut far_bodies = Vec::new();
            for &node_id in &solver.far_lists[slot] {
                mass_far += solver.nodes[node_id as usize].mass;
                collect_bodies(&solver, node_id, &mut far_bodies);
            }
            for j in far_bodies {
                let j = j as usize;
                assert!(!seen[j], "тело {j} в далёком и ближнем списке сразу");
                seen[j] = true;
            }
            assert!(seen.iter().all(|&s| s), "лист {slot}: часть тел потеряна");
            assert!(
                (mass_near + mass_far - total).abs() < 1e-9,
                "лист {slot}: сумма масс {} вместо {total}",
                mass_near + mass_far
            );
        }
    }

    #[test]
    fn far_nodes_respect_opening_criterion() {
        let bodies = cluster(300);
        let mut solver = MultipoleSolver::default();
        solver.prepare_sources(&bodies, 1.0);
        solver.build(&bodies, 1.0);
        solver.build_interaction_lists(0.6);
        for slot in 0..solver.leaf_nodes.len() {
            let leaf = solver.nodes[solver.leaf_nodes[slot] as usize];
            for &node_id in &solver.far_lists[slot] {
                let node = solver.nodes[node_id as usize];
                assert!(
                    well_separated(&node, leaf.center, leaf.half, 0.6),
                    "далёкий узел нарушает критерий раскрытия"
                );
            }
        }
    }

    #[test]
    fn result_is_deterministic() {
        let bodies = cluster(300);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let a = accel_multipole(&bodies, &positions, 1.0, 4.0, 0.5, 2);
        let b = accel_multipole(&bodies, &positions, 1.0, 4.0, 0.5, 2);
        for (x, y) in a.acc.iter().zip(b.acc.iter()) {
            assert_eq!(x.x.to_bits(), y.x.to_bits());
            assert_eq!(x.y.to_bits(), y.y.to_bits());
            assert_eq!(x.z.to_bits(), y.z.to_bits());
        }
    }

    #[test]
    fn reuse_matches_fresh_solver() {
        // Повторные вызовы на одном решателе не должны зависеть от истории.
        let a = cluster(200);
        let b = cluster(600);
        let pa: Vec<Vec3> = a.iter().map(|b| b.pos).collect();
        let pb: Vec<Vec3> = b.iter().map(|b| b.pos).collect();

        let fresh_a = accel_multipole(&a, &pa, 1.0, 4.0, 0.5, 2);
        let fresh_b = accel_multipole(&b, &pb, 1.0, 4.0, 0.5, 2);

        let mut solver = MultipoleSolver::default();
        let first_b = solver.accel(&b, &pb, 1.0, 4.0, 0.5, 2);
        let reused_a = solver.accel(&a, &pa, 1.0, 4.0, 0.5, 2);

        for (x, y) in reused_a.acc.iter().zip(fresh_a.acc.iter()) {
            assert!((*x - *y).len() < 1e-12, "повторный вызов расходится");
        }
        for (x, y) in first_b.acc.iter().zip(fresh_b.acc.iter()) {
            assert!((*x - *y).len() < 1e-12, "первый вызов расходится");
        }
    }

    #[test]
    fn cached_tree_handles_shifted_targets() {
        // Как в RK4: источники те же, пробные точки сдвинуты.
        let bodies = cluster(300);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let mut solver = MultipoleSolver::default();
        let _ = solver.accel(&bodies, &positions, 1.0, 4.0, 0.5, 2);

        let moved: Vec<Vec3> = positions
            .iter()
            .map(|p| *p + Vec3::new(0.7, -0.4, 0.25))
            .collect();
        let approx = solver.accel(&bodies, &moved, 1.0, 4.0, 0.5, 2);
        let exact = brute(&bodies, &moved, 1.0, 4.0);
        let err = relative_error(&exact, &approx.acc);
        assert!(err < 0.02, "сдвинутые цели: ошибка {err}");
    }

    #[test]
    fn order_zero_is_plain_monopole() {
        let bodies = cluster(64);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let mono = accel_multipole(&bodies, &positions, 1.0, 0.0, 0.8, 0);
        let quad = accel_multipole(&bodies, &positions, 1.0, 0.0, 0.8, 2);
        // Монополь грубее квадруполя, но обе оценки должны быть близки по порядку.
        let m = mono.acc.iter().fold(0.0_f64, |s, a| s.max(a.len()));
        let q = quad.acc.iter().fold(0.0_f64, |s, a| s.max(a.len()));
        assert!((m - q).abs() / q.max(1e-9) < 0.2);
    }

    #[test]
    fn empty_input_is_safe() {
        let result = accel_multipole(
            &[],
            &[Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0)],
            1.0,
            0.0,
            0.5,
            2,
        );
        assert_eq!(result.acc.len(), 2);
        assert!(result.regions.is_empty());
        assert!(result.acc.iter().all(|a| a.len2() == 0.0));
    }

    /// Сравнение с прямым расчётом: скорость и точность.
    ///
    /// ```text
    /// cargo test --release benchmark_fmm -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "бенчмарк, запускать вручную в release"]
    fn benchmark_fmm_against_brute_force() {
        use std::time::Instant;

        println!("\n   N | brute, мс | FMM холодн., мс | FMM шаг RK4, мс | ускорение | ошибка");
        for n in [256usize, 512, 1024, 2048, 4096, 8192] {
            let bodies = cluster(n);
            let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();

            let t = Instant::now();
            let exact = brute(&bodies, &positions, 1.0, 4.0);
            let brute_ms = t.elapsed().as_secs_f64() * 1e3;

            // Прогрев на других данных, чтобы измерить холодный вызов.
            let warm = cluster(64);
            let warm_pos: Vec<Vec3> = warm.iter().map(|b| b.pos).collect();
            let mut solver = MultipoleSolver::default();
            let _ = solver.accel(&warm, &warm_pos, 1.0, 4.0, 0.55, 2);

            let t = Instant::now();
            let fmm = solver.accel(&bodies, &positions, 1.0, 4.0, 0.55, 2);
            let cold_ms = t.elapsed().as_secs_f64() * 1e3;
            let err = relative_error(&exact, &fmm.acc);

            // Шаг RK4: 4 стадии, источники неизменны → структура строится один раз.
            let t = Instant::now();
            let mut last = Vec::new();
            for _ in 0..4 {
                last = solver.accel(&bodies, &positions, 1.0, 4.0, 0.55, 2).acc;
            }
            let step_ms = t.elapsed().as_secs_f64() * 1e3;
            let err4 = relative_error(&exact, &last);

            println!(
                "{n:4} | {brute_ms:9.3} | {cold_ms:15.3} | {step_ms:15.3} | {:8.2}x | {:.3}%",
                (4.0 * brute_ms) / step_ms,
                err4.max(err) * 100.0
            );
        }
    }

    /// Подбор параметров: как θ и порядок влияют на время и точность.
    #[test]
    #[ignore = "бенчмарк, запускать вручную в release"]
    fn benchmark_list_sizes() {
        for n in [256usize, 1024, 4096] {
            let bodies = cluster(n);
            let mut solver = MultipoleSolver::default();
            solver.prepare_sources(&bodies, 1.0);
            solver.build(&bodies, 1.0);
            solver.build_interaction_lists(0.55);
            let far: usize = solver.far_lists.iter().map(|l| l.len()).sum();
            let near: usize = solver.near_lists.iter().map(|l| l.len()).sum();
            let leaves = solver.leaf_nodes.len();
            println!(
                "n={n}: узлов {}, листьев {leaves}, далёких на лист {:.1}, ближних тел на лист {:.1}",
                solver.nodes.len(),
                far as f64 / leaves as f64,
                near as f64 / leaves as f64
            );
        }
    }

    /// Подбор параметров: как θ и порядок влияют на время и точность.
    #[test]
    #[ignore = "бенчмарк, запускать вручную в release"]
    fn benchmark_build_parts() {
        use std::time::Instant;
        for n in [1024usize, 4096, 16384] {
            let bodies = cluster(n);
            let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
            let mut solver = MultipoleSolver::default();
            solver.prepare_sources(&bodies, 1.0);

            let t = Instant::now();
            solver.build(&bodies, 1.0);
            let build_ms = t.elapsed().as_secs_f64() * 1e3;

            let t = Instant::now();
            solver.build_interaction_lists(0.55);
            let lists_ms = t.elapsed().as_secs_f64() * 1e3;

            let t = Instant::now();
            let _ = solver.accel(&bodies, &positions, 1.0, 4.0, 0.55, 2);
            let eval_ms = t.elapsed().as_secs_f64() * 1e3;

            println!(
                "n={n}: дерево {build_ms:.3} мс, списки {lists_ms:.3} мс, вычисление {eval_ms:.3} мс"
            );
        }
    }

    /// Подбор параметров: как θ и порядок влияют на время и точность.
    #[test]
    #[ignore = "бенчмарк, запускать вручную в release"]
    fn benchmark_interaction_tree_vs_recursive_walk() {
        use std::time::Instant;

        fn legacy_walk(
            nodes: &[Node],
            node_id: u32,
            p: Vec3,
            target: usize,
            solver: &MultipoleSolver,
            softening: f64,
            theta: f64,
            order: u8,
        ) -> Vec3 {
            let node = nodes[node_id as usize];
            let dist = (node.center - p).len();
            let inside = (p.x - node.center.x).abs() <= node.half
                && (p.y - node.center.y).abs() <= node.half
                && (p.z - node.center.z).abs() <= node.half;
            if !inside && dist > node.half * 2.0 / theta {
                return node.accel(p, order);
            }
            if node.is_leaf() {
                let start = node.body_start as usize;
                let mut a = Vec3::ZERO;
                for k in start..start + node.body_len as usize {
                    let j = solver.order[k] as usize;
                    if j == target {
                        continue;
                    }
                    let d = solver.src_pos[j] - p;
                    let r2 = d.len2() + softening;
                    a = a + d * (solver.src_gm[j] / (r2 * r2.sqrt()));
                }
                return a;
            }
            let mut a = Vec3::ZERO;
            for &child in &node.children {
                if child != NO_NODE {
                    a = a + legacy_walk(nodes, child, p, target, solver, softening, theta, order);
                }
            }
            a
        }

        println!(
            "\n   N | рекурсивный обход, мс | дерево взаимодействий, мс | ускорение | расхождение"
        );
        for n in [1024usize, 4096, 16384] {
            let bodies = cluster(n);
            let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
            let mut solver = MultipoleSolver::default();
            solver.prepare_sources(&bodies, 1.0);
            solver.build(&bodies, 1.0);
            solver.build_interaction_lists(0.55);

            let t = Instant::now();
            let mut old = vec![Vec3::ZERO; n];
            for (i, &p) in positions.iter().enumerate() {
                old[i] = legacy_walk(&solver.nodes, 0, p, i, &solver, 4.0, 0.55, 2);
            }
            let old_ms = t.elapsed().as_secs_f64() * 1e3;

            let t = Instant::now();
            solver.acc.clear();
            solver.acc.resize(n, Vec3::ZERO);
            solver.sum_leaves(&positions, 4.0, 2);
            let new_ms = t.elapsed().as_secs_f64() * 1e3;

            let err = relative_error(&old, &solver.acc);
            println!(
                "{n:4} | {old_ms:20.3} | {new_ms:22.3} | {:8.2}x | {:.4}%",
                old_ms / new_ms,
                err * 100.0
            );
        }
    }

    /// Подбор вместимости листа: компромисс между ближним полем и числом
    /// далёких узлов.
    #[test]
    #[ignore = "бенчмарк, запускать вручную в release"]
    fn benchmark_leaf_capacity() {
        use std::time::Instant;

        println!(
            "\n   N | лист | листьев | ближних/лист | далёких/лист | сборка, мс | расчёт, мс | ошибка"
        );
        for n in [1024usize, 4096] {
            let bodies = cluster(n);
            let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
            let exact = brute(&bodies, &positions, 1.0, 4.0);

            for cap in [2usize, 3, 4, 8, 16] {
                let mut solver = MultipoleSolver::default();
                solver.leaf_capacity = cap;

                let t = Instant::now();
                let first = solver.accel(&bodies, &positions, 1.0, 4.0, 0.55, 2);
                let build_ms = t.elapsed().as_secs_f64() * 1e3;

                let t = Instant::now();
                let cached = solver.accel(&bodies, &positions, 1.0, 4.0, 0.55, 2);
                let eval_ms = t.elapsed().as_secs_f64() * 1e3;

                let leaves = solver.leaf_nodes.len();
                let far: usize = solver.far_lists.iter().map(|l| l.len()).sum();
                let near: usize = solver.near_lists.iter().map(|l| l.len()).sum();
                let err =
                    relative_error(&exact, &cached.acc).max(relative_error(&exact, &first.acc));
                println!(
                    "{n:4} | {cap:4} | {leaves:7} | {:12.1} | {:12.1} | {build_ms:9.3} | {eval_ms:9.3} | {:.3}%",
                    near as f64 / leaves as f64,
                    far as f64 / leaves as f64,
                    err * 100.0
                );
            }
        }
    }

    /// Подбор параметров: как θ и порядок влияют на время и точность.
    #[test]
    #[ignore = "бенчмарк, запускать вручную в release"]
    fn benchmark_parameters() {
        use std::time::Instant;

        let n = 1024;
        let bodies = cluster(n);
        let positions: Vec<Vec3> = bodies.iter().map(|b| b.pos).collect();
        let exact = brute(&bodies, &positions, 1.0, 4.0);

        println!("\n  θ  | порядок | время, мс | ошибка");
        for theta in [0.3, 0.4, 0.55, 0.7, 0.9, 1.0] {
            for order in [0u8, 1, 2] {
                let mut solver = MultipoleSolver::default();
                let _ = solver.accel(&bodies, &positions, 1.0, 4.0, theta, order);
                let t = Instant::now();
                let fmm = solver.accel(&bodies, &positions, 1.0, 4.0, theta, order);
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let err = relative_error(&exact, &fmm.acc);
                println!("{theta:4.2} | {order:7} | {ms:9.3} | {:.4}%", err * 100.0);
            }
        }
    }
}
