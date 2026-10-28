//! Тепловая карта гравитационного поля.
//!
//! В отличие от старой версии (крупные квадраты, жёстко заданный диапазон):
//! * значения считаются в мировых координатах и подписываются адаптивно —
//!   по 2-му и 98-му перцентилям, поэтому контраст работает при любом
//!   масштабе и любой массе тел;
//! * заливка рисуется сеткой с интерполяцией цвета по вершинам (`Mesh`),
//!   а не отдельными прямоугольниками — переходы плавные;
//! * поверх заливки можно включить изолинии (marching squares);
//! * разрешение регулируется слайдером, а тяжёлый пересчёт кэшируется
//!   и распараллеливается через rayon.
//!
//! Карта рисуется на плоскости z = 0 и работает как в 2D, так и в 3D
//! (в перспективе плоскость проецируется вместе со сценой).

use eframe::egui;
use rayon::prelude::*;

use crate::physics::{Body, Vec2, Vec3};

/// Какое поле показывать.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldKind {
    /// Модуль гравитационного ускорения |a|.
    #[default]
    Acceleration,
    /// Модуль гравитационного потенциала |Φ|.
    Potential,
}

impl FieldKind {
    pub fn name(self) -> &'static str {
        match self {
            FieldKind::Acceleration => "Ускорение |a|",
            FieldKind::Potential => "Потенциал |Φ|",
        }
    }

    pub fn legend(self) -> &'static str {
        match self {
            FieldKind::Acceleration => "log₁₀ |a|",
            FieldKind::Potential => "log₁₀ |Φ|",
        }
    }

    pub const ALL: [FieldKind; 2] = [FieldKind::Acceleration, FieldKind::Potential];
}

/// Настройки и кэш тепловой карты (живут в состоянии приложения).
pub struct HeatmapState {
    /// Число ячеек по длинной стороне (16…128).
    pub res: usize,
    /// Рисовать изолинии уровней поля.
    pub contours: bool,
    pub kind: FieldKind,
    cache: Option<CachedField>,
}

impl Default for HeatmapState {
    fn default() -> Self {
        Self {
            res: 64,
            contours: true,
            kind: FieldKind::Acceleration,
            cache: None,
        }
    }
}

/// Ключ кэша: меняется при движении тел, камеры, смене поля/разрешения.
#[derive(Clone, PartialEq)]
struct Key {
    epoch: u64,
    /// Обновляем значения ~12 раз в секунду, а не каждый кадр.
    tick: u64,
    kind: FieldKind,
    res: usize,
    n: usize,
    bounds: [u64; 4],
    g: u64,
    soft: u64,
}

struct CachedField {
    key: Key,
    cols: usize,
    rows: usize,
    /// Логарифмические границы для легенды.
    lo: f64,
    hi: f64,
    /// Нормированные значения поля в [0,1], построчно.
    t: Vec<f32>,
}

/// Уровни изолиний (доли диапазона).
const LEVELS: [f32; 7] = [0.12, 0.24, 0.36, 0.48, 0.60, 0.74, 0.88];

/// Цветовая шкала: глубокая синева → бирюза → зелень → янтарь → красный.
pub fn ramp(t: f32) -> egui::Color32 {
    const STOPS: [(f32, [u8; 3]); 7] = [
        (0.00, [10, 16, 42]),
        (0.20, [20, 52, 120]),
        (0.40, [22, 112, 160]),
        (0.55, [32, 168, 145]),
        (0.70, [120, 200, 80]),
        (0.85, [245, 196, 70]),
        (1.00, [255, 96, 62]),
    ];
    let t = t.clamp(0.0, 1.0);
    for pair in STOPS.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = if (t1 - t0).abs() < 1e-9 {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            let f = f.clamp(0.0, 1.0);
            return egui::Color32::from_rgb(
                (c0[0] as f32 + (c1[0] as f32 - c0[0] as f32) * f) as u8,
                (c0[1] as f32 + (c1[1] as f32 - c0[1] as f32) * f) as u8,
                (c0[2] as f32 + (c1[2] as f32 - c0[2] as f32) * f) as u8,
            );
        }
    }
    egui::Color32::from_rgb(255, 96, 62)
}

/// Цвет вершины заливки: чем сильнее поле, тем плотнее и ярче.
fn fill_color(t: f32) -> egui::Color32 {
    let c = ramp(t);
    let alpha = (8.0 + 132.0 * t.clamp(0.0, 1.0).powf(1.35)) as u8;
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

/// Значение поля в точке (только модуль — карта «тепловая»).
fn field_value(bodies: &[Body], p: Vec3, g: f64, softening: f64, kind: FieldKind) -> f64 {
    let mut acc = Vec3::ZERO;
    let mut potential = 0.0;
    for b in bodies {
        let d = b.pos - p;
        let r2 = d.len2() + softening;
        if r2 <= softening || !r2.is_finite() {
            continue;
        }
        match kind {
            FieldKind::Acceleration => acc += d * (g * b.mass / (r2 * r2.sqrt())),
            FieldKind::Potential => potential += g * b.mass / r2.sqrt(),
        }
    }
    match kind {
        FieldKind::Acceleration => acc.len(),
        FieldKind::Potential => potential.abs(),
    }
}

/// Пересчитать сетку поля. Тяжёлый цикл распараллелен по строкам.
fn compute(
    bodies: &[Body],
    min: Vec2,
    max: Vec2,
    g: f64,
    softening: f64,
    kind: FieldKind,
    res: usize,
    key: Key,
) -> CachedField {
    let width = (max.x - min.x).abs().max(1e-9);
    let height = (max.y - min.y).abs().max(1e-9);
    let aspect = (width / height).clamp(1e-6, 1e6);
    let long = res.clamp(8, 256);
    let (cols, rows) = if aspect >= 1.0 {
        (long + 1, ((long as f64 / aspect) as usize + 1).clamp(3, 256))
    } else {
        (((long as f64 * aspect) as usize + 1).clamp(3, 256), long + 1)
    };

    let logs_rows: Vec<Vec<f32>> = (0..rows)
        .into_par_iter()
        .map(|j| {
            let y = min.y + (max.y - min.y) * (j as f64 / (rows - 1) as f64);
            (0..cols)
                .map(|i| {
                    let x = min.x + (max.x - min.x) * (i as f64 / (cols - 1) as f64);
                    let v = field_value(bodies, Vec3::new(x, y, 0.0), g, softening, kind);
                    (v.max(1e-18)).log10() as f32
                })
                .collect()
        })
        .collect();
    let logs: Vec<f32> = logs_rows.into_iter().flatten().collect();

    // Авто-контраст по перцентилям: одиночный выброс не «съедает» шкалу.
    let mut sorted = logs.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let pick = |q: f64| {
        let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    };
    let (mut lo, mut hi) = (pick(0.02) as f64, pick(0.98) as f64);
    if !(hi > lo) || (hi - lo) < 1e-4 {
        lo -= 1.0;
        hi += 1.0;
    }
    let span = (hi - lo).max(1e-9);
    let t = logs
        .iter()
        .map(|&v| ((v as f64 - lo) / span).clamp(0.0, 1.0) as f32)
        .collect();

    CachedField {
        key,
        cols,
        rows,
        lo,
        hi,
        t,
    }
}

/// Нарисовать карту. `project` переводит точку плоскости z=0 в экран.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    min: Vec2,
    max: Vec2,
    bodies: &[Body],
    g: f64,
    softening: f64,
    state: &mut HeatmapState,
    epoch: u64,
    tick: u64,
    project: &dyn Fn(Vec3) -> Option<egui::Pos2>,
) {
    if bodies.is_empty() || !(max.x > min.x) || !(max.y > min.y) {
        return;
    }
    let key = Key {
        epoch,
        tick: tick / 4,
        kind: state.kind,
        res: state.res,
        n: bodies.len(),
        bounds: [
            min.x.to_bits(),
            min.y.to_bits(),
            max.x.to_bits(),
            max.y.to_bits(),
        ],
        g: g.to_bits(),
        soft: softening.to_bits(),
    };
    let stale = state.cache.as_ref().map(|c| c.key != key).unwrap_or(true);
    if stale {
        state.cache = Some(compute(
            bodies,
            min,
            max,
            g,
            softening,
            state.kind,
            state.res,
            key,
        ));
    }
    let Some(cache) = state.cache.as_ref() else {
        return;
    };
    let contours = state.contours;

    let (cols, rows) = (cache.cols, cache.rows);
    let (lo, hi) = (cache.lo, cache.hi);

    // Проекция узлов сетки один раз — используется и заливкой, и изолиниями.
    let mut screen: Vec<Option<egui::Pos2>> = Vec::with_capacity(cols * rows);
    let mut mesh = egui::epaint::Mesh::default();
    let mut index = vec![u32::MAX; cols * rows];
    for j in 0..rows {
        let y = min.y + (max.y - min.y) * (j as f64 / (rows - 1) as f64);
        for i in 0..cols {
            let x = min.x + (max.x - min.x) * (i as f64 / (cols - 1) as f64);
            let p = project(Vec3::new(x, y, 0.0));
            let p = p.map(|p| {
                egui::pos2(
                    p.x.clamp(-1.0e5, 1.0e5),
                    p.y.clamp(-1.0e5, 1.0e5),
                )
            });
            screen.push(p);
            if let Some(pos) = p {
                let k = j * cols + i;
                index[k] = mesh.vertices.len() as u32;
                mesh.vertices.push(egui::epaint::Vertex {
                    pos,
                    uv: egui::epaint::WHITE_UV,
                    color: fill_color(cache.t[k]),
                });
            }
        }
    }
    // Треугольники только там, где все четыре угла попали в кадр.
    for j in 0..rows.saturating_sub(1) {
        for i in 0..cols.saturating_sub(1) {
            let a = index[j * cols + i];
            let b = index[j * cols + i + 1];
            let c = index[(j + 1) * cols + i + 1];
            let d = index[(j + 1) * cols + i];
            if a != u32::MAX && b != u32::MAX && c != u32::MAX && d != u32::MAX {
                mesh.add_triangle(a, b, c);
                mesh.add_triangle(a, c, d);
            }
        }
    }
    if !mesh.vertices.is_empty() && !mesh.indices.is_empty() {
        painter.add(egui::Shape::mesh(mesh));
    }

    // Изолинии по нормированному полю в экранных координатах.
    if contours {
        for level in LEVELS {
            let color = ramp(level).gamma_multiply(0.62);
            for [a, b] in marching_squares(&cache.t, cols, rows, &screen, level) {
                painter.line_segment([a, b], egui::Stroke::new(1.0, color));
            }
        }
    }

    draw_legend(painter, rect, lo, hi, state.kind);
}

/// Marching squares: отрезки уровня `level` по сетке `cols × rows`.
fn marching_squares(
    t: &[f32],
    cols: usize,
    rows: usize,
    pos: &[Option<egui::Pos2>],
    level: f32,
) -> Vec<[egui::Pos2; 2]> {
    let mut out = Vec::new();
    for j in 0..rows.saturating_sub(1) {
        for i in 0..cols.saturating_sub(1) {
            let ids = [j * cols + i, j * cols + i + 1, (j + 1) * cols + i + 1, (j + 1) * cols + i];
            let (Some(p0), Some(p1), Some(p2), Some(p3)) =
                (pos[ids[0]], pos[ids[1]], pos[ids[2]], pos[ids[3]])
            else {
                continue;
            };
            let corners = [p0, p1, p2, p3];
            let v = [t[ids[0]], t[ids[1]], t[ids[2]], t[ids[3]]];
            let mut hits: Vec<egui::Pos2> = Vec::with_capacity(4);
            for e in 0..4 {
                let (va, vb) = (v[e], v[(e + 1) % 4]);
                if (va - level) * (vb - level) < 0.0 {
                    let f = ((level - va) / (vb - va)).clamp(0.0, 1.0);
                    let pa = corners[e];
                    let pb = corners[(e + 1) % 4];
                    hits.push(egui::pos2(
                        pa.x + (pb.x - pa.x) * f,
                        pa.y + (pb.y - pa.y) * f,
                    ));
                }
            }
            match hits.len() {
                2 => out.push([hits[0], hits[1]]),
                4 => {
                    out.push([hits[0], hits[1]]);
                    out.push([hits[2], hits[3]]);
                }
                _ => {}
            }
        }
    }
    out
}

/// Компактная легенда: градиентная шкала и логарифмические границы.
fn draw_legend(
    painter: &egui::Painter,
    rect: egui::Rect,
    lo: f64,
    hi: f64,
    kind: FieldKind,
) {
    let (w, h) = (132.0_f32, 8.0_f32);
    let origin = egui::pos2(rect.left() + 12.0, rect.bottom() - 48.0);
    let steps = 32;
    for k in 0..steps {
        let t0 = k as f32 / steps as f32;
        let t1 = (k + 1) as f32 / steps as f32;
        let r = egui::Rect::from_min_max(
            egui::pos2(origin.x + t0 * w, origin.y),
            egui::pos2(origin.x + t1 * w + 0.5, origin.y + h),
        );
        painter.rect_filled(r, 0.0, ramp(0.5 * (t0 + t1)).gamma_multiply(0.9));
    }
    painter.rect_stroke(
        egui::Rect::from_min_size(origin, egui::vec2(w, h)),
        1.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(90)),
        egui::StrokeKind::Middle,
    );
    let label = egui::Color32::from_rgba_unmultiplied(175, 195, 220, 210);
    painter.text(
        origin + egui::vec2(0.0, -3.0),
        egui::Align2::LEFT_BOTTOM,
        kind.legend(),
        egui::FontId::proportional(10.0),
        label,
    );
    painter.text(
        origin + egui::vec2(0.0, h + 2.0),
        egui::Align2::LEFT_TOP,
        format!("{lo:.1}"),
        egui::FontId::monospace(9.0),
        label,
    );
    painter.text(
        origin + egui::vec2(w, h + 2.0),
        egui::Align2::RIGHT_TOP,
        format!("{hi:.1}"),
        egui::FontId::monospace(9.0),
        label,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair_bodies() -> Vec<Body> {
        vec![
            Body {
                id: 1,
                mass: 1000.0,
                pos: Vec3::ZERO,
                vel: Vec3::ZERO,
                radius: 10.0,
                color: [255, 255, 255],
            },
            Body {
                id: 2,
                mass: 1.0,
                pos: Vec3::new(100.0, 0.0, 0.0),
                vel: Vec3::ZERO,
                radius: 2.0,
                color: [200, 200, 255],
            },
        ]
    }

    #[test]
    fn field_falls_off_with_distance() {
        let bodies = pair_bodies();
        let near = field_value(&bodies, Vec3::new(10.0, 0.0, 0.0), 1.0, 1.0, FieldKind::Acceleration);
        let far = field_value(&bodies, Vec3::new(1000.0, 0.0, 0.0), 1.0, 1.0, FieldKind::Acceleration);
        assert!(near > far);
        let p_near = field_value(&bodies, Vec3::new(10.0, 0.0, 0.0), 1.0, 1.0, FieldKind::Potential);
        let p_far = field_value(&bodies, Vec3::new(1000.0, 0.0, 0.0), 1.0, 1.0, FieldKind::Potential);
        assert!(p_near > p_far);
    }

    #[test]
    fn ramp_is_bounded_and_monotonic_in_red() {
        let a = ramp(0.0);
        let b = ramp(1.0);
        assert_eq!((a.r(), a.g(), a.b()), (10, 16, 42));
        assert_eq!((b.r(), b.g(), b.b()), (255, 96, 62));
        assert!(ramp(0.5).g() > a.g());
    }

    #[test]
    fn marching_squares_finds_level_between_nodes() {
        // 2×2: значения 0,0 / 1,1 — линия уровня 0.5 идёт по середине.
        let t = vec![0.0, 0.0, 1.0, 1.0];
        let pos = vec![
            Some(egui::pos2(0.0, 0.0)),
            Some(egui::pos2(10.0, 0.0)),
            Some(egui::pos2(10.0, 10.0)),
            Some(egui::pos2(0.0, 10.0)),
        ];
        let segs = marching_squares(&t, 2, 2, &pos, 0.5);
        assert_eq!(segs.len(), 1);
        let [a, b] = segs[0];
        assert!((a.y - 5.0).abs() < 1e-6 && (b.y - 5.0).abs() < 1e-6);
    }
}
