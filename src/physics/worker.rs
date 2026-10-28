//! Связка движка с интерфейсом: физика живёт в отдельном потоке.
//!
//! Схема асинхронности:
//!   UI-поток  --Command-->  поток физики  --Arc<Snapshot>-->  UI-поток (отрисовка)
//!
//! * UI никогда не ждёт расчёта: он всегда рисует последний готовый снапшот.
//! * Поток физики тикает с фиксированной частотой и сам просит перерисовку
//!   через `Context::request_repaint` (Context потокобезопасен и клонируем).
//! * Закрытие окна дропает handle → канал разрывается → поток завершается.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::sim::{Body, Simulation};
use super::vec::Vec3;
use super::{Scenario, SimParams};
use crate::logging::{BodyDelta, BodyLog, LogMode, LogRecord, RecordPayload};

/// Команды из интерфейса в поток физики.
pub enum Command {
    SetParams(SimParams),
    SetRunning(bool),
    /// Один шаг вручную (работает и на паузе).
    StepOnce,
    Reset(Scenario),
    AddBody {
        mass: f64,
        radius: f64,
        color: [u8; 3],
        pos: Vec3,
    },
    RemoveBody(u64),
    SetBodyMass(u64, f64),
    Clear,
    /// Развернуть все скорости — система пойдёт назад по траектории.
    ReverseVelocities,
    /// Полная замена состояния (например, «продолжить с отметки лога»).
    ReplaceState(Vec<Body>),
    /// Настроить логирование: режим, период (в тиках) и канал писателя.
    SetLogging {
        mode: LogMode,
        every: u64,
        sender: Option<std::sync::mpsc::Sender<LogRecord>>,
    },
}

/// Неизменяемый кадр состояния для отрисовки.
pub struct Snapshot {
    pub bodies: Vec<Body>,
    pub sim_time: f64,
    pub tick: u64,
    /// EMA длительности шага физики, мс — видно, как интерфейс не тормозит
    /// даже когда шаг дороже кадра.
    pub step_ms: f32,
    pub running: bool,
    /// Поколение состояния: растёт при Reset/Clear/ReplaceState. UI по нему
    /// понимает, что id могли переиспользоваться, и сбрасывает следы.
    pub epoch: u64,
}

struct Shared {
    snapshot: Mutex<Arc<Snapshot>>,
}

/// Handle потока физики, живёт в приложении (UI-поток).
pub struct PhysicsWorker {
    tx: std::sync::mpsc::Sender<Command>,
    shared: Arc<Shared>,
    running: Arc<AtomicBool>,
}

/// Частота тика потока физики (не путать с dt интегрирования).
const TICK: Duration = Duration::from_millis(8);
/// Потолок шагов за один тик, чтобы после лага физика не пыталась «догнать»
/// бесконечным числом шагов.
const MAX_STEPS_PER_TICK: u32 = 64;

impl PhysicsWorker {
    pub fn spawn(ctx: egui::Context, params: SimParams, scenario: Scenario) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Command>();
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Arc::new(Snapshot {
                bodies: Vec::new(),
                sim_time: 0.0,
                tick: 0,
                step_ms: 0.0,
                running: true,
                epoch: 0,
            })),
        });
        let running = Arc::new(AtomicBool::new(true));

        let thread_shared = Arc::clone(&shared);
        let thread_running = Arc::clone(&running);
        let _ = thread::Builder::new()
            .name("physics".into())
            .spawn(move || {
                let mut sim = Simulation::new(params, scenario);
                // Аккумулятор переводит реальное время в симуляционное через params.speed.
                let mut accumulator = 0.0_f64;
                let mut last = Instant::now();
                let mut log = LogState::default();
                let mut epoch: u64 = 0;

                loop {
                    let mut dirty = false;

                    // 1. Забираем все накопившиеся команды без блокировки.
                    loop {
                        match rx.try_recv() {
                            Ok(cmd) => {
                                let new_epoch = matches!(
                                    &cmd,
                                    Command::Reset(_) | Command::Clear | Command::ReplaceState(_)
                                );
                                apply_command(&mut sim, &thread_running, cmd, &mut log);
                                if new_epoch {
                                    epoch = epoch.wrapping_add(1);
                                }
                                dirty = true;
                            }
                            Err(std::sync::mpsc::TryRecvError::Empty) => break,
                            // UI-приложение умерло — выходим.
                            Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
                        }
                    }

                    // 2. Считаем, сколько сим-времени должно пройти за реальный тик.
                    let now = Instant::now();
                    let real_dt = (now - last).as_secs_f64().min(0.25);
                    last = now;
                    let is_running = thread_running.load(Ordering::Relaxed);
                    if is_running {
                        accumulator += real_dt * sim.params().speed;
                        let dt = sim.params().dt;
                        let mut steps = 0u32;
                        while accumulator >= dt && steps < MAX_STEPS_PER_TICK {
                            sim.step();
                            accumulator -= dt;
                            steps += 1;
                        }
                        if steps == MAX_STEPS_PER_TICK {
                            accumulator = 0.0; // сбрасываем долг после лага
                        }
                        if steps > 0 {
                            dirty = true;
                        }
                    }

                    // 3. Логирование (не блокирует: запись делает другой поток).
                    let log_due = log.mode != LogMode::Off
                        && log.tx.is_some()
                        && sim.tick() != log.last_logged_tick
                        && (sim.tick() % log.every == 0
                            || log.since_keyframe >= KEYFRAME_EVERY);
                    if log_due {
                        log.last_logged_tick = sim.tick();
                        let record = build_log_record(&mut log, &sim);
                        if let Some(tx) = &log.tx {
                            let _ = tx.send(record);
                        }
                    }

                    // 4. Публикуем свежий снапшот и просим UI перерисоваться.
                    if dirty {
                        let snap = Arc::new(Snapshot {
                            bodies: sim.bodies().to_vec(),
                            sim_time: sim.sim_time(),
                            tick: sim.tick(),
                            step_ms: sim.step_ms(),
                            running: is_running,
                            epoch,
                        });
                        *thread_shared.snapshot.lock().unwrap() = snap;
                        ctx.request_repaint();
                    }

                    // 5. Спим до следующего тика.
                    let elapsed = last.elapsed();
                    if elapsed < TICK {
                        thread::sleep(TICK - elapsed);
                    }
                }
            });

        Self {
            tx,
            shared,
            running,
        }
    }

    pub fn send(&self, cmd: Command) {
        // Отправка в отключённый канал игнорируется: приложение закрывается.
        let _ = self.tx.send(cmd);
    }

    /// Последний опубликованный снапшот (копируется только Arc — дёшево).
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.shared.snapshot.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Relaxed);
        self.send(Command::SetRunning(running));
    }
}

fn apply_command(sim: &mut Simulation, running: &AtomicBool, cmd: Command, log: &mut LogState) {
    match cmd {
        Command::SetParams(p) => sim.set_params(p),
        Command::SetRunning(r) => running.store(r, Ordering::Relaxed),
        Command::StepOnce => sim.step(),
        Command::Reset(scenario) => sim.reset(scenario),
        Command::AddBody {
            mass,
            radius,
            color,
            pos,
        } => sim.add_body_orbiting(mass, radius, color, pos),
        Command::RemoveBody(id) => sim.remove_body(id),
        Command::SetBodyMass(id, mass) => sim.set_body_mass(id, mass),
        Command::Clear => sim.clear(),
        Command::ReverseVelocities => sim.reverse_velocities(),
        Command::ReplaceState(bodies) => sim.replace_bodies(bodies),
        Command::SetLogging { mode, every, sender } => {
            log.mode = mode;
            log.every = every.max(1);
            log.tx = sender;
            // Форсируем ключевой кадр при (пере)старте логирования.
            log.last = None;
            log.since_keyframe = KEYFRAME_EVERY;
        }
    }
}

/// Состояние логирования на стороне потока физики.
struct LogState {
    mode: LogMode,
    every: u64,
    tx: Option<std::sync::mpsc::Sender<LogRecord>>,
    /// Позиции/скорости на момент последней записи — база для дельт.
    last: Option<HashMap<u64, (Vec3, Vec3)>>,
    since_keyframe: u32,
    /// Тик последней записи — не пишем один тик дважды.
    last_logged_tick: u64,
}

impl Default for LogState {
    fn default() -> Self {
        Self {
            mode: LogMode::Off,
            every: 10,
            tx: None,
            last: None,
            since_keyframe: KEYFRAME_EVERY,
            last_logged_tick: u64::MAX,
        }
    }
}

/// Периодичность ключевых кадров внутри дельта-лога.
const KEYFRAME_EVERY: u32 = 600;

/// Собрать запись лога: ключевой кадр или дельта.
fn build_log_record(log: &mut LogState, sim: &Simulation) -> LogRecord {
    let bodies = sim.bodies();
    let need_keyframe = log.mode == LogMode::Full
        || log.last.is_none()
        || log.since_keyframe >= KEYFRAME_EVERY;

    let snapshot_last: HashMap<u64, (Vec3, Vec3)> =
        bodies.iter().map(|b| (b.id, (b.pos, b.vel))).collect();

    let record = if need_keyframe {
        LogRecord::keyframe(sim.tick(), sim.sim_time(), bodies)
    } else {
        let last = log.last.as_ref().unwrap();
        let mut deltas = Vec::new();
        let mut added = Vec::new();
        let mut present = std::collections::HashSet::new();
        for b in bodies {
            present.insert(b.id);
            match last.get(&b.id) {
                Some((lp, lv)) => {
                    if b.pos != *lp || b.vel != *lv {
                        deltas.push(BodyDelta {
                            id: b.id,
                            dx: b.pos.x - lp.x,
                            dy: b.pos.y - lp.y,
                            dz: b.pos.z - lp.z,
                            dvx: b.vel.x - lv.x,
                            dvy: b.vel.y - lv.y,
                            dvz: b.vel.z - lv.z,
                        });
                    }
                }
                None => added.push(BodyLog::from(b)),
            }
        }
        let removed: Vec<u64> = last
            .keys()
            .filter(|id| !present.contains(*id))
            .copied()
            .collect();

        if added.len() * 2 > bodies.len() {
            // Слишком много новых тел — ключевой кадр дешевле россыпи добавлений.
            LogRecord::keyframe(sim.tick(), sim.sim_time(), bodies)
        } else {
            LogRecord {
                tick: sim.tick(),
                sim_time: sim.sim_time(),
                payload: RecordPayload::Delta {
                    deltas,
                    added,
                    removed,
                },
            }
        }
    };

    log.last = Some(snapshot_last);
    log.since_keyframe = if need_keyframe { 0 } else { log.since_keyframe + 1 };
    record
}
