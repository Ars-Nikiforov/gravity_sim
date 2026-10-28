//! Оверлеи небесной механики: точки Лагранжа L1–L5, барицентр пары,
//! сфера Хилла.
//!
//! Всё считается для двух самых массивных тел системы — это естественная
//! «главная пара» (звезда + планета, две звезды, планета + луна). Для
//! коллинеарных точек решается точное уравнение равновесия в круговой
//! ограниченной задаче трёх тел (CR3BP), а не грубая формула первого
//! порядка; L4/L5 строятся как вершины равносторонних треугольников в
//! плоскости относительного движения.

use eframe::egui;

use crate::physics::{Body, Vec3};

/// Две самые массивные тела и геометрия их относительного движения.
#[derive(Clone, Copy, Debug)]
pub struct Pair {
    pub id1: u64,
    pub m1: f64,
    pub p1: Vec3,
    pub id2: u64,
    pub m2: f64,
    pub p2: Vec3,
    /// Нормаль плоскости относительного движения.
    pub normal: Vec3,
    pub separation: f64,
    pub barycenter: Vec3,
}

impl Pair {
    /// Доля массы вторичного тела от суммарной (0…0.5).
    pub fn mu(&self) -> f64 {
        let total = self.m1 + self.m2;
        if total > 0.0 { self.m2 / total } else { 0.0 }
    }

    /// Единичное направление от первичного тела ко вторичному.
    pub fn direction(&self) -> Vec3 {
        if self.separation > 1e-12 {
            (self.p2 - self.p1) * (1.0 / self.separation)
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        }
    }
}

/// Положения точек Лагранжа и радиусы Хилла.
#[derive(Clone, Copy, Debug)]
pub struct Lagrange {
    pub l1: Vec3,
    pub l2: Vec3,
    pub l3: Vec3,
    pub l4: Vec3,
    pub l5: Vec3,
    /// Радиус Хилла вторичного тела (сферы его гравитационного влияния).
    pub hill2: f64,
    pub mu: f64,
}

/// Найти две самые массивные ненулевые массы.
pub fn dominant_pair(bodies: &[Body]) -> Option<Pair> {
    let mut idx: Vec<usize> = (0..bodies.len())
        .filter(|&i| bodies[i].mass.is_finite() && bodies[i].mass > 0.0)
        .collect();
    if idx.len() < 2 {
        return None;
    }
    idx.sort_by(|&i, &j| bodies[j].mass.total_cmp(&bodies[i].mass));
    let (i1, i2) = (idx[0], idx[1]);
    let b1 = &bodies[i1];
    let b2 = &bodies[i2];

    let rel = b2.pos - b1.pos;
    let separation = rel.len();
    if !separation.is_finite() || separation < 1e-9 {
        return None;
    }
    let mut normal = rel.cross(b2.vel - b1.vel).normalized();
    if normal.len() < 1e-9 {
        normal = Vec3::new(0.0, 0.0, 1.0);
    }
    let barycenter = (b1.pos * b1.mass + b2.pos * b2.mass) * (1.0 / (b1.mass + b2.mass));

    Some(Pair {
        id1: b1.id,
        m1: b1.mass,
        p1: b1.pos,
        id2: b2.id,
        m2: b2.mass,
        p2: b2.pos,
        normal,
        separation,
        barycenter,
    })
}

/// Левая часть уравнения равновесия на оси (нормировка: расстояние = 1,
/// сумма масс = 1, ω = 1). Начало координат — первичное тело, `mu` — доля
/// вторичного; центробежная сила считается вокруг барицентра.
fn collinear_eq(xi: f64, mu: f64) -> f64 {
    let d1 = xi.abs().max(1e-12);
    let d2 = (xi - 1.0).abs().max(1e-12);
    (xi - mu) - (1.0 - mu) * xi / (d1 * d1 * d1) - mu * (xi - 1.0) / (d2 * d2 * d2)
}

/// Корень уравнения на отрезке; при необходимости отрезок расширяется
/// наружу (для L3 — в отрицательную сторону).
fn solve_collinear(mu: f64, mut lo: f64, mut hi: f64, expand: f64) -> f64 {
    let mut guard = 0;
    loop {
        let flo = collinear_eq(lo, mu);
        let fhi = collinear_eq(hi, mu);
        if flo * fhi <= 0.0 || guard >= 64 {
            break;
        }
        if hi > 0.0 {
            hi += expand;
        } else {
            lo -= expand;
        }
        guard += 1;
    }
    let mut flo = collinear_eq(lo, mu);
    for _ in 0..90 {
        let mid = 0.5 * (lo + hi);
        let fm = collinear_eq(mid, mu);
        if fm == 0.0 {
            return mid;
        }
        if flo * fm <= 0.0 {
            hi = mid;
        } else {
            lo = mid;
            flo = fm;
        }
    }
    0.5 * (lo + hi)
}

/// Точки Лагранжа для пары (приближение круговой ограниченной задачи).
pub fn lagrange(pair: &Pair) -> Lagrange {
    let mu = pair.mu().clamp(1e-9, 1.0 - 1e-9);
    let dir = pair.direction();
    let r = pair.separation;
    let at = |xi: f64| pair.p1 + dir * (xi * r);

    // L1 — между телами, L2 — за вторичным, L3 — с противоположной стороны.
    let xi1 = solve_collinear(mu, 1e-6, 1.0 - 1e-6, 0.1);
    let xi2 = solve_collinear(mu, 1.0 + 1e-6, 2.0, 0.5);
    let xi3 = solve_collinear(mu, -4.0, -1e-6, 1.0);

    // L4/L5 — равносторонние треугольники в плоскости движения.
    let rel = pair.p2 - pair.p1;
    let n = pair.normal;
    let rotate = |v: Vec3, ang: f64| -> Vec3 {
        v * ang.cos() + n.cross(v) * ang.sin()
    };
    let l4 = pair.p1 + rotate(rel, std::f64::consts::FRAC_PI_3);
    let l5 = pair.p1 + rotate(rel, -std::f64::consts::FRAC_PI_3);

    let hill2 = r * (mu / 3.0).cbrt();

    Lagrange {
        l1: at(xi1),
        l2: at(xi2),
        l3: at(xi3),
        l4,
        l5,
        hill2,
        mu,
    }
}

/// Ортонормированный базис плоскости с нормалью `n`.
fn plane_basis(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n.z.abs() < 0.9 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(1.0, 0.0, 0.0)
    };
    let u = helper.cross(n).normalized();
    (u, n.cross(u).normalized())
}

fn stroke_runs(painter: &egui::Painter, pts: &[Option<egui::Pos2>], stroke: egui::Stroke) {
    let mut run: Vec<egui::Pos2> = Vec::new();
    for p in pts {
        match p {
            Some(p) => run.push(*p),
            None => {
                if run.len() >= 2 {
                    painter.add(egui::Shape::line(std::mem::take(&mut run), stroke));
                } else {
                    run.clear();
                }
            }
        }
    }
    if run.len() >= 2 {
        painter.add(egui::Shape::line(run, stroke));
    }
}

/// Нарисовать оверлей. Возвращает пару, для которой он построен.
pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    project: &dyn Fn(Vec3) -> Option<egui::Pos2>,
    bodies: &[Body],
    show_points: bool,
    show_hill: bool,
) -> Option<Pair> {
    let pair = dominant_pair(bodies)?;
    let lag = lagrange(&pair);
    let visible = rect.expand(80.0);

    // Линия между телами — визуальная ось системы.
    if let (Some(a), Some(b)) = (project(pair.p1), project(pair.p2)) {
        painter.line_segment(
            [a, b],
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(130, 160, 210, 55),
            ),
        );
    }

    // Барицентр пары: перекрестие в кружке.
    if let Some(c) = project(pair.barycenter) {
        let color = egui::Color32::from_rgba_unmultiplied(200, 210, 255, 210);
        painter.circle_stroke(c, 6.0, egui::Stroke::new(1.2, color));
        painter.line_segment(
            [c + egui::vec2(-9.0, 0.0), c + egui::vec2(9.0, 0.0)],
            egui::Stroke::new(1.0, color),
        );
        painter.line_segment(
            [c + egui::vec2(0.0, -9.0), c + egui::vec2(0.0, 9.0)],
            egui::Stroke::new(1.0, color),
        );
        painter.text(
            c + egui::vec2(11.0, -11.0),
            egui::Align2::LEFT_BOTTOM,
            "B",
            egui::FontId::proportional(11.0),
            color,
        );
    }

    // Сфера Хилла вторичного тела — окружность в плоскости пары.
    if show_hill && lag.hill2 > 0.0 && lag.hill2.is_finite() {
        let (u, v) = plane_basis(pair.normal);
        let n = 96;
        let mut pts = Vec::with_capacity(n + 1);
        for k in 0..=n {
            let a = std::f64::consts::TAU * k as f64 / n as f64;
            let p = pair.p2 + u * (a.cos() * lag.hill2) + v * (a.sin() * lag.hill2);
            pts.push(project(p));
        }
        stroke_runs(
            painter,
            &pts,
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(170, 150, 255, 105),
            ),
        );
        // Подпись у верхней точки окружности.
        if let Some(top) = project(pair.p2 + v * lag.hill2) {
            painter.text(
                top + egui::vec2(4.0, -2.0),
                egui::Align2::LEFT_BOTTOM,
                "R Хилла",
                egui::FontId::proportional(9.0),
                egui::Color32::from_rgba_unmultiplied(170, 150, 255, 170),
            );
        }
    }

    // Коллинеарные точки — янтарные, треугольные — бирюзовые.
    if show_points {
        let collinear = egui::Color32::from_rgba_unmultiplied(255, 190, 90, 225);
        let triangular = egui::Color32::from_rgba_unmultiplied(120, 235, 205, 225);
        for (label, p, color) in [
            ("L1", lag.l1, collinear),
            ("L2", lag.l2, collinear),
            ("L3", lag.l3, collinear),
            ("L4", lag.l4, triangular),
            ("L5", lag.l5, triangular),
        ] {
            let Some(sp) = project(p) else { continue };
            if !visible.contains(sp) {
                continue;
            }
            painter.circle_filled(sp, 2.2, color);
            painter.circle_stroke(sp, 5.5, egui::Stroke::new(1.0, color.gamma_multiply(0.75)));
            painter.text(
                sp + egui::vec2(7.0, -7.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                egui::FontId::proportional(11.0),
                color,
            );
        }
    }

    Some(pair)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn circular_pair(m1: f64, m2: f64, r: f64) -> Vec<Body> {
        // Круговая орбита: скорость вторичного относительно первичного.
        let g = 1.0;
        let v = (g * (m1 + m2) / r).sqrt();
        vec![
            Body {
                id: 1,
                mass: m1,
                pos: Vec3::ZERO,
                vel: Vec3::ZERO,
                radius: 5.0,
                color: [255, 255, 255],
            },
            Body {
                id: 2,
                mass: m2,
                pos: Vec3::new(r, 0.0, 0.0),
                vel: Vec3::new(0.0, v, 0.0),
                radius: 2.0,
                color: [200, 220, 255],
            },
        ]
    }

    #[test]
    fn picks_two_heaviest_bodies() {
        let mut bodies = circular_pair(1000.0, 1.0, 100.0);
        bodies.push(Body {
            id: 3,
            mass: 0.1,
            pos: Vec3::new(200.0, 100.0, 0.0),
            vel: Vec3::ZERO,
            radius: 1.0,
            color: [255, 0, 0],
        });
        let pair = dominant_pair(&bodies).expect("пара есть");
        assert_eq!(pair.id1, 1);
        assert_eq!(pair.id2, 2);
        assert!((pair.mu() - 1.0 / 1001.0).abs() < 1e-12);
    }

    #[test]
    fn collinear_points_are_equilibria() {
        for mu in [1e-6, 1e-3, 0.01, 0.1, 0.4] {
            let xi1 = solve_collinear(mu, 1e-9, 1.0 - 1e-9, 0.1);
            let xi2 = solve_collinear(mu, 1.0 + 1e-9, 2.0, 0.5);
            let xi3 = solve_collinear(mu, -4.0, -1e-9, 1.0);
            assert!((0.0..1.0).contains(&xi1), "L1 вне отрезка: {xi1}");
            assert!(xi2 > 1.0, "L2 должен быть за вторичным: {xi2}");
            assert!(xi3 < 0.0, "L3 должен быть с другой стороны: {xi3}");
            assert!(collinear_eq(xi1, mu).abs() < 1e-9);
            assert!(collinear_eq(xi2, mu).abs() < 1e-9);
            assert!(collinear_eq(xi3, mu).abs() < 1e-9);
        }
    }

    #[test]
    fn triangular_points_are_equilateral() {
        let bodies = circular_pair(1000.0, 1.0, 100.0);
        let pair = dominant_pair(&bodies).unwrap();
        let lag = lagrange(&pair);
        let r = pair.separation;
        for p in [lag.l4, lag.l5] {
            let d1 = (p - pair.p1).len();
            let d2 = (p - pair.p2).len();
            assert!((d1 - r).abs() < 1e-9, "L4/L5 не на расстоянии r: {d1}");
            assert!((d2 - r).abs() < 1e-9, "L4/L5 не на расстоянии r: {d2}");
            // Точки лежат в плоскости движения (z = 0 для этой пары).
            assert!(p.z.abs() < 1e-9);
        }
        assert!((lag.hill2 / r - (lag.mu / 3.0).cbrt()).abs() < 1e-12);
    }
}
