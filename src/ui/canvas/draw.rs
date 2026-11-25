//! Статичные помощники отрисовки: сетка плоскости, «нити» глубины,
//! каркас областей разбиения, превью орбиты, цвета поля.

use eframe::egui;

use crate::physics::{Body, OrbitPlan, Region, Vec3};
use crate::ui::canvas::camera::Camera3D;

/// Гравитационное ускорение в точке от всех тел (для визуализаций).
pub(super) fn accel_at(bodies: &[Body], p: Vec3, g: f64, softening: f64) -> Vec3 {
    let mut a = Vec3::ZERO;
    for b in bodies {
        let d = b.pos - p;
        let r2 = d.len2() + softening;
        if r2 <= softening || !r2.is_finite() {
            continue;
        }
        a += d * (g * b.mass / (r2 * r2.sqrt()));
    }
    a
}

/// Цвет по логарифму модуля ускорения: синий → зелёный → жёлтый → красный.
pub(super) fn heat_color(t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let (r, gr, b) = if t < 0.5 {
        let u = t / 0.5;
        (
            (30.0 + 225.0 * u * u) as u8,
            (60.0 + 120.0 * u) as u8,
            (200.0 - 140.0 * u) as u8,
        )
    } else {
        let u = (t - 0.5) / 0.5;
        (255, (180.0 - 130.0 * u) as u8, (60.0 - 60.0 * u) as u8)
    };
    egui::Color32::from_rgb(r, gr, b)
}

/// Нормировка поля в [0,1] по лог-шкале.
pub(super) fn field_t(mag: f64) -> f32 {
    let lo = -3.5_f64;
    let hi = 0.5_f64;
    let v = (mag.max(1e-9).log10() - lo) / (hi - lo);
    v.clamp(0.0, 1.0) as f32
}

pub(super) fn lighten(c: egui::Color32, k: f32) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c.r() as f32 * k).min(255.0) as u8,
        (c.g() as f32 * k).min(255.0) as u8,
        (c.b() as f32 * k).min(255.0) as u8,
    )
}


/// Каркас областей мультипольного разбиения (12 рёбер каждого куба).
pub(super) fn draw_regions(
    painter: &egui::Painter,
    project: &dyn Fn(Vec3) -> Option<egui::Pos2>,
    regions: &[Region],
) {
    for region in regions {
        if region.bodies == 0 {
            continue;
        }
        let h = region.half;
        let c = region.center;
        let corners: [Vec3; 8] = std::array::from_fn(|i| {
            Vec3::new(
                c.x + if i & 1 != 0 { h } else { -h },
                c.y + if i & 2 != 0 { h } else { -h },
                c.z + if i & 4 != 0 { h } else { -h },
            )
        });
        let pts: [Option<egui::Pos2>; 8] = std::array::from_fn(|i| project(corners[i]));
        let mass_term = region.mass.max(1e-9).log10() * 5.0;
        let alpha =
            (26.0 + mass_term + region.bodies.min(30) as f64 * 2.0).clamp(22.0, 110.0) as u8;
        let stroke = egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(120, 170, 255, alpha),
        );
        for i in 0..8usize {
            for bit in [1usize, 2, 4] {
                if i & bit == 0 {
                    if let (Some(a), Some(b)) = (pts[i], pts[i | bit]) {
                        painter.line_segment([a, b], stroke);
                    }
                }
            }
        }
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

/// Кольцо будущей орбиты и точка вставки.
pub(super) fn draw_orbit_preview(
    painter: &egui::Painter,
    project: &dyn Fn(Vec3) -> Option<egui::Pos2>,
    center: Vec3,
    plan: &OrbitPlan,
) {
    let (u, v) = plane_basis(plan.normal);
    let n = 96;
    let mut pts: Vec<Option<egui::Pos2>> = Vec::with_capacity(n + 1);
    for k in 0..=n {
        let a = std::f64::consts::TAU * k as f64 / n as f64;
        pts.push(project(
            center + u * (plan.radius * a.cos()) + v * (plan.radius * a.sin()),
        ));
    }
    let color = egui::Color32::from_rgba_unmultiplied(120, 235, 205, 210);
    for w in pts.windows(2) {
        if let (Some(a), Some(b)) = (w[0], w[1]) {
            painter.line_segment([a, b], egui::Stroke::new(1.5, color));
        }
    }
    if let (Some(c), Some(p)) = (project(center), project(plan.pos)) {
        painter.line_segment([c, p], egui::Stroke::new(1.0, color.gamma_multiply(0.55)));
        painter.circle_filled(p, 3.6, color);
    }
}


/// «Красивый» шаг сетки: 1/2/5 × 10^n.
fn nice_step(raw: f64) -> f64 {
    let raw = raw.max(1e-6);
    let exp = raw.log10().floor();
    let base = 10f64.powf(exp);
    let m = raw / base;
    let mant = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    (mant * base).max(1e-3)
}

/// Сетка на плоскости z = 0 — главный ориентир глубины в 3D.
pub(super) fn draw_plane_grid(painter: &egui::Painter, rect: egui::Rect, cam3: Camera3D) {
    let visible = cam3.distance * (cam3.fov_y as f64 * 0.5).tan();
    let extent = (visible * 1.9).clamp(100.0, 1.0e7);
    let step = nice_step(visible / 6.0);
    let center = cam3.target;
    let cx = (center.x / step).round() * step;
    let cy = (center.y / step).round() * step;
    let n = ((extent / step).ceil() as i32).clamp(2, 80);

    let minor = egui::Color32::from_rgba_unmultiplied(70, 100, 150, 42);
    let axis_x = egui::Color32::from_rgba_unmultiplied(150, 90, 90, 70);
    let axis_y = egui::Color32::from_rgba_unmultiplied(90, 150, 90, 70);

    for i in -n..=n {
        let x = cx + i as f64 * step;
        let a = Vec3::new(x, cy - extent, 0.0);
        let b = Vec3::new(x, cy + extent, 0.0);
        if let (Some(pa), Some(pb)) = (cam3.project(rect, a), cam3.project(rect, b)) {
            let color = if i == 0 { axis_y } else { minor };
            painter.line_segment([pa.pos, pb.pos], egui::Stroke::new(1.0, color));
        }

        let y = cy + i as f64 * step;
        let a = Vec3::new(cx - extent, y, 0.0);
        let b = Vec3::new(cx + extent, y, 0.0);
        if let (Some(pa), Some(pb)) = (cam3.project(rect, a), cam3.project(rect, b)) {
            let color = if i == 0 { axis_x } else { minor };
            painter.line_segment([pa.pos, pb.pos], egui::Stroke::new(1.0, color));
        }
    }
}

/// Тонкие «нити» от каждого тела вниз к плоскости z = 0 — сразу видно
/// высоту и объём сцены.
pub(super) fn draw_depth_drops(painter: &egui::Painter, rect: egui::Rect, cam3: Camera3D, bodies: &[Body]) {
    let line = egui::Color32::from_rgba_unmultiplied(120, 160, 220, 38);
    let dot = egui::Color32::from_rgba_unmultiplied(120, 160, 220, 90);
    for b in bodies {
        if b.pos.z.abs() < 0.5 {
            continue;
        }
        let foot = Vec3::new(b.pos.x, b.pos.y, 0.0);
        if let (Some(top), Some(bottom)) = (cam3.project(rect, b.pos), cam3.project(rect, foot)) {
            painter.line_segment([top.pos, bottom.pos], egui::Stroke::new(1.0, line));
            painter.circle_filled(bottom.pos, 1.4, dot);
        }
    }
}

