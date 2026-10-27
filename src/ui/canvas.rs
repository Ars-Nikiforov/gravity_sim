//! Центральный дисплей: отрисовка тел из последнего снапшота физики
//! и взаимодействие (зум к курсору, панорама, инструменты).

use eframe::egui;

use crate::app::{GravityApp, Tool};
use crate::physics::{Command, Snapshot, Vec2};

/// Камера: преобразование мир ↔ экран. `zoom` — пикселей на мировую единицу.
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

/// Сколько точек следа хранит каждое тело.
const TRAIL_LEN: usize = 240;

pub fn show(ui: &mut egui::Ui, app: &mut GravityApp) {
    let (response, painter) = ui.allocate_painter(
        ui.available_size(),
        egui::Sense::click_and_drag(),
    );
    let rect = response.rect;

    // Космический фон.
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(10, 10, 16));

    // Всегда свежий снапшот: Arc-клон, без ожидания потока физики.
    let snapshot = app.physics.snapshot();
    let cam = app.camera;

    // ── Следы орбит (история на стороне UI) ─────────────────────
    if app.show_trails {
        if snapshot.running {
            for body in &snapshot.bodies {
                app.trails
                    .entry(body.id)
                    .or_default()
                    .push_back(body.pos);
                let trail = app.trails.get_mut(&body.id).unwrap();
                while trail.len() > TRAIL_LEN {
                    trail.pop_front();
                }
            }
        }
        // Чистим следы удалённых тел.
        let alive: std::collections::HashSet<u64> =
            snapshot.bodies.iter().map(|b| b.id).collect();
        app.trails.retain(|id, _| alive.contains(id));

        for (id, points) in &app.trails {
            if points.len() < 2 {
                continue;
            }
            let color = snapshot
                .bodies
                .iter()
                .find(|b| b.id == *id)
                .map(|b| {
                    egui::Color32::from_rgba_unmultiplied(
                        b.color[0],
                        b.color[1],
                        b.color[2],
                        70,
                    )
                })
                .unwrap_or(egui::Color32::GRAY);
            let screen_points: Vec<egui::Pos2> = points
                .iter()
                .map(|&p| cam.world_to_screen(rect, p))
                .collect();
            painter.add(egui::Shape::line(
                screen_points,
                egui::Stroke::new(1.0, color),
            ));
        }
    }

    // ── Тела ────────────────────────────────────────────────────
    for body in &snapshot.bodies {
        let sp = cam.world_to_screen(rect, body.pos);
        if !rect.expand(40.0).contains(sp) {
            continue;
        }
        let radius = (body.radius * cam.zoom).max(1.5) as f32;
        painter.circle_filled(
            sp,
            radius,
            egui::Color32::from_rgb(body.color[0], body.color[1], body.color[2]),
        );
        // Тонкое затемняющее ядро у массивных тел — дешёвая «объёмность».
        if body.mass >= 100.0 {
            painter.circle_filled(sp, radius * 0.55, egui::Color32::WHITE);
        }
        if app.selected_id == Some(body.id) {
            let ring = egui::Color32::from_rgb(120, 220, 255);
            painter.circle_stroke(sp, radius + 4.0, egui::Stroke::new(2.0, ring));
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

    // ── Оверлей FPS ─────────────────────────────────────────────
    if app.show_fps {
        painter.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "UI: {:5.1} fps | физика: {:5.2} мс/шаг | тел: {}",
                app.fps,
                snapshot.step_ms,
                snapshot.bodies.len()
            ),
            egui::FontId::monospace(12.0),
            egui::Color32::from_gray(170),
        );
    }

    // ── Взаимодействие ──────────────────────────────────────────
    // Зум колесом к позиции курсора.
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if scroll != 0.0 {
        if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(cursor) {
                let before = cam.screen_to_world(rect, cursor);
                let factor = (scroll * 0.01).exp() as f64;
                app.camera.zoom = (app.camera.zoom * factor).clamp(0.02, 40.0);
                let after = app.camera.screen_to_world(rect, cursor);
                app.camera.center = app.camera.center + (before - after);
            }
        }
    }

    // Панорама: ПКМ/СКМ в любом инструменте, ЛКМ — в режиме «Выбор».
    let primary_pan = app.tool == Tool::Select
        && response.dragged_by(egui::PointerButton::Primary);
    if primary_pan
        || response.dragged_by(egui::PointerButton::Secondary)
        || response.dragged_by(egui::PointerButton::Middle)
    {
        let d = response.drag_delta();
        app.camera.center.x -= d.x as f64 / app.camera.zoom;
        app.camera.center.y -= d.y as f64 / app.camera.zoom;
    }

    // Клик — применить инструмент.
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let world = cam.screen_to_world(rect, pos);
            let hit = pick_body(&snapshot, cam, rect, pos);
            match app.tool {
                Tool::Select => app.selected_id = hit.map(|b| b.id),
                Tool::AddBody => {
                    let nb = app.new_body;
                    app.physics.send(Command::AddBody {
                        mass: nb.mass,
                        radius: nb.radius,
                        color: nb.color,
                        pos: world,
                    });
                }
                Tool::DeleteBody => {
                    if let Some(body) = hit {
                        app.physics.send(Command::RemoveBody(body.id));
                        if app.selected_id == Some(body.id) {
                            app.selected_id = None;
                        }
                    }
                }
            }
        }
    }
}

/// Поиск тела под курсором (в экранных координатах).
fn pick_body(
    snapshot: &Snapshot,
    cam: Camera,
    rect: egui::Rect,
    pos: egui::Pos2,
) -> Option<&crate::physics::Body> {
    snapshot
        .bodies
        .iter()
        .filter(|b| {
            let sp = cam.world_to_screen(rect, b.pos);
            let radius = (b.radius * cam.zoom).max(1.5) as f32;
            (sp - pos).length() <= (radius + 6.0).max(10.0)
        })
        .min_by_key(|b| {
            let sp = cam.world_to_screen(rect, b.pos);
            ((sp - pos).length() * 10.0) as i32
        })
}
