# wb-r

Reescritura nativa en **Rust + Slint (Material Design 3)** de `wb-rew` (WeekBox).

## Stack Tecnológico

- **UI:** [Slint](https://slint.dev/) `1.19` + [Material Components](https://github.com/slint-ui/material-components) (`@material`)
- **Runtime Async:** `tokio`
- **Red / APIs:** `reqwest` (GameBanana, GitHub Releases, feeds de noticias)
- **Serialización y Config:** `serde`, `serde_json`, `directories`
- **Sistema y Procesos:** `sysinfo` (detección de engines en ejecución y playtime)
- **Logs / Errores:** `tracing`, `anyhow`, `thiserror`

## Estructura

- `ui/` — Vistas declarativas `.slint` con Material Design 3 (`Home`, `Library`, `Instances`, `News`) y `ui/vendor/material-components`.
- `src/app.rs` — Puente entre el estado en Rust/Tokio y la UI de Slint.
- `src/core/` — Configuración, registro de engines, gestión de mods, detección de procesos y utilidades de host.
- `src/services/` — Clientes HTTP para GameBanana, GitHub y noticias.

## Comandos

```bash
cargo check
cargo run
```
