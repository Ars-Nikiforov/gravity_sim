//! Центральный дисплей: отрисовка тел из последнего снапшота физики
//! (или кадров повтора), взаимодействие и визуализации.
//!
//! Два режима камеры:
//! * 2D — вид сверху (как раньше);
//! * 3D — настоящая перспективная камера с положением в пространстве:
//!   look-at цель, азимут/возвышение, дистанция и угол обзора.
//!   Физика честно трёхмерная, сценарии имеют наклонения и объём.

use std::collections::HashSet;

use eframe::egui;

use crate::app::{barycenter, barycenter_filtered, ColorMode, FocusMode, GravityApp, Tool};
use crate::physics::{Body, Command, Vec2, Vec3};
use crate::ui::{heatmap, overlays};

/// Камера 2D: преобразование мир ↔ экран. `zoom` — пикселей на мировую единицу.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub center: Vec2,
    pub zoom: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Vec2::ZERO,
            zoom: 1.2,
        }
    }
}

impl Camera {
    pub fn world_to_screen(self, rect: egui::Rect, p: Vec2) -> egui::Pos2 {
        let c = rect.center();
        egui::pos2(
            c.x + ((p.x - self.center.x) * self.zoom) as f32,
            c.y + ((p.y - self.center.y) * self.zoom) as f32,
        )
    }

    pub fn screen_to_world(self, rect: egui::Rect, p: egui::Pos2) -> Vec2 {
        let c = rect.center();
        Vec2::new(
            (p.x - c.x) as f64 / self.zoom + self.center.x,
            (p.y - c.y) as f64 / self.zoom + self.center.y,
        )
    }
}

/// Настоящая 3D-камера: положение задаётся точкой взгляда (target),
/// сферическими углами и дистанцией. Матрица вида строится через
/// ортонормированный базис (right, up, forward), проекция — перспективная.
#[derive(Clone, Copy, Debug)]
pub struct Camera3D {
    /// Точка, на которую смотрит камера.
    pub target: Vec3,
    /// Азимут (радианы).
    pub yaw: f32,
    /// Возвышение над плоскостью xy (радианы): 0 — с ребра, +π/2 — сверху.
    pub pitch: f32,
    /// Расстояние от камеры до цели (мировые единицы).
    pub distance: f64,
    /// Вертикальный угол обзора (радианы).
    pub fov_y: f32,
}

impl Default for Camera3D {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: 0.7,
            pitch: 0.78,
            distance: 1150.0,
            fov_y: 0.9,
        }
    }
}

impl Camera3D {
    /// Направление от цели к камере.
    pub fn direction(&self) -> Vec3 {
        let cp = self.pitch.cos() as f64;
        // Нормализуем: sin/cos во f32 дают длину, отличную от 1 на ~1e-7,
        // а проекция и обратная задача требуют точного ортонормированного базиса.
        Vec3::new(
            self.yaw.cos() as f64 * cp,
            self.yaw.sin() as f64 * cp,
            self.pitch.sin() as f64,
        )
        .normalized()
    }

    /// Положение камеры в мире.
    pub fn eye(&self) -> Vec3 {
        self.target + self.direction() * self.distance
    }

    /// Ортонормированный базис вида: (right, up, forward).
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = -self.direction();
        let world_up = Vec3::new(0.0, 0.0, 1.0);
        let mut right = forward.cross(world_up);
        if right.len() < 1e-9 {
            right = Vec3::new(1.0, 0.0, 0.0);
        } else {
            right = right.normalized();
        }
        let up = right.cross(forward).normalized();
        (right, up, forward)
    }

    /// Фокусное расстояние в пикселях для данного прямоугольника.
    pub fn focal(&self, rect: egui::Rect) -> f64 {
        let fov = (self.fov_y as f64).clamp(0.15, 2.8);
        (rect.height() as f64 * 0.5) / (fov * 0.5).tan()
    }

    /// Перспективная проекция точки мира на экран.
    pub fn project(&self, rect: egui::Rect, p: Vec3) -> Option<Projected> {
        let (right, up, forward) = self.basis();
        let eye = self.eye();
        let rel = p - eye;
        let depth = rel.dot(forward);
        if depth <= 0.08 {
            return None;
        }
        let x = rel.dot(right);
        let y = rel.dot(up);
        let focal = self.focal(rect);
        let c = rect.center();
        Some(Projected {
            pos: egui::pos2(
                c.x + (x / depth * focal) as f32,
                c.y - (y / depth * focal) as f32,
            ),
            depth,
        })
    }

    /// Экран → точка на плоскости, проходящей через target перпендикулярно
    /// взгляду. Удобно для добавления тел «перед камерой» в 3D.
    pub fn unproject_view_plane(&self, rect: egui::Rect, sp: egui::Pos2) -> Vec3 {
        let (right, up, _) = self.basis();
        let focal = self.focal(rect).max(1e-6);
        let c = rect.center();
        let sx = (sp.x - c.x) as f64 / focal;
        let sy = -(sp.y - c.y) as f64 / focal;
        self.target + right * (sx * self.distance) + up * (sy * self.distance)
    }

    /// Экран → точка на плоскости z = 0 (если луч её пересекает).
    pub fn unproject_ground(&self, rect: egui::Rect, sp: egui::Pos2) -> Option<Vec3> {
        let (right, up, forward) = self.basis();
        let eye = self.eye();
        let focal = self.focal(rect).max(1e-6);
        let c = rect.center();
        let sx = (sp.x - c.x) as f64 / focal;
        let sy = -(sp.y - c.y) as f64 / focal;
        let dir = (forward + right * sx + up * sy).normalized();
        if dir.z.abs() < 1e-6 {
            return None;
        }
        let t = -eye.z / dir.z;
        if t <= 0.0 || !t.is_finite() {
            return None;
        }
        Some(eye + dir * t)
    }

    /// Вращение вокруг цели.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.01;
        self.pitch = (self.pitch - dy * 0.01).clamp(-1.45, 1.45);
        if self.pitch.abs() < 0.05 {
            self.pitch = 0.05 * if self.pitch < 0.0 { -1.0 } else { 1.0 };
        }
    }

    /// Панорама: цель едет в плоскости камеры («хватаем» сцену мышью).
    pub fn pan(&mut self, rect: egui::Rect, dx: f32, dy: f32) {
        let scale = 2.0 * self.distance * (self.fov_y as f64 * 0.5).tan()
            / rect.height().max(1.0) as f64;
        let (right, up, _) = self.basis();
        self.target += right * (-dx as f64 * scale) + up * (dy as f64 * scale);
    }

    /// Приблизить/отдалить. `factor > 1` — ближе.
    pub fn zoom_by(&mut self, factor: f64) {
        if factor.is_finite() && factor > 0.0 {
            self.distance = (self.distance / factor).clamp(5.0, 1.0e7);
        }
    }
}

/// Результат проекции точки мира на экран.
#[derive(Clone, Copy, Debug)]
pub struct Projected {
    pub pos: egui::Pos2,
    /// Расстояние вдоль оси взгляда (больше — дальше).
    pub depth: f64,
}

/// Кэш предсказания траектории выбранного тела.
pub struct Prediction {
    pub id: u64,
    pub tick: u64,
    pub pts: Vec<Vec3>,
}

/// Сколько точек следа хранит каждое тело.
const TRAIL_LEN: usize = 240;
/// Горизонт предсказания: шагов и коэффициент укрупнения dt.
const PREDICT_STEPS: usize = 320;
const PREDICT_DT_SCALE: f64 = 2.0;

/// Гравитационное ускорение в точке от всех тел (для визуализаций).
fn accel_at(bodies: &[Body], p: Vec3, g: f64, softening: f64) -> Vec3 {
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
fn heat_color(t: f32) -> egui::Color32 {
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
fn field_t(mag: f64) -> f32 {
    let lo = -3.5_f64;
    let hi = 0.5_f64;
    let v = (mag.max(1e-9).log10() - lo) / (hi - lo);
    v.clamp(0.0, 1.0) as f32
}

fn lighten(c: egui::Color32, k: f32) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c.r() as f32 * k).min(255.0) as u8,
        (c.g() as f32 * k).min(255.0) as u8,
        (c.b() as f32 * k).min(255.0) as u8,
    )
}

pub fn show(ui: &mut egui::Ui, app: &mut GravityApp) {
    let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = response.rect;

    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(10, 10, 16));

    // ── Источник кадров: живая физика или повтор ────────────────
    let replay_active = app.replay.is_some();
    if let Some(rp) = &mut app.replay {
        let dt = ui.input(|i| i.unstable_dt).min(0.1);
        rp.advance(dt);
    }
    let (bodies, running, step_ms, sim_time, tick, epoch) = if let Some(rp) = &app.replay {
        let f = rp.current();
        (f.bodies.clone(), rp.playing, 0.0, f.sim_time, f.tick, 0u64)
    } else {
        let snapshot = app.physics.snapshot();
        (
            snapshot.bodies.clone(),
            snapshot.running,
            snapshot.step_ms,
            snapshot.sim_time,
            snapshot.tick,
            snapshot.epoch,
        )
    };

    // ── Новое поколение состояния: id переиспользованы → чистим следы ──
    if !replay_active && app.last_epoch != epoch {
        app.last_epoch = epoch;
        app.trails.clear();
        app.prediction = None;
        app.selection.clear();
        app.selected_id = None;
    }

    // ── Камера: слежение/автоцентр/автомасштаб ──────────────────
    update_camera(app, &bodies, rect);
    // Держим 2D-центр и 3D-цель синхронными, чтобы переключение вида
    // не прыгало в начало координат.
    if app.view3d {
        app.camera.center = app.cam3d.target.xy();
    } else {
        app.cam3d.target = Vec3::new(
            app.camera.center.x,
            app.camera.center.y,
            app.cam3d.target.z,
        );
    }

    let cam = app.camera;
    let cam3 = app.cam3d;
    let (g, softening) = (app.params.g, app.params.softening);
    let view3d = app.view3d;

    let project = move |p: Vec3| -> Option<Projected> {
        if view3d {
            cam3.project(rect, p)
        } else {
            Some(Projected {
                pos: cam.world_to_screen(rect, p.xy()),
                depth: p.z,
            })
        }
    };

    // ── Тепловая карта поля (2D и 3D, на плоскости z = 0) ────────
    if app.show_field_heatmap && !bodies.is_empty() {
        let (hmin, hmax) = heatmap_bounds(app.camera, app.cam3d, rect, app.view3d);
        // В повторе epoch постоянен — подмешиваем тик, чтобы не показывать
        // поле из другого места лога.
        let heat_epoch = if replay_active {
            tick.wrapping_add(0x9E37_79B9_7F4A_7C15)
        } else {
            epoch
        };
        heatmap::draw(
            &painter,
            rect,
            hmin,
            hmax,
            &bodies,
            g,
            softening,
            &mut app.heatmap,
            heat_epoch,
            tick,
            &|p| project(p).map(|pr| pr.pos),
        );
    }

    // ── 3D: сетка плоскости + «нити» от тел к плоскости ─────────
    if app.view3d {
        draw_plane_grid(&painter, rect, cam3);
        draw_depth_drops(&painter, rect, cam3, &bodies);
    }

    // ── Следы орбит ─────────────────────────────────────────────
    if app.show_trails {
        if running {
            for body in &bodies {
                app.trails.entry(body.id).or_default().push_back(body.pos);
                let trail = app.trails.get_mut(&body.id).unwrap();
                while trail.len() > TRAIL_LEN {
                    trail.pop_front();
                }
            }
        }
        let alive: HashSet<u64> = bodies.iter().map(|b| b.id).collect();
        app.trails.retain(|id, _| alive.contains(id));

        for (id, points) in &app.trails {
            if points.len() < 2 {
                continue;
            }
            let color = bodies
                .iter()
                .find(|b| b.id == *id)
                .map(|b| {
                    egui::Color32::from_rgba_unmultiplied(b.color[0], b.color[1], b.color[2], 70)
                })
                .unwrap_or(egui::Color32::GRAY);
            let screen_points: Vec<egui::Pos2> =
                points.iter().filter_map(|&p| project(p).map(|pr| pr.pos)).collect();
            if screen_points.len() >= 2 {
                painter.add(egui::Shape::line(screen_points, egui::Stroke::new(1.0, color)));
            }
        }
    }

    // ── Предсказание орбиты выбранного тела ─────────────────────
    if app.show_prediction && !replay_active {
        update_prediction(app, &bodies, tick);
        if let Some(pred) = &app.prediction {
            let color = bodies
                .iter()
                .find(|b| b.id == pred.id)
                .map(|b| egui::Color32::from_rgb(b.color[0], b.color[1], b.color[2]))
                .unwrap_or(egui::Color32::LIGHT_BLUE);
            let pts: Vec<egui::Pos2> = pred
                .pts
                .iter()
                .filter_map(|&p| project(p).map(|pr| pr.pos))
                .collect();
            // Две проходки: широкое тусклое «свечение» + яркий пунктир.
            // Так линия читается и на тёмном фоне, и поверх сетки/следов.
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(
                    pts.clone(),
                    egui::Stroke::new(3.5, color.gamma_multiply(0.16)),
                ));
            }
            for chunk in pts.chunks(3) {
                if chunk.len() >= 2 {
                    painter.add(egui::Shape::line(
                        chunk.to_vec(),
                        egui::Stroke::new(1.8, color.gamma_multiply(1.05)),
                    ));
                }
            }
            // Отметки вдоль траектории — видно направление движения.
            for (i, p) in pts.iter().enumerate() {
                if i % 24 == 0 {
                    painter.circle_filled(*p, 1.8, color.gamma_multiply(0.9));
                }
            }
            if let Some(last) = pts.last() {
                painter.circle_filled(*last, 3.0, color);
            }
        }
    }

    // ── Точки Лагранжа, барицентр пары, сфера Хилла ─────────────
    if app.show_lagrange || app.show_hill {
        overlays::draw(
            &painter,
            rect,
            &|p| project(p).map(|pr| pr.pos),
            &bodies,
            app.show_lagrange,
            app.show_hill,
        );
    }

    // ── Ускорения тел (для стрелок и раскраски) ─────────────────
    let accels: Vec<Vec3> = bodies
        .iter()
        .map(|b| accel_at(&bodies, b.pos, g, softening))
        .collect();

    // ── Тела (в 3D — сортировка по глубине, дальние раньше) ─────
    let mut order: Vec<usize> = (0..bodies.len()).collect();
    if app.view3d {
        order.sort_by(|&i, &j| {
            let di = project(bodies[i].pos).map(|p| p.depth).unwrap_or(f64::INFINITY);
            let dj = project(bodies[j].pos).map(|p| p.depth).unwrap_or(f64::INFINITY);
            dj.total_cmp(&di)
        });
    }

    let focal = cam3.focal(rect);
    for i in order {
        let body = &bodies[i];
        let Some(pr) = project(body.pos) else { continue };
        let sp = pr.pos;
        if !rect.expand(60.0).contains(sp) {
            continue;
        }
        let scale = if app.view3d {
            (focal / pr.depth.max(0.1)).clamp(0.0, 1.0e6)
        } else {
            cam.zoom
        };
        let radius = (body.radius * scale)
            .clamp(2.0, rect.height().max(1.0) as f64 * 1.5) as f32;
        if !radius.is_finite() {
            continue;
        }
        let fog = if app.view3d {
            let d = cam3.distance.max(1.0);
            (1.0 - ((pr.depth - d).abs() / d) * 0.45).clamp(0.35, 1.0) as f32
        } else {
            1.0_f32
        };
        let base = match app.color_mode {
            ColorMode::Normal => {
                egui::Color32::from_rgb(body.color[0], body.color[1], body.color[2])
            }
            ColorMode::ByGravity => heat_color(field_t(accels[i].len())),
        };
        let color = base.gamma_multiply(fog);

        if app.view3d {
            // Мягкое свечение — читается как объёмное тело, а не точка.
            let glow = if body.mass >= 100.0 { 2.4 } else { 1.8 };
            painter.circle_filled(sp, radius * glow, base.gamma_multiply(0.10 * fog));
            painter.circle_filled(sp, radius, color);
            // Блик — читается как освещённая сфера, а не плоский кружок.
            painter.circle_filled(
                sp + egui::vec2(-radius * 0.32, -radius * 0.32),
                (radius * 0.55).max(0.8),
                lighten(base, 1.55).gamma_multiply(fog),
            );
            painter.circle_stroke(
                sp,
                (radius * 0.98).max(1.0),
                egui::Stroke::new(1.0, egui::Color32::from_black_alpha(90)),
            );
        } else {
            painter.circle_filled(sp, radius, color);
            if body.mass >= 100.0 {
                painter.circle_filled(sp, radius * 0.55, egui::Color32::WHITE);
            }
        }

        if app.selected_id == Some(body.id) {
            let ring = egui::Color32::from_rgb(120, 220, 255);
            painter.circle_stroke(sp, radius + 4.0, egui::Stroke::new(2.0, ring));
        } else if app.selection.contains(&body.id) {
            let ring = egui::Color32::from_rgb(120, 255, 190);
            painter.circle_stroke(sp, radius + 3.0, egui::Stroke::new(1.5, ring));
        }

        if app.show_labels {
            painter.text(
                sp + egui::vec2(radius + 6.0, -radius - 6.0),
                egui::Align2::LEFT_BOTTOM,
                format!("#{}", body.id),
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(200),
            );
        }
    }

    // ── Векторы сил ─────────────────────────────────────────────
    if app.show_forces {
        for (i, body) in bodies.iter().enumerate() {
            let Some(pr) = project(body.pos) else { continue };
            if !rect.expand(20.0).contains(pr.pos) {
                continue;
            }
            let a = accels[i];
            let mag = a.len();
            if mag < 1e-6 {
                continue;
            }
            let len = (mag.max(1e-6).log10() * 14.0 + 26.0).clamp(14.0, 120.0);
            let tip_world = body.pos + a * (len / mag);
            let Some(pr_tip) = project(tip_world) else { continue };
            let t = field_t(mag);
            painter.arrow(
                pr.pos,
                pr_tip.pos - pr.pos,
                egui::Stroke::new(1.6, heat_color(t).gamma_multiply(0.9)),
            );
        }
    }

    // ── 3D-колодец в углу (только 2D-режим) ─────────────────────
    let mut well_rect: Option<egui::Rect> = None;
    if app.show_well3d && !app.view3d {
        well_rect = Some(super::well3d::draw(
            ui,
            &painter,
            rect,
            &bodies,
            &mut app.well,
        ));
    }

    // ── Оверлей статуса ─────────────────────────────────────────
    if app.show_fps {
        let mode = if app.view3d { "3D" } else { "2D" };
        let status = if replay_active {
            format!(
                "ПОВТОР [{mode}] | UI: {:5.1} fps | тик {} | t = {:.1}",
                app.fps, tick, sim_time
            )
        } else {
            format!(
                "{mode} | UI: {:5.1} fps | физика: {:5.2} мс/шаг | тел: {} | t = {:.1}",
                app.fps,
                step_ms,
                bodies.len(),
                sim_time
            )
        };
        painter.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            status,
            egui::FontId::monospace(12.0),
            if replay_active {
                egui::Color32::from_rgb(255, 200, 120)
            } else {
                egui::Color32::from_gray(170)
            },
        );
    }

    // Подсказки по управлению и статус камеры.
    let mut hints = Vec::new();
    if app.view3d {
        hints.push("ПКМ — орбита · ЛКМ — панорама · Shift+ЛКМ — рамка · колесо — зум");
    }
    if app.show_prediction && app.selected_id.is_none() {
        hints.push("Предсказание: выберите тело (клик или рамка)");
    }
    if (app.show_lagrange || app.show_hill) && overlays::dominant_pair(&bodies).is_none() {
        hints.push("Лагранж/Хилл: нужно ≥ 2 массивных тела");
    }
    if app.focus_mode != FocusMode::Free {
        hints.push(app.focus_mode.name());
    }
    if !hints.is_empty() {
        let text = hints.join("   ·   ");
        painter.text(
            rect.right_top() + egui::vec2(-8.0, 8.0),
            egui::Align2::RIGHT_TOP,
            text,
            egui::FontId::proportional(11.0),
            egui::Color32::from_rgba_unmultiplied(150, 175, 210, 190),
        );
    }
    if !app.selection.is_empty() {
        painter.text(
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            format!("Выделено тел: {}", app.selection.len()),
            egui::FontId::proportional(11.0),
            egui::Color32::from_rgba_unmultiplied(140, 230, 190, 200),
        );
    }

    // ── Взаимодействие ──────────────────────────────────────────
    let cursor = ui.input(|i| i.pointer.hover_pos());
    let over_well = well_rect.map_or(false, |r| cursor.map_or(false, |c| r.contains(c)));
    let shift = ui.input(|i| i.modifiers.shift);

    // Зум колесом.
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll != 0.0 && !over_well {
        if let Some(cursor) = cursor {
            if rect.contains(cursor) {
                let factor = (scroll as f64 * 0.01).exp();
                if app.view3d {
                    // Зум к курсору: точка под курсором остаётся на месте.
                    let anchor = app.cam3d.unproject_view_plane(rect, cursor);
                    let before = app.cam3d.distance.max(1e-9);
                    app.cam3d.zoom_by(factor);
                    let ratio = app.cam3d.distance / before;
                    app.cam3d.target = anchor + (app.cam3d.target - anchor) * ratio;
                } else {
                    let before = cam.screen_to_world(rect, cursor);
                    app.camera.zoom = (app.camera.zoom * factor).clamp(0.02, 40.0);
                    let after = app.camera.screen_to_world(rect, cursor);
                    app.camera.center = app.camera.center + (before - after);
                }
            }
        }
    }

    // Начало рамки выделения: Select + Shift + ЛКМ.
    if app.tool == Tool::Select
        && !over_well
        && response.drag_started_by(egui::PointerButton::Primary)
        && shift
    {
        if let Some(p) = response.interact_pointer_pos() {
            app.box_select_start = Some(p);
            app.box_select_additive =
                ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        }
    }
    let box_active = app.box_select_start.is_some();

    // Панорама/орбита (рамка выделения блокирует панораму).
    let primary_pan = app.tool == Tool::Select
        && response.dragged_by(egui::PointerButton::Primary)
        && !over_well
        && !shift;
    let secondary_drag = response.dragged_by(egui::PointerButton::Secondary) && !over_well;
    let middle_drag = response.dragged_by(egui::PointerButton::Middle) && !over_well;

    if !box_active {
        if app.view3d {
            if secondary_drag {
                let d = response.drag_delta();
                app.cam3d.orbit(d.x, d.y);
            } else if primary_pan || middle_drag {
                let d = response.drag_delta();
                app.cam3d.pan(rect, d.x, d.y);
                if app.focus_mode != FocusMode::Free {
                    app.focus_mode = FocusMode::Free;
                }
            }
        } else if primary_pan || secondary_drag || middle_drag {
            let d = response.drag_delta();
            app.camera.center.x -= d.x as f64 / app.camera.zoom;
            app.camera.center.y -= d.y as f64 / app.camera.zoom;
            if app.focus_mode != FocusMode::Free {
                app.focus_mode = FocusMode::Free;
            }
        }
    }

    // Завершение рамки: выбрать тела, попавшие в прямоугольник.
    if app.box_select_start.is_some() && response.drag_stopped() {
        let start = app.box_select_start.take().unwrap();
        let end = response.interact_pointer_pos().unwrap_or(start);
        let sel_rect = egui::Rect::from_two_pos(start, end);
        if !app.box_select_additive {
            app.selection.clear();
        }
        for b in &bodies {
            if let Some(pr) = project(b.pos) {
                if sel_rect.contains(pr.pos) {
                    app.selection.insert(b.id);
                }
            }
        }
        // Основное выбранное тело — самое массивное из выделения.
        app.selected_id = bodies
            .iter()
            .filter(|b| app.selection.contains(&b.id))
            .max_by(|a, b| a.mass.total_cmp(&b.mass))
            .map(|b| b.id);
    }

    // Рисуем рамку поверх сцены.
    if let Some(start) = app.box_select_start {
        if let Some(cur) = response.interact_pointer_pos() {
            let r = egui::Rect::from_two_pos(start, cur);
            painter.rect_filled(
                r,
                2.0,
                egui::Color32::from_rgba_unmultiplied(120, 200, 255, 26),
            );
            painter.rect_stroke(
                r,
                2.0,
                egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 200, 255)),
                egui::StrokeKind::Middle,
            );
        }
    }

    // Клик — применить инструмент.
    if response.clicked() && !over_well && app.box_select_start.is_none() {
        if let Some(pos) = response.interact_pointer_pos() {
            let hit = pick_body(&bodies, &project, pos);
            match app.tool {
                Tool::Select => {
                    if shift {
                        // Shift+клик — добавить/убрать тело из выделения.
                        if let Some(b) = hit {
                            if !app.selection.remove(&b.id) {
                                app.selection.insert(b.id);
                            }
                            app.selected_id = if app.selection.is_empty() {
                                None
                            } else {
                                Some(b.id)
                            };
                        }
                    } else {
                        app.selection.clear();
                        if let Some(b) = hit {
                            app.selection.insert(b.id);
                            app.selected_id = Some(b.id);
                        } else {
                            app.selected_id = None;
                        }
                    }
                }
                Tool::AddBody => {
                    let world = if app.view3d {
                        // Смотрим сверху — ставим на плоскость z=0; с ребра —
                        // на плоскость взгляда (иначе клик уходит в бесконечность).
                        let p = if app.cam3d.pitch > 0.45 {
                            app.cam3d
                                .unproject_ground(rect, pos)
                                .unwrap_or_else(|| app.cam3d.unproject_view_plane(rect, pos))
                        } else {
                            app.cam3d.unproject_view_plane(rect, pos)
                        };
                        Some(p)
                    } else {
                        Some(cam.screen_to_world(rect, pos).to3())
                    };
                    if let Some(world) = world {
                        let nb = app.new_body;
                        app.physics.send(Command::AddBody {
                            mass: nb.mass,
                            radius: nb.radius,
                            color: nb.color,
                            pos: world,
                        });
                    }
                }
                Tool::DeleteBody => {
                    if let Some(body) = hit {
                        app.physics.send(Command::RemoveBody(body.id));
                        app.selection.remove(&body.id);
                        if app.selected_id == Some(body.id) {
                            app.selected_id = None;
                        }
                    }
                }
            }
        }
    }

    // Двойной клик по телу — разово навести камеру на него.
    if response.double_clicked() && !over_well {
        if let Some(pos) = response.interact_pointer_pos() {
            if let Some(body) = pick_body(&bodies, &project, pos) {
                app.focus_point_once(body.pos);
            }
        }
    }
}

/// Подтягиваем камеру к цели слежения, автоцентру и автомасштабу.
fn update_camera(app: &mut GravityApp, bodies: &[Body], rect: egui::Rect) {
    let view3d = app.view3d;

    // 1. Слежение за телом/системой/выделением.
    let follow: Option<Vec3> = match app.focus_mode {
        FocusMode::Free => None,
        FocusMode::FollowSelected => app
            .selected_id
            .and_then(|id| bodies.iter().find(|b| b.id == id))
            .map(|b| b.pos),
        FocusMode::FollowSystem => barycenter_filtered(bodies, |_| true),
        FocusMode::FollowSelection => {
            if app.selection.is_empty() {
                None
            } else {
                barycenter_filtered(bodies, |b| app.selection.contains(&b.id))
            }
        }
    };
    if let Some(t) = follow.filter(|p| p.is_finite()) {
        let k = 0.18;
        app.cam3d.target = app.cam3d.target.lerp(t, k);
        app.camera.center = app.camera.center.lerp(t.xy(), k);
    } else if app.auto_center && !bodies.is_empty() {
        // Мёртвая зона: не дёргаем камеру, пока система в кадре.
        let c = barycenter(bodies);
        if c.is_finite() {
            let (visible, dist) = if view3d {
                let visible = app.cam3d.distance * (app.cam3d.fov_y as f64 * 0.5).tan();
                (visible, (c - app.cam3d.target).len())
            } else {
                let visible =
                    rect.width().min(rect.height()) as f64 * 0.5 / app.camera.zoom.max(1e-6);
                (visible, c.xy().distance(app.camera.center))
            };
            if dist > visible * 0.6 {
                // Далеко улетели — «телепорт» ближе; чуть вышли — плавно.
                let k = if dist > visible * 2.0 { 1.0 } else { 0.10 };
                app.cam3d.target = app.cam3d.target.lerp(c, k);
                app.camera.center = app.camera.center.lerp(c.xy(), k);
            }
        }
    }

    // 2. Автомасштаб: вписать 90% тел в кадр.
    if app.auto_fit && !bodies.is_empty() {
        let center = if view3d {
            app.cam3d.target
        } else {
            app.camera.center.to3()
        };
        let radius = robust_radius(bodies, center, view3d);
        if view3d {
            let desired =
                (radius / (app.cam3d.fov_y as f64 * 0.5).tan()).clamp(10.0, 1.0e7);
            app.cam3d.distance += (desired - app.cam3d.distance) * 0.08;
        } else {
            let desired = (rect.width().min(rect.height()) as f64 * 0.42 / radius)
                .clamp(0.02, 40.0);
            app.camera.zoom += (desired - app.camera.zoom) * 0.08;
        }
    }
}

/// Область мира на плоскости z = 0, которую покрывает тепловая карта.
fn heatmap_bounds(cam: Camera, cam3: Camera3D, rect: egui::Rect, view3d: bool) -> (Vec2, Vec2) {
    if view3d {
        let visible = cam3.distance * (cam3.fov_y as f64 * 0.5).tan();
        let aspect = (rect.width() / rect.height().max(1.0)) as f64;
        let half = (visible * aspect.max(1.0) * 2.2).clamp(20.0, 5.0e5);
        let c = cam3.target.xy();
        (
            Vec2::new(c.x - half, c.y - half),
            Vec2::new(c.x + half, c.y + half),
        )
    } else {
        (
            cam.screen_to_world(rect, rect.left_top()),
            cam.screen_to_world(rect, rect.right_bottom()),
        )
    }
}

/// Радиус, внутри которого лежат ~90% тел (устойчив к одиночным выбросам).
fn robust_radius(bodies: &[Body], center: Vec3, use_z: bool) -> f64 {
    let mut distances: Vec<f64> = bodies
        .iter()
        .map(|b| {
            let d = if use_z {
                (b.pos - center).len()
            } else {
                (b.pos.xy() - center.xy()).len()
            };
            d + b.radius
        })
        .filter(|d| d.is_finite())
        .collect();
    if distances.is_empty() {
        return 400.0;
    }
    distances.sort_by(|a, b| a.total_cmp(b));
    let idx = ((distances.len() as f64 * 0.9) as usize).min(distances.len() - 1);
    distances[idx].max(60.0)
}

/// Шаг и число точек предсказания. Горизонт подбирается по орбитальному
/// периоду вокруг самого влиятельного тела: иначе у быстрой планеты
/// «призрак» оказывается коротким огрызком, а не орбитой.
fn predict_step(app: &GravityApp, bodies: &[Body], focus: usize) -> (usize, f64) {
    let steps = if bodies.len() > 150 {
        PREDICT_STEPS / 2
    } else {
        PREDICT_STEPS
    };
    let base = (app.params.dt * PREDICT_DT_SCALE).max(0.005);
    let b = &bodies[focus];

    // Доминирующее тело — максимальный вклад m/r².
    let mut best = 0.0_f64;
    let mut m_in = 0.0_f64;
    let mut r_min = f64::INFINITY;
    for (i, o) in bodies.iter().enumerate() {
        if i == focus {
            continue;
        }
        let r = (o.pos - b.pos).len().max(1e-9);
        let influence = o.mass / (r * r);
        if influence.is_finite() && influence > best {
            best = influence;
            m_in = o.mass;
            r_min = r;
        }
    }

    let g = app.params.g;
    if !(g > 0.0) || m_in <= 0.0 || !r_min.is_finite() {
        return (steps, base * 4.0);
    }

    let mu = g * m_in;
    // Период круговой орбиты на текущем радиусе: T = 2π·sqrt(r³/μ).
    let period = std::f64::consts::TAU * (r_min * r_min * r_min / mu).sqrt();
    // Гипербола/парабола: периода нет — берём умеренный горизонт.
    let energy = 0.5 * b.vel.len2() - mu / r_min;
    let horizon = if energy >= 0.0 || !period.is_finite() {
        base * steps as f64 * 0.5
    } else {
        // ~85% орбиты: видно форму и направление, без лишнего «хвоста».
        period * 0.85
    };
    let dt = (horizon / steps as f64).clamp(base, base * 80.0);
    (steps, dt)
}

/// Обновить кэш предсказания: пересчёт не чаще раза в 30 тиков, при смене
/// выбранного тела или сбросе тика (новый сценарий).
fn update_prediction(app: &mut GravityApp, bodies: &[Body], tick: u64) {
    let Some(sel) = app.selected_id else {
        app.prediction = None;
        return;
    };
    let need = match &app.prediction {
        Some(p) => p.id != sel || tick < p.tick || tick.saturating_sub(p.tick) >= 30,
        None => true,
    };
    if !need {
        return;
    }
    let Some(focus) = bodies.iter().position(|b| b.id == sel) else {
        app.prediction = None;
        return;
    };
    let (steps, dt) = predict_step(app, bodies, focus);
    let pts = crate::physics::predict_path(bodies, app.params, focus, steps, dt);
    app.prediction = Some(Prediction {
        id: sel,
        tick,
        pts,
    });
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
fn draw_plane_grid(painter: &egui::Painter, rect: egui::Rect, cam3: Camera3D) {
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
fn draw_depth_drops(painter: &egui::Painter, rect: egui::Rect, cam3: Camera3D, bodies: &[Body]) {
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

/// Поиск тела под курсором (в экранных координатах, через проекцию).
fn pick_body<'a>(
    bodies: &'a [Body],
    project: &impl Fn(Vec3) -> Option<Projected>,
    pos: egui::Pos2,
) -> Option<&'a Body> {
    bodies
        .iter()
        .filter_map(|b| {
            let pr = project(b.pos)?;
            let d = (pr.pos - pos).length();
            Some((b, d))
        })
        .filter(|(_, d)| *d <= 18.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(b, _)| b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::sim::Simulation;
    use crate::physics::{Scenario, SimParams};

    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    #[test]
    fn camera_basis_is_orthonormal() {
        let cam = Camera3D::default();
        let (right, up, forward) = cam.basis();
        assert!((right.len() - 1.0).abs() < 1e-9);
        assert!((up.len() - 1.0).abs() < 1e-9);
        assert!((forward.len() - 1.0).abs() < 1e-9);
        assert!(right.dot(up).abs() < 1e-9);
        assert!(right.dot(forward).abs() < 1e-9);
        assert!(up.dot(forward).abs() < 1e-9);
    }

    #[test]
    fn view_plane_ray_roundtrip() {
        let cam = Camera3D::default();
        let r = rect();
        let p = Vec3::new(120.0, -80.0, 40.0);
        let sp = cam.project(r, p).expect("точка перед камерой");
        let q = cam.unproject_view_plane(r, sp.pos);
        let sp2 = cam.project(r, q).expect("точка перед камерой");
        assert!((sp.pos - sp2.pos).length() < 1e-3);
        let (_, _, forward) = cam.basis();
        assert!(((q - cam.target).dot(forward)).abs() < 1e-6);
    }

    #[test]
    fn ground_plane_roundtrip() {
        let cam = Camera3D::default();
        let r = rect();
        let p = Vec3::new(200.0, -150.0, 0.0);
        let sp = cam.project(r, p).expect("точка перед камерой");
        let q = cam.unproject_ground(r, sp.pos).expect("луч пересекает z=0");
        // Экранные координаты egui — f32, поэтому допускаем микронную ошибку.
        assert!((q - p).len() < 1e-3);
        assert!(q.z.abs() < 1e-6);
    }

    #[test]
    fn scenarios_are_really_3d() {
        let params = SimParams::default();
        for scenario in [
            Scenario::SolarSystem,
            Scenario::BinaryStar,
            Scenario::RandomCloud,
            Scenario::StableSystem,
        ] {
            let sim = Simulation::new(params, scenario);
            let max_z = sim
                .bodies()
                .iter()
                .map(|b| b.pos.z.abs())
                .fold(0.0_f64, f64::max);
            assert!(
                max_z > 1.0,
                "сценарий {} остался плоским (max |z| = {max_z})",
                scenario.name()
            );
        }
    }
}
