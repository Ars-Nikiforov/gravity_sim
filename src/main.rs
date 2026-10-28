//! Гравитационный симулятор — заготовка интерфейса.
//!
//! Архитектура:
//! * `physics` — движок и отдельный поток расчёта (не знает про egui);
//! * `logging` — асинхронная запись симуляции в JSONL-лог;
//! * `replay` — повтор симуляции по логу;
//! * `ui` — меню, панели и дисплей;
//! * `app` — состояние и сборка интерфейса.
//!
//! UI и физика асинхронны: интерфейс всегда рисует последний готовый
//! снапшот и просыпается по `request_repaint` от потока физики.

mod app;
mod logging;
mod physics;
mod replay;
mod ui;

use eframe::egui;
use std::panic::AssertUnwindSafe;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_gpu_panic_filter();

    let renderer = requested_renderer();
    match run(renderer) {
        Ok(()) => Ok(()),
        Err(err) => {
            // eframe 0.37 по умолчанию рисует через wgpu. На машинах с двумя
            // GPU (или с монитором, подключённым не к тому адаптеру) wgpu
            // может выбрать адаптер, который не умеет выводить в наше окно.
            // Тогда пробуем OpenGL — он почти всегда доступен.
            if renderer == eframe::Renderer::Wgpu {
                eprintln!("wgpu не подошёл: {err}\nПереключаюсь на glow (OpenGL).");
                run(eframe::Renderer::Glow)
            } else {
                Err(err)
            }
        }
    }
}

/// Рендерер по умолчанию — wgpu; `GRAV_SIM_RENDERER=glow` включает OpenGL.
fn requested_renderer() -> eframe::Renderer {
    match std::env::var("GRAV_SIM_RENDERER").as_deref() {
        Ok("glow") | Ok("opengl") => eframe::Renderer::Glow,
        _ => eframe::Renderer::Wgpu,
    }
}

fn run(renderer: eframe::Renderer) -> Result<(), Box<dyn std::error::Error>> {
    let options = native_options(renderer);

    // Инициализация wgpu паникует, если выбранный адаптер не поддерживает
    // поверхность окна. Ловим только эту панику и превращаем её в ошибку,
    // чтобы main мог перейти на glow; остальные паники пробрасываем.
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        eframe::run_native(
            "grav_sim",
            options,
            Box::new(|cc| Ok(Box::new(app::GravityApp::new(cc)))),
        )
    }));

    match result {
        Ok(res) => res.map_err(Into::into),
        Err(payload) => {
            let message = panic_message(&payload);
            if message.contains("surface isn't supported") {
                Err(message.into())
            } else {
                std::panic::resume_unwind(payload);
            }
        }
    }
}

fn native_options(renderer: eframe::Renderer) -> eframe::NativeOptions {
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([960.0, 600.0])
            .with_title("Гравитационный симулятор"),
        renderer,
        ..Default::default()
    };

    // Выбираем адаптер с учётом поверхности окна. Иначе при двух GPU wgpu
    // может взять адаптер, который не подключён к монитору окна, и упасть
    // с «The surface isn't supported by this adapter».
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.native_adapter_selector = Some(std::sync::Arc::new(pick_adapter));
    }

    options
}

/// Адаптер, который умеет выводить в это окно; предпочитаем дискретный GPU.
fn pick_adapter(
    adapters: &[eframe::wgpu::Adapter],
    surface: Option<&eframe::wgpu::Surface<'_>>,
) -> Result<eframe::wgpu::Adapter, String> {
    let mut best: Option<(u8, eframe::wgpu::Adapter)> = None;

    for adapter in adapters {
        if let Some(surface) = surface {
            // Пустой список форматов — этим адаптером окно нарисовать нельзя.
            if surface.get_capabilities(adapter).formats.is_empty() {
                continue;
            }
        }
        let rank = match adapter.get_info().device_type {
            eframe::wgpu::DeviceType::DiscreteGpu => 3,
            eframe::wgpu::DeviceType::IntegratedGpu => 2,
            eframe::wgpu::DeviceType::VirtualGpu => 1,
            _ => 0, // Other, Cpu: программный рендер — в последнюю очередь.
        };
        if best.as_ref().is_none_or(|(best_rank, _)| rank > *best_rank) {
            best = Some((rank, adapter.clone()));
        }
    }

    let (_, adapter) = best.ok_or_else(|| {
        format!(
            "ни один из {} графических адаптеров не поддерживает окно",
            adapters.len()
        )
    })?;

    let info = adapter.get_info();
    eprintln!(
        "GPU: {} ({:?}, {:?})",
        info.name, info.device_type, info.backend
    );
    Ok(adapter)
}

/// Паника «поверхность не поддерживается» — ожидаемая ситуация, о которой
/// сообщаем сами; остальные паники печатаются как обычно.
fn install_gpu_panic_filter() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.to_string().contains("surface isn't supported") {
            return;
        }
        default_hook(info);
    }));
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "неизвестная ошибка инициализации GPU".to_owned()
    }
}
