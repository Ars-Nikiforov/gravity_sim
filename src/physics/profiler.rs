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

    /// Измерить время выполнения `f` под именем `name` и обновить статистику.
    pub fn measure<F, R>(&mut self, name: &'static str, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let start = Instant::now();
        let r = f();
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;

        if let Some(st) = self.stages.iter_mut().find(|s| s.name == name) {
            st.last_ms = elapsed;
            st.count += 1;
            st.avg_ms = st.avg_ms * 0.9 + elapsed * 0.1;
            st.max_ms = st.max_ms.max(elapsed);
        } else {
            self.stages.push(StageStat {
                name,
                last_ms: elapsed,
                avg_ms: elapsed,
                max_ms: elapsed,
                count: 1,
            });
        }
        r
    }

    pub fn stages(&self) -> &[StageStat] {
        &self.stages
    }

    pub fn clear(&mut self) {
        self.stages.clear();
    }

    /// Общая сумма средних по всем этапам, мс.
    pub fn total_avg_ms(&self) -> f64 {
        self.stages.iter().map(|s| s.avg_ms).sum()
    }
}
