# gravity_sim
мой 1 проект

## Запуск

```bash
cargo run --release
```

## Если не запускается окно

Движок рендеринга по умолчанию — wgpu. На машинах с двумя видеокартами
(или при работе через удалённый рабочий стол) wgpu иногда выбирает адаптер,
который не может выводить изображение в окно, — приложение само переключится
на OpenGL (glow).

Выбрать рендерер вручную:

```bash
GRAV_SIM_RENDERER=glow cargo run --release     # OpenGL
GRAV_SIM_RENDERER=wgpu cargo run --release     # wgpu (по умолчанию)
WGPU_BACKEND=dx12 cargo run --release          # конкретный бэкенд wgpu: dx12, vulkan, gl
```

В консоль при старте печатается выбранный GPU (`GPU: <имя> (<тип>, <бэкенд>)`).
