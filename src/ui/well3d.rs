//! 3D-визуализация «гравитационный колодец» в углу холста.
//!
//! Классический лист резины: сетка прогибается под телами — чем ближе
//! к массивному телу, тем глубже впадина. Проекция аксонометрическая,
//! сетка медленно вращается; её можно крутить мышью (drag).
//!
//! Важно: мини-вид не зависит от зума основной камеры. Область мира
//! выбирается по самим телам (устойчивый радиус), поэтому масштабирование
//! холста больше не ломает картинку.

use eframe::egui;

use super::canvas::Projected;
use crate::app::WellState;
use crate::physics::{Body, Vec2, Vec3};

/// Размер мини-вью по умолчанию.
const SIZE: egui::Vec2 = egui::Vec2::new(250.0, 180.0);
/// Плотность сетки (N×N ячеек).
const MESH: usize = 16;
/// Плотность сетки «резины» в основной сцене (N×N ячеек).
const SCENE_MESH: usize = 22;
/// Порог числа тел, после которого сценовая сетка прореживается.
const SCENE_MESH_DENSE: usize = 500;
/// Плотность сетки при большом числе тел.
const SCENE_MESH_DENSE_N: usize = 14;

/// Поле глубин «гравитационного колодца»: область мира и нормировка,
/// общие для мини-вида и сценовой сетки.
pub struct WellField {
    /// Центр области (барицентр тел по xy).
    pub center: Vec2,
    /// Половина стороны квадратной области мира.
    pub half: f64,
    /// Смягчение поля (масштаб тел).
    soft: f64,
    /// Нормировочный множитель: z = Σ m/r · norm.
    norm: f64,
    /// Потолок глубины (мировые единицы).
    max_depth: f64,
}

impl WellField {
    /// Поле по телам; глубина под самым массивным телом ≈ `dip_fraction`
    /// от радиуса области (90-й перцентиль расстояний).
    pub fn new(bodies: &[Body], dip_fraction: f64) -> Self {
        // Центр — барицентр тел (только xy).
        let (mut m_sum, mut cx, mut cy) = (0.0_f64, 0.0_f64, 0.0_f64);
        for b in bodies {
            let m = if b.mass.is_finite() {
                b.mass.max(1e-9)
            } else {
                1e-9
            };
            m_sum += m;
            cx += b.pos.x * m;
            cy += b.pos.y * m;
        }
        let center = if m_sum > 1e-12 && cx.is_finite() && cy.is_finite() {
            Vec2::new(cx / m_sum, cy / m_sum)
        } else {
            Vec2::ZERO
        };

        // Устойчивый радиус: 90-й перцентиль расстояний — одиночные выбросы
        // не раздувают область до бесконечности.
        let mut distances: Vec<f64> = bodies
            .iter()
            .map(|b| (b.pos.xy() - center).len() + b.radius)
            .filter(|d| d.is_finite())
            .collect();
        distances.sort_by(|a, b| a.total_cmp(b));
        let radius = if distances.is_empty() {
            400.0
        } else {
            let idx = ((distances.len() as f64 * 0.9) as usize).min(distances.len() - 1);
            distances[idx].clamp(80.0, 1.0e6)
        };

        let soft = (radius * 0.06).max(1e-6);
        let heaviest = bodies
            .iter()
            .map(|b| if b.mass.is_finite() { b.mass } else { 0.0 })
            .fold(0.0_f64, f64::max)
            .max(1e-9);
        let dip = radius * dip_fraction;
        Self {
            center,
            half: radius * 1.15,
            soft,
            norm: dip * soft / heaviest,
            max_depth: dip * 1.2,
        }
    }

    /// Глубина прогиба в точке (мировые единицы, вниз от плоскости z = 0).
    pub fn depth(&self, bodies: &[Body], x: f64, y: f64) -> f64 {
        let mut d = 0.0;
        for b in bodies {
            let dx = x - b.pos.x;
            let dy = y - b.pos.y;
            let r2 = dx * dx + dy * dy + self.soft * self.soft;
            if r2 > 1e-12 && r2.is_finite() {
                d += b.mass / r2.sqrt();
            }
        }
        let z = d * self.norm;
        if z.is_finite() {
            z.clamp(0.0, self.max_depth)
        } else {
            0.0
        }
    }

    /// Цвет линии по глубине: глубже впадина — ярче.
    fn line_color(&self, depth: f64) -> egui::Color32 {
        let t = (depth / self.max_depth).clamp(0.0, 1.0) as f32;
        let alpha = 50.0 + t * 150.0;
        egui::Color32::from_rgba_unmultiplied(
            (70.0 + t * 120.0) as u8,
            (130.0 + t * 60.0) as u8,
            230,
            alpha as u8,
        )
    }
}

/// «Резина» в основной 3D-сцене: сетка на плоскости z = 0, прогнутая вниз
/// под телами, спроецированная камерой холста. Глубина под самым массивным
/// телом — около 30% радиуса области.
pub fn draw_scene(
    painter: &egui::Painter,
    project: &dyn Fn(Vec3) -> Option<Projected>,
    bodies: &[Body],
) {
    if bodies.is_empty() {
        return;
    }
    let field = WellField::new(bodies, 0.30);
    let mesh = if bodies.len() > SCENE_MESH_DENSE {
        SCENE_MESH_DENSE_N
    } else {
        SCENE_MESH
    }
    .max(SCENE_MESH_DENSE_N);

    // Глубины всех узлов сетки — один проход по телам на узел.
    let mut depths = vec![0.0_f64; (mesh + 1) * (mesh + 1)];
    let world = |i: usize, len: f64| -> f64 { (i as f64 / mesh as f64 - 0.5) * 2.0 * len };
    for gy in 0..=mesh {
        for gx in 0..=mesh {
            let ox = world(gx, field.half);
            let oy = world(gy, field.half);
            depths[gy * (mesh + 1) + gx] =
                field.depth(bodies, field.center.x + ox, field.center.y + oy);
        }
    }
    let point = |gx: usize, gy: usize| -> Option<Projected> {
        let z = depths[gy * (mesh + 1) + gx];
        project(Vec3::new(
            field.center.x + world(gx, field.half),
            field.center.y + world(gy, field.half),
            -z,
        ))
    };

    // Полилинии по строкам и столбцам; сегменты с непроектированными
    // вершинами пропускаются, линия рвётся на части.
    for gy in 0..=mesh {
        for gx in 0..mesh {
            if let (Some(a), Some(b)) = (point(gx, gy), point(gx + 1, gy)) {
                let d = (depths[gy * (mesh + 1) + gx] + depths[gy * (mesh + 1) + gx + 1]) * 0.5;
                painter.line_segment([a.pos, b.pos], egui::Stroke::new(1.0, field.line_color(d)));
            }
        }
    }
    for gx in 0..=mesh {
        for gy in 0..mesh {
            if let (Some(a), Some(b)) = (point(gx, gy), point(gx, gy + 1)) {
                let d = (depths[gy * (mesh + 1) + gx]
                    + depths[(gy + 1) * (mesh + 1) + gx])
                    * 0.5;
                painter.line_segment([a.pos, b.pos], egui::Stroke::new(1.0, field.line_color(d)));
            }
        }
    }
}

/// Нарисовать колодец и вернуть его прямоугольник (чтобы основной холст
/// не обрабатывал клики по этой области).
pub fn draw(
    ui: &egui::Ui,
    painter: &egui::Painter,
    canvas_rect: egui::Rect,
    bodies: &[Body],
    state: &mut WellState,
) -> egui::Rect {
    let margin = 10.0;
    // Не вылезаем за холст при маленьком окне — мини-вид сжимается.
    let max_w = (canvas_rect.width() - margin * 2.0).max(60.0);
    let max_h = (canvas_rect.height() - margin * 2.0).max(44.0);
    let size = egui::vec2(SIZE.x.min(max_w), SIZE.y.min(max_h));
    let rect = egui::Rect::from_min_max(
        canvas_rect.right_bottom() - size - egui::vec2(margin, margin),
        canvas_rect.right_bottom() - egui::vec2(margin, margin),
    );

    // Взаимодействие: drag вращает сетку.
    let resp = ui.interact(
        rect,
        egui::Id::unique("well3d"),
        egui::Sense::click_and_drag(),
    );
    let now = ui.input(|i| i.time);
    if resp.dragged() {
        state.yaw += resp.drag_delta().x * 0.012;
        state.last_drag = now;
    } else if now - state.last_drag > 3.0 {
        // Авто-вращение после трёх секунд бездействия.
        state.yaw += ui.input(|i| i.unstable_dt).min(0.1) * 0.25;
    }

    // Полупрозрачная подложка и рамка.
    painter.rect_filled(
        rect,
        5.0,
        egui::Color32::from_rgba_unmultiplied(8, 10, 22, 210),
    );
    painter.rect_stroke(
        rect,
        5.0,
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(90, 120, 200, 120),
        ),
        egui::StrokeKind::Middle,
    );

    // Центр мини-вида — барицентр тел (только xy).
    let (mut m_sum, mut cx, mut cy) = (0.0_f64, 0.0_f64, 0.0_f64);
    for b in bodies {
        let m = if b.mass.is_finite() {
            b.mass.max(1e-9)
        } else {
            1e-9
        };
        m_sum += m;
        cx += b.pos.x * m;
        cy += b.pos.y * m;
    }
    let center_world = if m_sum > 1e-12 && cx.is_finite() && cy.is_finite() {
        Vec2::new(cx / m_sum, cy / m_sum)
    } else {
        Vec2::ZERO
    };

    // Устойчивый радиус: 90-й перцентиль расстояний — одиночные выбросы
    // не раздувают область до бесконечности.
    let mut distances: Vec<f64> = bodies
        .iter()
        .map(|b| (b.pos.xy() - center_world).len() + b.radius)
        .filter(|d| d.is_finite())
        .collect();
    distances.sort_by(|a, b| a.total_cmp(b));
    let radius = if distances.is_empty() {
        400.0
    } else {
        let idx = ((distances.len() as f64 * 0.9) as usize).min(distances.len() - 1);
        distances[idx].clamp(80.0, 1.0e6)
    };

    let half = Vec2::new(radius * 1.15, radius * 1.15);
    let soft = (radius * 0.06).max(1e-6);
    let heaviest = bodies
        .iter()
        .map(|b| if b.mass.is_finite() { b.mass } else { 0.0 })
        .fold(0.0_f64, f64::max)
        .max(1e-9);

    // Нормировка: глубина под самым массивным телом ≈ 45% высоты вью.
    let max_depth = (half.x.max(half.y) * 0.85).max(1e-6);
    let norm = (size.y as f64 * 0.45 * soft / heaviest).clamp(0.0, 1.0e12);

    let depth = |x: f64, y: f64| -> f64 {
        let mut d = 0.0;
        for b in bodies {
            let dx = x - b.pos.x;
            let dy = y - b.pos.y;
            let r2 = dx * dx + dy * dy + soft * soft;
            if r2 > 1e-12 && r2.is_finite() {
                d += b.mass / r2.sqrt();
            }
        }
        let z = d * norm;
        if z.is_finite() {
            z.clamp(0.0, max_depth)
        } else {
            0.0
        }
    };

    // Проекция: поворот вокруг вертикали, затем лёгкий наклон; z (глубина
    // впадины) смещает точку вниз по экрану.
    let scale = (size.x as f64 * 0.42) / radius.max(1.0);
    let (sin, cos) = (state.yaw.sin() as f64, state.yaw.cos() as f64);
    let origin = rect.center() + egui::vec2(0.0, size.y * 0.12);
    let project = |x: f64, y: f64, z: f64| -> egui::Pos2 {
        let rx = x * cos - y * sin;
        let ry = x * sin + y * cos;
        egui::pos2(
            origin.x + (rx * scale) as f32,
            origin.y + (ry * scale * 0.52 + z * scale) as f32,
        )
    };

    // Сетка: строки и столбцы полилиниями. Глубже впадина — ярче линия.
    let line = |points: Vec<egui::Pos2>, depth_avg: f64| {
        let t = (depth_avg / max_depth).clamp(0.0, 1.0) as f32;
        let alpha = 50.0 + t * 150.0;
        let color = egui::Color32::from_rgba_unmultiplied(
            (70.0 + t * 120.0) as u8,
            (130.0 + t * 60.0) as u8,
            230,
            alpha as u8,
        );
        painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
    };

    let world =
        |i: usize, axis_len: f64| -> f64 { (i as f64 / MESH as f64 - 0.5) * 2.0 * axis_len };

    for gy in 0..=MESH {
        let mut points = Vec::with_capacity(MESH + 1);
        let mut d_sum = 0.0;
        for gx in 0..=MESH {
            let ox = world(gx, half.x);
            let oy = world(gy, half.y);
            let z = depth(center_world.x + ox, center_world.y + oy);
            d_sum += z;
            points.push(project(ox, oy, z));
        }
        line(points, d_sum / (MESH + 1) as f64);
    }
    for gx in 0..=MESH {
        let mut points = Vec::with_capacity(MESH + 1);
        let mut d_sum = 0.0;
        for gy in 0..=MESH {
            let ox = world(gx, half.x);
            let oy = world(gy, half.y);
            let z = depth(center_world.x + ox, center_world.y + oy);
            d_sum += z;
            points.push(project(ox, oy, z));
        }
        line(points, d_sum / (MESH + 1) as f64);
    }

    // Тела — светящиеся точки на дне своих впадин.
    for b in bodies {
        let rel = b.pos.xy() - center_world;
        if !rel.x.is_finite()
            || !rel.y.is_finite()
            || rel.x.abs() > half.x * 1.25
            || rel.y.abs() > half.y * 1.25
        {
            continue;
        }
        let z = depth(b.pos.x, b.pos.y);
        let p = project(rel.x, rel.y, z);
        if !rect.contains(p) {
            continue;
        }
        let r = ((b.radius * scale * 0.9) as f32).clamp(2.0, 12.0);
        painter.circle_filled(
            p,
            r,
            egui::Color32::from_rgb(b.color[0], b.color[1], b.color[2]),
        );
        if b.mass >= 100.0 {
            painter.circle_filled(p, r * 0.55, egui::Color32::WHITE);
        }
    }

    painter.text(
        rect.left_top() + egui::vec2(8.0, 6.0),
        egui::Align2::LEFT_TOP,
        "3D · гравитационный колодец (тяните мышью)",
        egui::FontId::proportional(11.0),
        egui::Color32::from_gray(160),
    );

    rect
}
