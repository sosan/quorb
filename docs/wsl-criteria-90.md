# Cierre de criterios de WSL — issue #90

Este documento contrasta los criterios de aceptación de #90 con el código integrado en `main` y la evidencia publicada en las PRs encadenadas #220, #239, #248 y #253. No reabre la implementación WSL: consolida la evidencia y deja explícitos los límites aceptados.

## Matriz de aceptación

| Criterio | Estado | Evidencia |
|---|---|---|
| Configuración ausente o antigua conserva el modo Windows y el resto de datos | Cumplido | `SessionEnvConfig` tiene `Host` como default y la deserialización tolera valores desconocidos como `host`. Test `session_env_defaults_to_host_in_legacy_config` en `src-tauri/src/config.rs`; PR #239. |
| Modo y distro persisten; la configuración inválida se interpreta de forma segura; una distro desaparecida se distingue | Cumplido con límite | `session_env`/`wsl_distro` forman parte de `AppConfig`; una distro vacía se interpreta como host (`session_env_maps_wsl_with_distro_and_guards_empty_distro`); una distro ausente devuelve `DistroNotFound` con detalle de las disponibles (`unavailable_distro_detail_lists_available_ones`). PRs #239 y #248. La configuración no se reescribe para borrar una distro eliminada: el saneado aprobado es de interpretación, no de mutación del JSON legado. |
| Distro, estado y home del usuario predeterminado se resuelven sin asumir nombres/rutas ni instalar herramientas | Cumplido | `list_distributions` no despierta la distro; `resolve_home` obtiene `whoami` y el campo home de `getent passwd`. Tests de nombres con espacios y resolución del home. PR #220; integración de sesiones en #239. |
| Diagnóstico distingue WSL ausente, cero distros, distro detenida/eliminada, salida inválida y timeout | Cumplido | Errores tipados `WslNotInstalled`, `NoDistributions`, `DistroNotFound`, `InvalidArgument`, `UnparsableOutput` y `Timeout`; tests de tokens de error, UTF-16 y transiciones. PR #220 y seguimiento #248/#253. |
| Procesos y filesystem tienen límites; no bloquean UI ni despiertan por polling | Cumplido | Cancelación acotada con pidfile y PID remoto; `--terminate` solo como recuperación explícita; lecturas async con `spawn_blocking`; sondeo con `list_distributions` antes de entrar en la distro. Tests `session_resolver_does_not_wake_a_stopped_distro`, `cli_available_does_not_wake_a_stopped_distro` y `cli_status_reports_stopped_without_waking_the_distro_or_probing`. PRs #220, #239, #248 y #253. |
| Configuración, datos e historial propios de Quorb conservan ubicación Windows | Cumplido | `app_config_dir()` y `app_data_dir()` siguen siendo la fuente de `config.json` e `history.json` en `lib.rs`; el adaptador WSL solo resuelve sesiones CLI dentro del guest y no mueve datos de Quorb. PRs #220/#239. |
| Tests deterministas cubren persistencia, legacy, errores, varias distros y nombres especiales | Cumplido | 86 tests en `src-tauri/src/wsl/`; `MockWslAdapter`/`FakeRunner`; tests de configuración legacy, tabla de varias distros, nombres con espacios, UTF-16/no-ASCII, errores y rutas POSIX. PRs #220/#239/#248/#253. |
| Evidencia Windows/WSL real de descubrimiento y recuperación, sin afirmar compatibilidad no comprobada | Cumplido con límites | W0 names: run `39124241492`; W3 sobre Windows Server 2025 + WSL 3.0.3 con Alpine y Ubuntu: run [`41050473110`](https://github.com/sosan/quorb/actions/runs/41050473110), GAP1/GAP1B/GAP2 24/24/GAP3 26/26; login real de cuenta adicional: runs [`43337164772`](https://github.com/sosan/quorb/actions/runs/43337164772) y [`43341859872`](https://github.com/sosan/quorb/actions/runs/43341859872). |

## Límites declarados

- La evidencia de login en WSL usa shims dentro de una distro aislada; no acredita instalaciones reales de cada CLI en una máquina de usuario.
- No hay un run que elimine o detenga una distro durante una sesión de la UI y compruebe la recuperación E2E del widget. Los errores y transiciones están cubiertos por mocks, tests del resolvedor y el diagnóstico de proveedores.
- La distro eliminada se distingue y produce un error accionable; no se borra automáticamente `wsl_distro` del JSON. Esto conserva la configuración elegida y evita inventar una distro o modificarla silenciosamente.
- El alcance de WSL cubre las CLIs y sesiones definidas por las PRs #239/#248/#253. Copilot, APIs y proveedores que permanecen en host no se presentan como ejecución dentro de WSL.

Con la matriz anterior, los criterios funcionales de #90 están cubiertos por código, tests y evidencia real. Los límites no bloqueantes quedan documentados para decidir si se cierra #90 o se crea una sub-issue opcional de recuperación E2E/CLIs reales.

Referencias: [#90](https://github.com/egdev6/quorb/issues/90), [#220](https://github.com/egdev6/quorb/pull/220), [#239](https://github.com/egdev6/quorb/pull/239), [#248](https://github.com/egdev6/quorb/pull/248), [#253](https://github.com/egdev6/quorb/pull/253).
