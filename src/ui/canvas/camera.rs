//! Камеры холста: 2D «вид сверху» и перспективная 3D-камера
//! (look-at + сферические углы + дистанция), результат проекции
//! [`Projected`]. Тесты обратимости проекций — здесь же.

use eframe::egui;

use crate::physics::{Vec2, Vec3};

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

    /// Вращение вокруг цели. Возвышение ограничено только чуть-чуть вдали от
    /// вертикали (±88.6°): камера свободно уходит под плоскость z = 0 и почти
    /// в зенит; ровно в полюсе базис вырожден, поэтому туда не пускаем.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw += dx * 0.01;
        self.pitch = (self.pitch - dy * 0.01).clamp(-1.547, 1.547);
    }

    /// Панорама: цель едет в плоскости камеры («хватаем» сцену мышью).
    pub fn pan(&mut self, rect: egui::Rect, dx: f32, dy: f32) {
        let scale =
            2.0 * self.distance * (self.fov_y as f64 * 0.5).tan() / rect.height().max(1.0) as f64;
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
