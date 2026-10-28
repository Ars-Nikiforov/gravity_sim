//! Повтор симуляции по логу: загрузка JSONL-лога, раскодировка дельт
//! в последовательность полных состояний и проигрывание их на холсте.
//! Из любой отметки можно продолжить живую симуляцию (состояние тел
//! отправляется в поток физики).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::logging::{BodyLog, LogRecord, RecordPayload};
use crate::physics::Body;

/// Одно полное состояние из лога.
pub struct Frame {
    pub tick: u64,
    pub sim_time: f64,
    pub bodies: Vec<Body>,
}

pub struct ReplayPlayer {
    pub file: PathBuf,
    frames: Vec<Frame>,
    pub idx: usize,
    pub playing: bool,
    /// Скорость повтора относительно реального времени.
    pub speed: f32,
    /// Дробный прогресс между кадрами — для плавного темпа.
    frac: f32,
    /// Средний шаг времени между кадрами (из лога), сек.
    frame_dt: f32,
    /// Пропущенных битых строк при загрузке.
    pub skipped_lines: usize,
}

impl ReplayPlayer {
    /// Загрузить лог и раскодировать в кадры.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("не удалось прочитать файл: {e}"))?;
        let mut frames = Vec::new();
        let mut current: HashMap<u64, BodyLog> = HashMap::new();
        let mut skipped = 0usize;
        // Шаг времени между кадрами берём из первых записей.
        let mut frame_dt = 1.0 / 60.0;
        let mut dt_hint: Option<f64> = None;
        let mut first_tick: Option<u64> = None;
        let mut second_tick: Option<u64> = None;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let record: LogRecord = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            match record.payload {
                RecordPayload::Meta { dt, .. } => dt_hint = Some(dt),
                RecordPayload::Keyframe { bodies } => {
                    current = bodies.into_iter().map(|b| (b.id, b)).collect();
                    push_frame(&mut frames, record.tick, record.sim_time, &current);
                }
                RecordPayload::Delta {
                    deltas,
                    added,
                    removed,
                } => {
                    for d in deltas {
                        if let Some(b) = current.get_mut(&d.id) {
                            b.x += d.dx;
                            b.y += d.dy;
                            b.z += d.dz;
                            b.vx += d.dvx;
                            b.vy += d.dvy;
                            b.vz += d.dvz;
                        }
                    }
                    for b in added {
                        current.insert(b.id, b);
                    }
                    for id in removed {
                        current.remove(&id);
                    }
                    push_frame(&mut frames, record.tick, record.sim_time, &current);
                }
            }
            if first_tick.is_none() {
                first_tick = Some(record.tick);
            } else if second_tick.is_none() {
                second_tick = Some(record.tick);
            }
        }

        if frames.is_empty() {
            return Err("лог не содержит кадров".into());
        }
        // Темп повтора: dt из meta × интервал тиков между кадрами.
        if let (Some(a), Some(b), Some(dt)) = (first_tick, second_tick, dt_hint) {
            let gap = b.saturating_sub(a).max(1) as f64;
            frame_dt = (gap * dt) as f32;
        }

        Ok(Self {
            file: path.to_path_buf(),
            frames,
            idx: 0,
            playing: true,
            speed: 1.0,
            frac: 0.0,
            frame_dt,
            skipped_lines: skipped,
        })
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn current(&self) -> &Frame {
        &self.frames[self.idx.min(self.frames.len() - 1)]
    }

    /// Продвинуть плеер на dt реального времени. Возвращает true, если
    /// индекс кадра изменился (используется, чтобы не дублировать следы).
    pub fn advance(&mut self, dt_real: f32) -> bool {
        if !self.playing {
            return false;
        }
        let frame_dt = self.frame_dt.max(1e-4);
        self.frac += dt_real * self.speed / frame_dt;
        let mut moved = false;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            if self.idx + 1 < self.frames.len() {
                self.idx += 1;
                moved = true;
            } else {
                // Конец лога — останавливаемся на последнем кадре.
                self.playing = false;
                self.frac = 0.0;
                break;
            }
        }
        moved
    }

    /// Сбросить дробный прогресс (вызывается после ручной перемотки слайдером).
    pub fn note_scrub(&mut self) {
        self.frac = 0.0;
    }
}

fn push_frame(frames: &mut Vec<Frame>, tick: u64, sim_time: f64, current: &HashMap<u64, BodyLog>) {
    let mut bodies: Vec<Body> = current.values().cloned().map(Body::from).collect();
    // Стабильный порядок по id — важен для следов и списка тел.
    bodies.sort_unstable_by_key(|b| b.id);
    frames.push(Frame {
        tick,
        sim_time,
        bodies,
    });
}

/// Последний (по времени изменения) файл лога в каталоге logs/.
pub fn latest_log(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p)
}
