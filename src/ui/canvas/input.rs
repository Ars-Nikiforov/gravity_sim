//! Взаимодействие с холстом: зум колесом, панорама/орбита камеры,
//! рамка выделения, инструменты (выбор, добавление, быстрая орбита,
//! удаление), превью быстрой орбиты. Вызывается из `super::show`.

use eframe::egui;

use crate::app::{FocusMode, GravityApp, Tool};
use crate::physics::{Body, Command, Vec3, orbit_plan};
use crate::ui::canvas::camera::Projected;
use crate::ui::canvas::draw::draw_orbit_preview;

pub(super) fn handle_input(
    ui: &egui::Ui,
    painter: &egui::Painter,
    response: &egui::Response,
    rect: egui::Rect,
    app: &mut GravityApp,
    bodies: &[Body],
    project: &dyn Fn(Vec3) -> Option<Projected>,
    well_rect: Option<egui::Rect>,
) {
    // ── Взаимодействие ──────────────────────────────────────────
    let cursor = ui.input(|i| i.pointer.hover_pos());
    let over_well = well_rect.map_or(false, |r| cursor.map_or(false, |c| r.contains(c)));
    let shift = ui.input(|i| i.modifiers.shift);

    // Зум колесом.
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll != 0.0 && !over_well {
        if let Some(cursor) = cursor {
            if rect.contains(cursor) {
                let factor = (scroll as f64 * 0.0025).exp();
                if app.view3d {
                    // Зум к курсору: точка под курсором остаётся на месте.
                    let anchor = app.cam3d.unproject_view_plane(rect, cursor);
                    let before = app.cam3d.distance.max(1e-9);
                    app.cam3d.zoom_by(factor);
                    let ratio = app.cam3d.distance / before;
                    app.cam3d.target = anchor + (app.cam3d.target - anchor) * ratio;
                } else {
                    let before = app.camera.screen_to_world(rect, cursor);
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
            app.box_select_additive = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
        }
    }
    let box_active = app.box_select_start.is_some();

    // Панорама/орбита (рамка выделения блокирует панораму).
    let primary_pan = response.dragged_by(egui::PointerButton::Primary) && !over_well && !shift;
    let secondary_drag = response.dragged_by(egui::PointerButton::Secondary) && !over_well;
    let middle_drag = response.dragged_by(egui::PointerButton::Middle) && !over_well;

    if !box_active {
        if app.view3d {
            if primary_pan {
                // ЛКМ — вращение вокруг цели (как в 3D-редакторах).
                let d = response.drag_delta();
                app.cam3d.orbit(d.x, d.y);
                if app.focus_mode != FocusMode::Free {
                    app.focus_mode = FocusMode::Free;
                }
            } else if secondary_drag || middle_drag {
                // ПКМ/СКМ — панорама в плоскости камеры.
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
        for b in bodies {
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
                    if let Some(world) = click_world(app, rect, pos) {
                        let nb = app.new_body;
                        app.physics.send(Command::AddBody {
                            mass: nb.mass,
                            radius: nb.radius,
                            color: nb.color,
                            pos: world,
                        });
                    }
                }
                Tool::Orbit => {
                    if let Some(body) = hit {
                        // Первый клик — выбрать хозяина орбиты.
                        app.orbit_host = Some(body.id);
                        app.selected_id = Some(body.id);
                        app.selection.clear();
                        app.selection.insert(body.id);
                    } else if let Some(host_id) = app.orbit_host {
                        if let Some(host_idx) = bodies.iter().position(|b| b.id == host_id) {
                            if let Some(world) = click_world(app, rect, pos) {
                                let nb = app.new_body;
                                if let Some(plan) =
                                    orbit_plan(&bodies, host_idx, world, nb.mass, app.params.g)
                                {
                                    app.physics.send(Command::AddBodyState {
                                        mass: nb.mass,
                                        radius: nb.radius,
                                        color: nb.color,
                                        pos: plan.pos,
                                        vel: plan.vel,
                                    });
                                }
                            }
                        }
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

    // ── Превью быстрой орбиты ──────────────────────────────────
    if app.tool == Tool::Orbit && !over_well {
        if let Some(cursor) = cursor.filter(|c| rect.contains(*c)) {
            if let Some(host_id) = app.orbit_host {
                if let Some(host_idx) = bodies.iter().position(|b| b.id == host_id) {
                    let host = &bodies[host_idx];
                    if let Some(pr) = project(host.pos) {
                        painter.circle_stroke(
                            pr.pos,
                            9.0,
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(120, 235, 205)),
                        );
                    }
                    if let Some(world) = click_world(app, rect, cursor) {
                        if let Some(plan) =
                            orbit_plan(&bodies, host_idx, world, app.new_body.mass, app.params.g)
                        {
                            draw_orbit_preview(
                                &painter,
                                &|p| project(p).map(|pr| pr.pos),
                                host.pos,
                                &plan,
                            );
                        }
                    }
                }
            } else if let Some(body) = pick_body(&bodies, &project, cursor) {
                if let Some(pr) = project(body.pos) {
                    painter.circle_stroke(
                        pr.pos,
                        11.0,
                        egui::Stroke::new(1.5, egui::Color32::from_rgb(150, 220, 255)),
                    );
                }
            }
        }
    }
}

/// Экран → мир для клика: сверху (в т.ч. из-под плоскости) — на плоскость
/// z = 0, с ребра — на плоскость взгляда (иначе луч уходит в бесконечность).
fn click_world(app: &GravityApp, rect: egui::Rect, sp: egui::Pos2) -> Option<Vec3> {
    if app.view3d {
        if app.cam3d.pitch.abs() > 0.35 {
            app.cam3d
                .unproject_ground(rect, sp)
                .or_else(|| Some(app.cam3d.unproject_view_plane(rect, sp)))
        } else {
            Some(app.cam3d.unproject_view_plane(rect, sp))
        }
    } else {
        Some(app.camera.screen_to_world(rect, sp).to3())
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
