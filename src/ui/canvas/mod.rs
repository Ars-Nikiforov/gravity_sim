//! Центральный дисплей: композиция кадра — источник данных (живая
//! физика или повтор), порядок слоёв отрисовки (карта поля → ямы →
//! следы → орбиты → тела → силы), затем взаимодействие. Камеры — в
//! [`camera`], ввод — в [`input`], кэши предсказаний и слежение — в
//! [`predict`], статичные помощники отрисовки — в [`draw`].

mod camera;
mod draw;
mod input;
mod predict;

use std::collections::{HashMap, HashSet};

use eframe::egui;

use crate::app::{ColorMode, FocusMode, GravityApp, Tool};
use crate::physics::Vec3;
use crate::ui::canvas::draw::{accel_at, draw_depth_drops, draw_plane_grid, draw_regions, field_t, heat_color, lighten};
use crate::ui::canvas::input::handle_input;
use crate::ui::canvas::predict::{heatmap_bounds, update_camera, update_orbits, update_prediction};
use crate::ui::{heatmap, overlays};

pub use camera::{Camera, Camera3D, Projected};
pub use predict::Prediction;

/// Сколько точек следа хранит каждое тело.
const TRAIL_LEN: usize = 240;
/// Горизонт предсказания: шагов и коэффициент укрупнения dt.
const PREDICT_STEPS: usize = 320;
const PREDICT_DT_SCALE: f64 = 2.0;
/// Пересчёт орбит-предсказаний не чаще, чем раз в N тиков.
const ORBIT_REFRESH_TICKS: u64 = 30;
/// С какого числа тел орбиты мелких тел сливаются (если включено).
pub const ORBIT_BODY_LIMIT: usize = 100;
/// Сколько самых массивных тел получают орбиту в скоплении.
pub const ORBIT_TOP_MASSIVE: usize = 16;
/// На сколько кусков разбивается путь для затухания яркости.
const ORBIT_FADE_CHUNKS: usize = 10;


pub fn show(ui: &mut egui::Ui, app: &mut GravityApp) {
    let (response, painter) =
        ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
    let rect = response.rect;

    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(10, 10, 16));

    // ── Источник кадров: живая физика или повтор ────────────────
    let replay_active = app.replay.is_some();
    if let Some(rp) = &mut app.replay {
        let dt = ui.input(|i| i.unstable_dt).min(0.1);
        rp.advance(dt);
    }
    let (bodies, running, step_ms, sim_time, tick, epoch, regions) = if let Some(rp) = &app.replay {
        let f = rp.current();
        (
            f.bodies.clone(),
            rp.playing,
            0.0,
            f.sim_time,
            f.tick,
            0u64,
            Vec::new(),
        )
    } else {
        let snapshot = app.physics.snapshot();
        (
            snapshot.bodies.clone(),
            snapshot.running,
            snapshot.step_ms,
            snapshot.sim_time,
            snapshot.tick,
            snapshot.epoch,
            snapshot.regions.clone(),
        )
    };

    // ── Новое поколение состояния: id переиспользованы → чистим следы ──
    if !replay_active && app.last_epoch != epoch {
        app.last_epoch = epoch;
        app.trails.clear();
        app.prediction = None;
        app.orbits.clear();
        app.orbits_tick = 0;
        app.selection.clear();
        app.selected_id = None;
        app.orbit_host = None;
        app.frame_pending = true;
    }

    // ── Камера: слежение/автоцентр/автомасштаб ──────────────────
    let frame_dt = ui.input(|i| i.unstable_dt).clamp(1e-4, 0.1) as f64;
    update_camera(app, &bodies, rect, frame_dt);
    // Держим 2D-центр и 3D-цель синхронными, чтобы переключение вида
    // не прыгало в начало координат.
    if app.view3d {
        app.camera.center = app.cam3d.target.xy();
    } else {
        app.cam3d.target = Vec3::new(app.camera.center.x, app.camera.center.y, app.cam3d.target.z);
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

    // ── Гравитационные ямы: в 3D — «резина» в самой сцене ───────
    if app.show_well3d && app.view3d && !bodies.is_empty() {
        super::well3d::draw_scene(&painter, &project, &bodies);
    }

    // Цвета тел по id — один индекс за кадр: поиск в списке на каждый
    // след/орбиту давал O(N²) на кадр при большом числе тел.
    let colors_by_id: HashMap<u64, egui::Color32> = bodies
        .iter()
        .map(|b| {
            (
                b.id,
                egui::Color32::from_rgb(b.color[0], b.color[1], b.color[2]),
            )
        })
        .collect();

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
            let color = colors_by_id
                .get(id)
                .map(|c| egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 70))
                .unwrap_or(egui::Color32::GRAY);
            let screen_points: Vec<egui::Pos2> = points
                .iter()
                .filter_map(|&p| project(p).map(|pr| pr.pos))
                .collect();
            if screen_points.len() >= 2 {
                painter.add(egui::Shape::line(
                    screen_points,
                    egui::Stroke::new(1.0, color),
                ));
            }
        }
    }

    // ── Орбиты-предсказания всех тел (с затуханием к концу пути) ─
    if app.show_orbits && !replay_active {
        update_orbits(app, &bodies, tick);
        let selected = app.selected_id;
        let color_of = |id: u64| -> egui::Color32 {
            colors_by_id
                .get(&id)
                .copied()
                .unwrap_or(egui::Color32::LIGHT_BLUE)
        };
        for (id, path) in &app.orbits {
            if path.len() < 2 {
                continue;
            }
            let color = color_of(*id);
            let is_focus = selected == Some(*id);
            // Путь режется на куски; яркость падает от тела к концу пути,
            // поэтому видно и форму орбиты, и направление движения.
            let chunk = path.len().div_ceil(ORBIT_FADE_CHUNKS).max(2);
            let base_alpha = if is_focus { 200.0 } else { 110.0 };
            for (k, seg) in path.chunks(chunk).enumerate() {
                if seg.len() < 2 {
                    continue;
                }
                let t0 = (k * chunk) as f32 / path.len() as f32;
                let alpha = (base_alpha * (1.0 - t0 * 0.9)) as u8;
                let screen: Vec<egui::Pos2> = seg
                    .iter()
                    .filter_map(|&p| project(p).map(|pr| pr.pos))
                    .collect();
                if screen.len() >= 2 {
                    painter.add(egui::Shape::line(
                        screen,
                        egui::Stroke::new(
                            if is_focus { 1.8 } else { 1.1 },
                            color.gamma_multiply(1.05).linear_multiply(alpha as f32 / 255.0),
                        ),
                    ));
                }
            }
            if let (Some(last), Some(body)) = (path.last(), bodies.iter().find(|b| b.id == *id)) {
                if let (Some(pl), Some(pb)) = (
                    project(*last).map(|pr| pr.pos),
                    project(body.pos).map(|pr| pr.pos),
                ) {
                    // Точка-маркер конца пути (куда прилетит тело).
                    painter.circle_filled(pl, 2.0, color.gamma_multiply(0.5));
                    if is_focus {
                        painter.line_segment([pb, pl], egui::Stroke::new(0.8, color.gamma_multiply(0.35)));
                    }
                }
            }
        }
    }

    // ── Предсказание орбиты выбранного тела ─────────────────────
    if app.show_prediction && !replay_active {
        update_prediction(app, &bodies, tick);
        if let Some(pred) = &app.prediction {
            let color = colors_by_id
                .get(&pred.id)
                .copied()
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

    // ── Области мультипольного разбиения ────────────────────────
    if app.show_regions && !regions.is_empty() {
        draw_regions(&painter, &|p| project(p).map(|pr| pr.pos), &regions);
    }

    // ── Ускорения тел (для стрелок и раскраски) ─────────────────
    // Точный расчёт O(N²) на кадр тянем только когда он действительно
    // нужен: выключенные стрелки сил и обычная раскраска его не требуют.
    let need_accels = app.show_forces || app.color_mode == ColorMode::ByGravity;
    let accels: Vec<Vec3> = if need_accels {
        bodies
            .iter()
            .map(|b| accel_at(&bodies, b.pos, g, softening))
            .collect()
    } else {
        Vec::new()
    };

    // ── Тела (в 3D — сортировка по глубине, дальние раньше) ─────
    // Глубины считаются один раз, а не при каждом сравнении в сортировке.
    let mut order: Vec<usize> = (0..bodies.len()).collect();
    if app.view3d {
        let depths: Vec<f64> = bodies
            .iter()
            .map(|b| project(b.pos).map(|p| p.depth).unwrap_or(f64::INFINITY))
            .collect();
        order.sort_by(|&i, &j| depths[j].total_cmp(&depths[i]));
    }

    let focal = cam3.focal(rect);
    for i in order {
        let body = &bodies[i];
        let Some(pr) = project(body.pos) else {
            continue;
        };
        let sp = pr.pos;
        if !rect.expand(60.0).contains(sp) {
            continue;
        }
        let scale = if app.view3d {
            (focal / pr.depth.max(0.1)).clamp(0.0, 1.0e6)
        } else {
            cam.zoom
        };
        let radius = (body.radius * scale).clamp(2.0, rect.height().max(1.0) as f64 * 1.5) as f32;
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
            let Some(pr) = project(body.pos) else {
                continue;
            };
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
            let Some(pr_tip) = project(tip_world) else {
                continue;
            };
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
                "{mode} | {} | UI: {:5.1} fps | физика: {:5.2} мс/шаг | тел: {} | t = {:.1}",
                app.scenario.name(),
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
        hints.push("ЛКМ — орбита · ПКМ — панорама · Shift+ЛКМ — рамка · колесо — зум");
    }
    if app.tool == Tool::Orbit {
        hints.push(if app.orbit_host.is_some() {
            "Быстрая орбита: клик — разместить тело вокруг выбранной планеты"
        } else {
            "Быстрая орбита: клик по планете-хозяину"
        });
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

    // ── Ввод: зум, панорама, инструменты, рамка выделения ───────
    handle_input(
        ui,
        &painter,
        &response,
        rect,
        app,
        &bodies,
        &project,
        well_rect,
    );
}
