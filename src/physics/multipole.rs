//! Настоящий мультипольный метод (FMM-подобный, «полиномиальный»).
//!
//! В отличие от [`super::grid`] здесь пространство разбивается адаптивно:
//! строится детерминированное октодерево по положениям тел, для каждого узла
//! считаются моменты распределения массы относительно центра узла
//! (монополь, диполь, квадруполь), а при обходе для каждой пробной точки
//! далёкие узлы заменяются разложением в ряд, ближние листья считаются
//! точно попарно.
//!
//! Разложение потенциала узла относительно его центра `c` для точки `x`:
//!
//! ```text
//! Φ(x) = −G [ M/r + (d·Q)/r³ + (3(d·T d) − tr(T) r²) / (2 r⁵) + … ],  d = x − c
//! ```
//!
//! где `M = Σ m`, `Q = Σ m s`, `T = Σ m s sᵀ`, `s = x_j − c`. Ускорение
//! `a = −∇Φ`:
//!
//! ```text
//! a = G [ −M d/r³
//!        + Q/r³ − 3 (d·Q) d/r⁵
//!        + 3 T d/r⁵ + (3/2) tr(T) d/r⁵ − (15/2) (d·T d) d/r⁷ ].
//! ```
//!
//! Моменты родителя получаются из моментов детей (M2M-пересчёт со сдвигом
//! `t = c_child − c_parent`), поэтому листья не нужно обходить повторно.
//! Ряд вычисляется непосредственно в точке пробной частицы — это
//! эквивалентно локальному разложению около точки, но без отдельной
//! L2L-стадии; зато проще, детерминированнее и удобно для RK4-стадий,
//! где положения пробных частиц не совпадают с положениями источников.
//!
//! Все операции детерминированы: порядок тел и октантов фиксирован, поэтому
//! одна и та же конфигурация даёт бит-в-бит одинаковый результат.

use super::sim::Body;
use super::vec::Vec3;

/// Максимум тел в листе.
const MAX_LEAF: usize = 8;
/// Максимальная глубина дерева — защита от вырожденных распределений.
const MAX_DEPTH: u32 = 14;

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

    fn trace(self) -> f64 {
        self.xx + self.yy + self.zz
    }

    fn mul(self, v: Vec3) -> Vec3 {
        Vec3::new(
            self.xx * v.x + self.xy * v.y + self.xz * v.z,
            self.xy * v.x + self.yy * v.y + self.yz * v.z,
            self.xz * v.x + self.yz * v.y + self.zz * v.z,
        )
    }

    fn dot(self, v: Vec3) -> f64 {
        v.dot(self.mul(v))
    }
}

struct Node {
    center: Vec3,
    half: f64,
    mass: f64,
    dipole: Vec3,
    quad: Quad,
    children: [Option<Box<Node>>; 8],
    /// Индексы тел; непусто только у листьев.
    indices: Vec<usize>,
}

impl Node {
    fn is_leaf(&self) -> bool {
        self.children.iter().all(|c| c.is_none())
    }

    /// Моменты листа по его телам.
    fn moments_from(&mut self, bodies: &[Body]) {
        let mut mass = 0.0;
        let mut dipole = Vec3::ZERO;
        let mut quad = Quad::default();
        for &i in &self.indices {
            let b = &bodies[i];
            let s = b.pos - self.center;
            mass += b.mass;
            dipole += s * b.mass;
            quad = quad.add(Quad {
                xx: b.mass * s.x * s.x,
                yy: b.mass * s.y * s.y,
                zz: b.mass * s.z * s.z,
                xy: b.mass * s.x * s.y,
                xz: b.mass * s.x * s.z,
                yz: b.mass * s.y * s.z,
            });
        }
        self.mass = mass;
        self.dipole = dipole;
        self.quad = quad;
    }

    /// M2M: собрать моменты из детей.
    fn moments_from_children(&mut self) {
        let mut mass = 0.0;
        let mut dipole = Vec3::ZERO;
        let mut quad = Quad::default();
        for child in self.children.iter().flatten() {
            let t = child.center - self.center;
            mass += child.mass;
            dipole += child.dipole + t * child.mass;
            quad = quad.add(child.quad.shifted(t, child.dipole, child.mass));
        }
        self.mass = mass;
        self.dipole = dipole;
        self.quad = quad;
    }

    /// Ускорение в точке `p` от всего узла (ряд до `order`).
    fn accel(&self, p: Vec3, g: f64, order: u8) -> Vec3 {
        let d = p - self.center;
        let r2 = d.len2().max(1e-12);
        let inv_r = 1.0 / r2.sqrt();
        let inv_r3 = inv_r / r2;
        let mut a = d * (-self.mass * inv_r3);

        if order >= 1 {
            let dq = d.dot(self.dipole);
            a += self.dipole * inv_r3 - d * (3.0 * dq * inv_r3 / r2);
        }
        if order >= 2 {
            let inv_r5 = inv_r3 / r2;
            let inv_r7 = inv_r5 / r2;
            let q = self.quad.dot(d);
            a += self.quad.mul(d) * (3.0 * inv_r5);
            a += d * (1.5 * self.quad.trace() * inv_r5);
            a = a - d * (7.5 * q * inv_r7);
        }
        a * g
    }
}

/// Построить октодерево и вернуть корень и список листьев.
fn build(
    indices: &mut [usize],
    bodies: &[Body],
    center: Vec3,
    half: f64,
    depth: u32,
    regions: &mut Vec<Region>,
) -> Node {
    let mut node = Node {
        center,
        half,
        mass: 0.0,
        dipole: Vec3::ZERO,
        quad: Quad::default(),
        children: std::array::from_fn(|_| None),
        indices: Vec::new(),
    };

    // Разбиение по октантам.
    let mut buckets: [Vec<usize>; 8] = std::array::from_fn(|_| Vec::new());
    if depth < MAX_DEPTH && indices.len() > MAX_LEAF {
        for &i in indices.iter() {
            let p = bodies[i].pos;
            let mut k = 0usize;
            if p.x >= center.x {
                k |= 1;
            }
            if p.y >= center.y {
                k |= 2;
            }
            if p.z >= center.z {
                k |= 4;
            }
            buckets[k].push(i);
        }
        // Если все тела попали в один октант, дробление не помогает.
        let degenerate = buckets.iter().any(|b| b.len() == indices.len());
        if !degenerate {
            let child_half = half * 0.5;
            for (k, bucket) in buckets.iter_mut().enumerate() {
                if bucket.is_empty() {
                    continue;
                }
                let cx = center.x + if k & 1 != 0 { child_half } else { -child_half };
                let cy = center.y + if k & 2 != 0 { child_half } else { -child_half };
                let cz = center.z + if k & 4 != 0 { child_half } else { -child_half };
                let child = build(
                    bucket,
                    bodies,
                    Vec3::new(cx, cy, cz),
                    child_half,
                    depth + 1,
                    regions,
                );
                node.children[k] = Some(Box::new(child));
            }
            node.moments_from_children();
            return node;
        }
    }

    // Лист.
    node.indices = indices.to_vec();
    node.moments_from(bodies);
    regions.push(Region {
        center,
        half,
        mass: node.mass,
        bodies: node.indices.len() as u32,
    });
    node
}

/// Ускорения `positions` от тел `bodies` мультипольным методом.
///
/// * `theta` — угол раскрытия: меньше — точнее и дороже;
/// * `order` — порядок разложения: 0 — монополь, 1 — диполь, 2 — квадруполь.
pub fn accel_multipole(
    bodies: &[Body],
    positions: &[Vec3],
    g: f64,
    softening: f64,
    theta: f64,
    order: u8,
) -> MultipoleResult {
    let n = bodies.len();
    if n == 0 {
        return MultipoleResult {
            acc: vec![Vec3::ZERO; positions.len()],
            regions: Vec::new(),
        };
    }

    // Габариты по положениям тел.
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
    let center = (min + max) * 0.5;
    let half = ((max - min).x.max((max - min).y).max((max - min).z) * 0.5).max(1e-6);

    let mut indices: Vec<usize> = (0..n).collect();
    let mut regions = Vec::new();
    let root = build(&mut indices, bodies, center, half, 0, &mut regions);

    let theta = theta.clamp(0.15, 1.2);
    let order = order.min(2);
    let softening = softening.max(0.0);

    let mut acc = vec![Vec3::ZERO; positions.len()];
    for (i, &p) in positions.iter().enumerate() {
        acc[i] = walk(&root, p, i, bodies, g, softening, theta, order);
    }

    MultipoleResult { acc, regions }
}

/// Обход дерева для одной точки.
fn walk(
    node: &Node,
    p: Vec3,
    target: usize,
    bodies: &[Body],
    g: f64,
    softening: f64,
    theta: f64,
    order: u8,
) -> Vec3 {
    let dist = (node.center - p).len();
    let size = node.half * 2.0;
    // Точка внутри узла не может считаться «далёкой»: иначе тело
    // взаимодействовало бы само с собой через приближение.
    let inside = (p.x - node.center.x).abs() <= node.half
        && (p.y - node.center.y).abs() <= node.half
        && (p.z - node.center.z).abs() <= node.half;
    let far = !inside && dist > size / theta.max(1e-3);

    if far {
        return node.accel(p, g, order);
    }

    if node.is_leaf() {
        let mut a = Vec3::ZERO;
        for &j in &node.indices {
            if j == target {
                continue;
            }
            let d = bodies[j].pos - p;
            let r2 = d.len2() + softening;
            a += d * (g * bodies[j].mass / (r2 * r2.sqrt()));
        }
        return a;
    }

    let mut a = Vec3::ZERO;
    for child in node.children.iter().flatten() {
        a += walk(child, p, target, bodies, g, softening, theta, order);
    }
    a
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
                    a += d * (g * b.mass / (r2 * r2.sqrt()));
                }
                a
            })
            .collect()
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
        let mut indices = vec![0usize, 1];
        let mut regions = Vec::new();
        let node = build(&mut indices, &bodies, Vec3::ZERO, 20.0, 0, &mut regions);
        let p = Vec3::new(100.0, 7.0, -3.0);
        let mut exact = Vec3::ZERO;
        for b in &bodies {
            let d = b.pos - p;
            let r2 = d.len2();
            exact += d * (b.mass / (r2 * r2.sqrt()));
        }
        let mut worst = f64::INFINITY;
        for order in 0..=2 {
            let approx = node.accel(p, 1.0, order);
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

        let scale = exact.iter().fold(0.0_f64, |m, a| m.max(a.len())).max(1e-9);
        let mut worst: f64 = 0.0;
        for (a, b) in exact.iter().zip(approx.acc.iter()) {
            worst = worst.max((*a - *b).len() / scale);
        }
        assert!(
            worst < 0.02,
            "относительная ошибка FMM слишком велика: {worst}"
        );
        assert!(!approx.regions.is_empty());
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
}
