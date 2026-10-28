//! Разбиение на области (uniform grid, 3D) для ускорения расчёта сил.
//!
//! Идея: ближние ячейки (3×3×3) считаются точно попарно, дальние ячейки
//! схлопываются в точечную массу в центре масс ячейки. Это O(N·C) вместо
//! O(N²): C — число ячеек, которое растёт медленнее N при адаптивном шаге.
//!
//! Порядок обхода детерминирован (BTreeMap), поэтому при одинаковых
//! входных данных результат воспроизводится бит в бит.
//!
//! Для копланарных систем (z = 0) третий индекс ячейки всегда нулевой —
//! сетка вырождается в 2D автоматически.

use std::collections::BTreeMap;

use super::sim::Body;
use super::vec::Vec3;

type CellKey = (i64, i64, i64);

struct Cell {
    /// Индексы тел в ячейке.
    idxs: Vec<usize>,
    /// Суммарная масса и центр масс ячейки.
    mass: f64,
    com: Vec3,
}

/// Ускорения всех тел в заданных позициях (позиции могут отличаться от
/// позиций тел — это нужно RK4 для пробных состояний).
pub fn accel_grid(bodies: &[Body], pos: &[Vec3], g: f64, softening: f64) -> Vec<Vec3> {
    let n = bodies.len();
    let mut acc = vec![Vec3::ZERO; n];
    if n == 0 {
        return acc;
    }

    // Габарит области и адаптивный размер ячейки.
    let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in pos {
        min.x = min.x.min(p.x);
        min.y = min.y.min(p.y);
        min.z = min.z.min(p.z);
        max.x = max.x.max(p.x);
        max.y = max.y.max(p.y);
        max.z = max.z.max(p.z);
    }
    let diag = (max - min).len().max(1.0);
    let cell_size = (diag / 14.0).clamp(40.0, 1e6);

    let key_of = |p: Vec3| -> CellKey {
        (
            (p.x / cell_size).floor() as i64,
            (p.y / cell_size).floor() as i64,
            (p.z / cell_size).floor() as i64,
        )
    };

    // Строим ячейки.
    let mut cells: BTreeMap<CellKey, Cell> = BTreeMap::new();
    for (i, b) in bodies.iter().enumerate() {
        let e = cells
            .entry(key_of(pos[i]))
            .or_insert_with(|| Cell { idxs: Vec::new(), mass: 0.0, com: Vec3::ZERO });
        e.idxs.push(i);
        e.mass += b.mass;
        e.com += pos[i] * b.mass;
    }
    for cell in cells.values_mut() {
        if cell.mass > 0.0 {
            cell.com = cell.com * (1.0 / cell.mass);
        }
    }

    // Ближние ячейки — точный попарный расчёт; дальние — точечные массы.
    for (key, cell) in &cells {
        for &i in &cell.idxs {
            let pi = pos[i];
            let mut a = acc[i];
            // 3×3×3 окрестность своей ячейки.
            for dx in -1..=1i64 {
                for dy in -1..=1i64 {
                    for dz in -1..=1i64 {
                        let near = cells.get(&(key.0 + dx, key.1 + dy, key.2 + dz));
                        let Some(near) = near else { continue };
                        for &j in &near.idxs {
                            if i == j {
                                continue;
                            }
                            let d = pos[j] - pi;
                            let r2 = d.len2() + softening;
                            a += d * (g * bodies[j].mass / (r2 * r2.sqrt()));
                        }
                    }
                }
            }
            // Остальные ячейки — как точечные массы в их центрах масс.
            for (far_key, far) in &cells {
                let (ddx, ddy, ddz) =
                    (far_key.0 - key.0, far_key.1 - key.1, far_key.2 - key.2);
                if ddx.abs() <= 1 && ddy.abs() <= 1 && ddz.abs() <= 1 {
                    continue;
                }
                let d = far.com - pi;
                let r2 = d.len2() + softening;
                a += d * (g * far.mass / (r2 * r2.sqrt()));
            }
            acc[i] = a;
        }
    }
    acc
}
