//! Асинхронное логирование симуляции.
//!
//! Поток физики только формирует записи и отправляет их в канал —
//! сериализацией и записью в файл занимается отдельный поток-писатель,
//! поэтому тяжёлый I/O никогда не тормозит ни физику, ни интерфейс.
//!
//! Формат — JSON Lines (`logs/sim_ГГГГММДД_ЧЧММСС.jsonl`):
//! * первая строка — `meta` (параметры на момент старта);
//! * далее `keyframe` (полное состояние) и `delta` (только изменения:
//!   смещения позиций/скоростей, добавленные и удалённые тела).
//!   Ключевые кадры вставляются периодически, чтобы лог можно было
//!   читать с произвольного места.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::physics::{Body, SimParams, Vec3};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogMode {
    #[default]
    Off,
    /// Только изменения относительно предыдущей записи.
    Deltas,
    /// Полное состояние в каждой записи.
    Full,
}

impl LogMode {
    pub fn name(self) -> &'static str {
        match self {
            LogMode::Off => "Выкл",
            LogMode::Deltas => "Дельты",
            LogMode::Full => "Полный",
        }
    }
}

/// Тело в логе: то же, что физическое тело, но плоско и сериализуемо.
/// z/vz с `default` — совместимость с логами, записанными в 2D-эпоху.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BodyLog {
    pub id: u64,
    pub mass: f64,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    #[serde(default)]
    pub vz: f64,
    pub radius: f64,
    pub color: [u8; 3],
}

impl From<&Body> for BodyLog {
    fn from(b: &Body) -> Self {
        Self {
            id: b.id,
            mass: b.mass,
            x: b.pos.x,
            y: b.pos.y,
            z: b.pos.z,
            vx: b.vel.x,
            vy: b.vel.y,
            vz: b.vel.z,
            radius: b.radius,
            color: b.color,
        }
    }
}

impl From<BodyLog> for Body {
    fn from(b: BodyLog) -> Self {
        Body {
            id: b.id,
            mass: b.mass,
            pos: Vec3::new(b.x, b.y, b.z),
            vel: Vec3::new(b.vx, b.vy, b.vz),
            radius: b.radius,
            color: b.color,
        }
    }
}

/// Изменение одного тела между записями.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BodyDelta {
    pub id: u64,
    pub dx: f64,
    pub dy: f64,
    #[serde(default)]
    pub dz: f64,
    pub dvx: f64,
    pub dvy: f64,
    #[serde(default)]
    pub dvz: f64,
}

/// Содержимое записи лога.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "rec", rename_all = "snake_case")]
pub enum RecordPayload {
    Meta {
        dt: f64,
        g: f64,
        algorithm: String,
        mode: String,
    },
    Keyframe {
        bodies: Vec<BodyLog>,
    },
    Delta {
        deltas: Vec<BodyDelta>,
        added: Vec<BodyLog>,
        removed: Vec<u64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogRecord {
    pub tick: u64,
    pub sim_time: f64,
    #[serde(flatten)]
    pub payload: RecordPayload,
}

impl LogRecord {
    pub fn meta(params: &SimParams, mode: LogMode) -> Self {
        Self {
            tick: 0,
            sim_time: 0.0,
            payload: RecordPayload::Meta {
                dt: params.dt,
                g: params.g,
                algorithm: params.algorithm.name().to_string(),
                mode: mode.name().to_string(),
            },
        }
    }

    pub fn keyframe(tick: u64, sim_time: f64, bodies: &[Body]) -> Self {
        Self {
            tick,
            sim_time,
            payload: RecordPayload::Keyframe {
                bodies: bodies.iter().map(BodyLog::from).collect(),
            },
        }
    }
}

/// Handle писателя. Живёт в приложении; `sender` передаётся потоку физики.
pub struct Logger {
    tx: Option<Sender<LogRecord>>,
    pub file: PathBuf,
    pub records: Arc<AtomicU64>,
    writer: Option<JoinHandle<()>>,
}

impl Logger {
    /// Создать файл лога и запустить поток-писатель.
    pub fn start(params: &SimParams, mode: LogMode) -> std::io::Result<Self> {
        fs::create_dir_all("logs")?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Простая человекочитаемая метка времени UTC без зависимостей.
        let (y, mo, d, h, mi, s) = civil_from_unix(stamp);
        let file = PathBuf::from(format!(
            "logs/sim_{:04}{:02}{:02}_{:02}{:02}{:02}.jsonl",
            y, mo, d, h, mi, s
        ));
        let out = File::create(&file)?;
        let mut writer = BufWriter::with_capacity(256 * 1024, out);

        let meta = LogRecord::meta(params, mode);
        serde_json::to_writer(&mut writer, &meta)?;
        writer.write_all(b"\n")?;
        writer.flush()?;

        let (tx, rx) = std::sync::mpsc::channel::<LogRecord>();
        let records = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&records);
        let handle = std::thread::Builder::new()
            .name("log-writer".into())
            .spawn(move || writer_loop(rx, writer, counter))?;

        Ok(Self {
            tx: Some(tx),
            file,
            records,
            writer: Some(handle),
        })
    }

    pub fn sender(&self) -> Option<Sender<LogRecord>> {
        self.tx.clone()
    }

    pub fn record_count(&self) -> u64 {
        self.records.load(Ordering::Relaxed)
    }

    /// Остановить: канал закрывается, писатель дописывает буфер и выходит.
    pub fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            drop(tx);
        }
        if let Some(handle) = self.writer.take() {
            let _ = handle.join();
        }
    }
}

fn writer_loop(rx: Receiver<LogRecord>, mut writer: BufWriter<File>, counter: Arc<AtomicU64>) {
    let mut pending = 0u32;
    while let Ok(record) = rx.recv() {
        if serde_json::to_writer(&mut writer, &record).is_ok() {
            let _ = writer.write_all(b"\n");
            counter.fetch_add(1, Ordering::Relaxed);
        }
        // Флаш раз в 32 записи: комфортный баланс между стойкостью к краху
        // и стоимостью системных вызовов.
        pending += 1;
        if pending >= 32 {
            let _ = writer.flush();
            pending = 0;
        }
    }
    let _ = writer.flush();
}

/// Перевод unix-секунд в гражданскую дату (алгоритм Говарда Хинниканта).
fn civil_from_unix(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = ((rem / 3600) as u32, ((rem % 3600) / 60) as u32, (rem % 60) as u32);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32, h, mi, s)
}
