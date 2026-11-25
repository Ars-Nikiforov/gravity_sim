//! Измеритель времени этапов шага физики.
//!
//! Каждый этап замеряется отдельно: построение дерева, списки взаимодействий,
//! вычисление ускорений, интегрирование. Результаты сглаживаются EMA и
//! публикуются в [`Snapshot`] для отображения в интерфейсе.

use std::time::Instant;

/// Измерение одного этапа.
#[derive(Clone, Copy, Debug, Default)]
pub struct StageStat {
    pub name: &'static str,
    /// Последнее измерение, мс.
    pub last_ms: f64,
    /// Сглаженное среднее (EMA α=0.1), мс.
    pub avg_ms: f64,
    /// Максимум за сессию, мс.
    pub max_ms: f64,
    pub count: u64,
}

/// Измерить время выполнения `f`: свободная функция, не заимствует ни структуру,
/// ни профилировщик — замыкание может свободно владеть `&mut self`.
/// Возвращает результат и затраченное время в мс.
pub fn timed<F, R>(f: F) -> (R, f64)
where
    F: FnOnce() -> R,
{
    let start = Instant::now();
    let r = f();
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    (r, elapsed)
}

/// Профилировщик шага физики. Живёт в потоке физики; после каждого шага
/// его снимок копируется в Snapshot — UI лишь читает.
#[derive(Clone, Debug, Default)]
pub struct Profiler {
    stages: Vec<StageStat>,
}

impl Profiler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Записать ранее измеренное время этапа (в мс) в статистику.
    pub fn record(&mut self, name: &'static str, elapsed_ms: f64) {
        if let Some(st) = self.stages.iter_mut().find(|s| s.name == name) {
            st.last_ms = elapsed_ms;
            st.count += 1;
            st.avg_ms = st.avg_ms * 0.9 + elapsed_ms * 0.1;
            st.max_ms = st.max_ms.max(elapsed_ms);
        } else {
            self.stages.push(StageStat {
                name,
                last_ms: elapsed_ms,
                avg_ms: elapsed_ms,
                max_ms: elapsed_ms,
                count: 1,
            });
        }
    }

    pub fn stages(&self) -> &[StageStat] {
        &self.stages
    }
}
