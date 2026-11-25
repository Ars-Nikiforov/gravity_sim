//! Логика камеры и предсказаний: слежение и авто-подгонка камеры, кэш
//! «призрака» выбранного тела и кэш орбит-предсказаний всех тел.

use std::collections::HashSet;

use eframe::egui;

use crate::app::{FocusMode, GravityApp, barycenter, barycenter_filtered};
use crate::physics::{Body, Vec2, Vec3};
use super::{ORBIT_BODY_LIMIT, ORBIT_REFRESH_TICKS, ORBIT_TOP_MASSIVE, PREDICT_DT_SCALE, PREDICT_STEPS};
use crate::ui::canvas::camera::{Camera, Camera3D};

/// Кэш предсказания траектории выбранного тела.
pub struct Prediction {
    pub id: u64,
    pub tick: u64,
    pub pts: Vec<Vec3>,
}

/// Подтягиваем камеру к цели слежения, автоцентру и автомасштабу.
/// Сглаживание экспоненциальное и не зависит от частоты кадров.
pub(super) fn update_camera(app: &mut GravityApp, bodies: &[Body], rect: egui::Rect, dt: f64) {
    let view3d = app.view3d;
    let dt = dt.clamp(1e-4, 0.1);

    // 0. Кадрирование после сброса/загрузки пресета: сразу вписываем сцену,
    //    без «прыжка» из начала координат на первых кадрах.
    if app.frame_pending && !bodies.is_empty() {
        let c = barycenter(bodies);
        let radius = robust_radius(bodies, c, view3d);
        app.camera.center = c.xy();
        app.cam3d.target = c;
        app.camera.zoom =
            (rect.width().min(rect.height()) as f64 * 0.42 / radius).clamp(0.02, 40.0);
        let fov = (app.cam3d.fov_y as f64).clamp(0.15, 2.8);
        app.cam3d.distance = (radius / (fov * 0.5).tan()).clamp(10.0, 1.0e7) * 1.15;
        app.frame_pending = false;
    }

    // 1. Слежение за телом/системой/выделением — плавно, с постоянной времени.
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
        let k = smooth(dt, 0.12);
        app.cam3d.target = app.cam3d.target.lerp(t, k);
        app.camera.center = app.camera.center.lerp(t.xy(), k);
    } else if app.auto_center && !bodies.is_empty() {
        // Мёртвая зона: камера не дёргается, пока система в кадре.
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
            if dist > visible * 0.65 {
                // Чем дальше, тем быстрее догоняем, но без телепорта.
                let tau = if dist > visible * 2.0 { 0.22 } else { 0.45 };
                let k = smooth(dt, tau);
                app.cam3d.target = app.cam3d.target.lerp(c, k);
                app.camera.center = app.camera.center.lerp(c.xy(), k);
            }
        }
    }

    // 2. Автомасштаб: вписать ~90% тел, плавно.
    if app.auto_fit && !bodies.is_empty() {
        let center = if view3d {
            app.cam3d.target
        } else {
            app.camera.center.to3()
        };
        let radius = robust_radius(bodies, center, view3d);
        let k = smooth(dt, 0.35);
        if view3d {
            let fov = (app.cam3d.fov_y as f64).clamp(0.15, 2.8);
            let desired = (radius / (fov * 0.5).tan()).clamp(10.0, 1.0e7);
            app.cam3d.distance += (desired - app.cam3d.distance) * k;
        } else {
            let desired =
                (rect.width().min(rect.height()) as f64 * 0.42 / radius).clamp(0.02, 40.0);
            app.camera.zoom += (desired - app.camera.zoom) * k;
        }
    }
}

/// Коэффициент экспоненциального сглаживания, независимый от FPS.
fn smooth(dt: f64, tau: f64) -> f64 {
    1.0 - (-dt / tau.max(1e-4)).exp()
}

/// Область мира на плоскости z = 0, которую покрывает тепловая карта.
pub(super) fn heatmap_bounds(cam: Camera, cam3: Camera3D, rect: egui::Rect, view3d: bool) -> (Vec2, Vec2) {
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

/// Обновить кэш орбит-предсказаний всех тел: одна симуляция вперёд даёт
/// пути сразу для всех. Пересчёт не чаще раза в ORBIT_REFRESH_TICKS тиков.
/// В скоплениях (включено слияние) пути считаются только для самых
/// массивных тел и выделенных — сотни мелких путей сливаются в шум.
pub(super) fn update_orbits(app: &mut GravityApp, bodies: &[Body], tick: u64) {
    let need = tick < app.orbits_tick
        || tick.saturating_sub(app.orbits_tick) >= ORBIT_REFRESH_TICKS;
    if !need {
        return;
    }
    // Горизонт по выбранному телу, иначе по самому массивному.
    let focus = app
        .selected_id
        .and_then(|id| bodies.iter().position(|b| b.id == id))
        .or_else(|| {
            bodies
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.mass.total_cmp(&b.mass))
                .map(|(i, _)| i)
        });
    let Some(focus) = focus else {
        app.orbits.clear();
        return;
    };
    let (steps, dt) = predict_step(app, bodies, focus);
    let paths = crate::physics::predict_paths(bodies, app.params, steps, dt);

    // Кому рисовать: всем, либо (в скоплениях) самым массивным + выделенным.
    let mut keep: Option<HashSet<u64>> = None;
    if app.merge_small_orbits && bodies.len() > ORBIT_BODY_LIMIT {
        let mut order: Vec<(f64, u64)> =
            bodies.iter().map(|b| (b.mass, b.id)).collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut set: HashSet<u64> = order
            .into_iter()
            .take(ORBIT_TOP_MASSIVE)
            .map(|(_, id)| id)
            .collect();
        set.extend(app.selection.iter().copied());
        if let Some(sel) = app.selected_id {
            set.insert(sel);
        }
        keep = Some(set);
    }
    app.orbits = paths
        .into_iter()
        .zip(bodies.iter())
        .filter(|(_, b)| keep.as_ref().is_none_or(|k| k.contains(&b.id)))
        .map(|(path, b)| (b.id, path))
        .collect();
    app.orbits_tick = tick;
}

/// Обновить кэш предсказания: пересчёт не чаще раза в 30 тиков, при смене
/// выбранного тела или сбросе тика (новый сценарий).
pub(super) fn update_prediction(app: &mut GravityApp, bodies: &[Body], tick: u64) {
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
    app.prediction = Some(Prediction { id: sel, tick, pts });
}

