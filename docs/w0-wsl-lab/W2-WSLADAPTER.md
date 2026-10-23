# W2 — Esbozo del trait `WslAdapter` (Fase A, issue #90, quorb-wsl-v1)

> Esbozo de diseño de la interfaz que W2 implementará. Condicionado al
> checklist §1–§7 de `W1-CONTRACT.md` (aceptado) y a las decisiones del paso 0
> de `W2-PLAN.md`. No es código mergeable — es el contrato de la Fase A.
> Fecha: 2026-10-21.

---

## 1. Propósito

Aislar toda la interacción con `wsl.exe` detrás de un trait inyectable, para
que:
- la app dependa de `dyn WslAdapter` (inyectado en `AppState`), no de procesos;
- los tests usen `MockWslAdapter` (determinista, sin WSL real);
- producción use `RealWslAdapter` (envuelve `wsl.exe` vía el `ProcessRunner` /
  `Containment` ya existentes en `cli_process.rs`);
- el futuro port macOS no se vea afectado (módulo `wsl/` aislado, como exige
  la issue).

## 2. Errores del adaptador

Reutiliza `RunError` de `cli_process.rs` cuando aplique (timeout, proceso no
lanzado) y define los errores de dominio WSL:

```rust
/// Error tipado de una operación WSL. Se asigna desde el parseo de tokens
/// del output (F2) o desde fallos del lanzador (F13/timeout).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WslError {
    /// WSL no está instalado / no en PATH (`wsl --version` falla) — F6.
    WslNotInstalled,
    /// Cero distros (`wsl --list` → "no installed distributions") — F6.
    NoDistributions,
    /// Distro elegida no existe (`Wsl/Service/WSL_E_DISTRO_NOT_FOUND`) — F2/F6.
    DistroNotFound(String),
    /// Argumento inválido del CLI ("Invalid command line argument") — F2.
    InvalidArgument(String),
    /// Salida inesperada que no se pudo parsear.
    UnparsableOutput(String),
    /// Timeout: se mató el launcher local y se mató el proceso remoto de la
    /// operación (kill acotado por pidfile, nivel 1 de W2-PLAN §4). La distro
    /// queda Running y no se toca nada ajeno. `--terminate` no entra aquí.
    Timeout { distro: String, pid: i64 },
    /// Error de lanzamiento de proceso (re-export de RunError bajo el mismo).
    Spawn(RunError),
}
```

## 3. El trait

```rust
use std::path::{Path, PathBuf};

/// Interfaz que W2 implementa (RealWslAdapter) y los tests simulan
/// (MockWslAdapter). Métodos sincronos que devuelven Result — el lanzador
/// con timeout vive dentro del impl, no en el llamador.
pub trait WslAdapter: Send + Sync {
    /// Enumerar distros instaladas (F4, `wsl --list --quiet`/`--verbose`).
    /// NO despierta ninguna distro.
    fn list_distributions(&self) -> Result<Vec<Distribution>, WslError>;

    /// Resolver el home del default user de la distro (F7, SÍ despierta;
    /// solo se llama en acción explícita del usuario — W1 §2).
    /// vía `wsl -d <distro> -- getent passwd <user>` SIN pipe (F14: el
    /// pipeline `| cut -d: -f6` exigiría interpolación de shell; el campo 6
    /// se parsea en Rust por `:`). El `<user>` se resuelve PRIMERO con
    /// `wsl -d X -- whoami` (F5), nunca se asume el user Windows (W1 §1).
    /// Ver nota de implementación.
    fn resolve_home(&self, distro: &str) -> Result<PathBuf, WslError>;

    /// Leer un archivo dentro de la distro (decisión paso 0: `wsl -d X --
    /// cat <path>`), con timeout aplicable vía `--terminate` (F13).
    /// Devuelve el contenido como texto UTF-8 (lossy) — ver nota de
    /// implementación: el runner reutilizable devuelve `String`, no bytes.
    fn read_file(&self, distro: &str, path: &Path) -> Result<String, WslError>;

    /// Detectar la estrategia de shell de la distro (W1 §3, "una vez"): corre
    /// `command -v bash` (F12) y devuelve `BashLogin` si existe, `ShPosix` si
    /// no. El llamador la cachea y la pasa a `run_command`.
    fn shell_strategy(&self, distro: &str) -> Result<ShellStrategy, WslError>;

    /// Ejecutar un comando en la distro con la estrategia de shell (F8/F12):
    /// `-e bash -lc` si bash existe, si no `-e sh -c`.
    /// F14 (doble frontera): el script pasado al shell es CONSTANTE del código
    /// (allowlist, p. ej. `exec "$1" "$@"`) y `args`/`env` entran como
    /// parámetros posicionales y variables del proceso, nunca interpolados en
    /// el texto del script. `env` transmite las variables de sesión
    /// (CLAUDE_CONFIG_DIR / CODEX_HOME / GEMINI_CLI_HOME) a la distro
    /// (W1 §5/§7, WSLENV/env del proceso).
    fn run_command(
        &self,
        distro: &str,
        shell: ShellStrategy,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<CommandOutput, WslError>;

    /// Diagnóstico de los 4 estados (F6) con sondas separadas.
    /// `target` es la distro elegida (si hay): permite producir
    /// `WslState::DistroUnavailable` (decisión paso 0 #3) y el detalle F2.
    /// Sin `target`, devuelve Available / WslNotInstalled / NoDistributions.
    fn diagnose(&self, target: Option<&str>) -> Result<WslDiagnostic, WslError>;

    /// Detener la distro completa. SOLO como recuperación explícita del
    /// usuario (nivel 2 de W2-PLAN §4): `--terminate` detiene todo lo que
    /// corra en la distro, incluidos procesos y sesiones ajenos a Quorb.
    /// Nunca como primitive automático de timeout (ese es el kill acotado por
    /// pidfile de `run_command`/`read_file`).
    fn terminate(&self, distro: &str) -> Result<(), WslError>;
}

/// Distro enumerada (`wsl --list --verbose` bajo WSL_UTF8, F1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Distribution {
    pub name: String,        // nombre de store o importado
    pub state: DistroState,  // Running / Stopped
    pub version: u8,         // 1 o 2
}

/// Estado de la columna STATE de `wsl --list --verbose`. Incluye los estados
/// transitorios que WSL reporta (Installing, Converting, Uninstalling) para
/// no degradar a `UnparsableOutput` una distro legítimamente en transición.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistroState {
    Running,
    Stopped,
    Installing,
    Converting,
    Uninstalling,
}

impl Distribution {
    /// ¿Es usable para sesiones ahora mismo? (Running, o Stopped pero no en
    /// transición). Una distro Installing/Converting/Uninstalling NO lo es.
    pub fn is_usable(&self) -> bool {
        matches!(self.state, DistroState::Running | DistroState::Stopped)
    }
}

/// Estrategia de shell (W1 §3): se detecta una vez y se reutiliza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellStrategy {
    BashLogin,   // -e bash -lc  (bash existe; PATH limpio del login)
    ShPosix,     // -e sh -c     (fallback; Alpine/busybox)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslDiagnostic {
    pub state: WslState,
    /// Detalle por estado; sin tokens en UI/logs (W1 §7).
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WslState {
    Available,        // WSL + ≥1 distro
    WslNotInstalled,  // F6
    NoDistributions,  // F6
    DistroUnavailable, // elegida pero no presente (decisión paso 0 #3)
}
```

## 4. Notas de implementación (Fase B)

- **`run_command` + F14 (doble frontera)**: el `RealWslAdapter` construye
  `wsl.exe -d <distro> -e <shell> <script> <args…>` donde `<script>` es una
  **constante del código** (allowlist: `exec "$1" "$@"`, o
  `cd "$1" && exec "$2" "$@"` para resolver el home) y los args viajan como
  **parámetros posicionales**, referenciados por el propio script entre
  comillas (`"$1"`, `"$@"`). Nada se interpola en el texto del script; la
  ruta del shell y cada arg son elementos separados del `Command`.
  Los tests de preservación literal cubren las DOS fronteras (argv de
  Windows → `sh -c` → `exec`): espacios, no-ASCII, comillas, `$`, backticks,
  globs, newline, string vacío y argumento que empieza por `-`.
- **`resolve_home`** despierta la distro (F7): solo se llama desde la acción
  explícita "Detectar home WSL" en Ajustes (W1 §2), nunca en el refresco de
  la lista (F4).
- **`read_file`** usa `wsl -d X -- cat <path>` (decisión paso 0 #1) para poder
  aplicar el timeout con `--terminate` (F13), no UNC.
- **`diagnose`** combina `wsl --version` (WSL ausente) + `wsl --list`
  (cero/no distros) + salida de distro objetivo (F2/F6).
- **`resolve_home` — default user (F5)**: el `<user>` del `getent passwd` se
  obtiene con `wsl -d X -- whoami` (default user real de la distro), NUNCA
  asumiendo el user Windows (W1 §1). En una distro recién importada sin
  `/etc/wsl.conf`, el default es `root` (F5) — el impl debe devolver
  `/root` en ese caso, no fallar por asumir `quorbuser`.
- **`resolve_home` — el pipe `| cut -d: -f6`**: el pipeline del comando
  canónico (W1 §1) exige interpretación de shell. Para cumplir F14 (nunca
  interpolando strings), el impl debe ejecutar `getent passwd <user>` SIN
  pipe y **parsear el campo 6 del output en Rust** (split por `:`), en lugar
  de pasar `sh -c '... | cut ...'`.
- **`read_file` — contrato de texto, no bytes**: el runner reutilizable
  (`ProcessRunner`, cli_process.rs) devuelve `CommandOutput { stdout: String,
  stderr: String }` con `String::from_utf8_lossy`. `read_file` devuelve
  `String` (UTF-8 lossy), NO `Vec<u8>`, porque los bytes crudos ya se
  pierden en el runner. Para credenciales JSON (UTF-8) es suficiente; si W2
  necesitara bytes exactos, habría que añadir un runner de bytes (fuera del
  alcance del adaptador actual).
- **Locale / idioma del output**: el mapeo de tokens F2 (`"no installed
  distributions"`, `Wsl/Service/...`) se observó en imagen en-US con
  `WSL_UTF8=1` (que fija codificación, NO locale). En un sistema con locale
  es-ES/de-DE, el mensaje de cero distros puede venir localizado y no
  matchear el token exacto → `NoDistributions` no se produciría. Fijar el
  locale del proceso hijo (p. ej. `LANG=C`/`LC_ALL=C` via env) o parsear por
  estructura (`--list --quiet` vacío ⇒ cero distros) en lugar de texto
  localizado. A validar en W3.
- **Sanitización de errores**: `UnparsableOutput(String)` e
  `InvalidArgument(String)` transportan texto crudo del output. Para cumplir
  W1 §7 ("sin tokens en UI/logs/errores") y la §5 del propio esbozo, el
  impl debe **sanitizar/etiquetar** el payload antes de exponerlo a la UI
  (nunca formatear el `String` crudo directamente); si el output incluye
  fragmentos sensibles, se descartan.
- **`MockWslAdapter`** (Fase A) programable por estados: como el trait usa
  `&self`, el mock requiere **mutabilidad interior** (`Mutex`/`RwLock` o
  `Cell`) para configurar estados distintos por caso (ausente/inválido/cero
  distros/timeout). El esbozo lo declara para que la Fase A no rediseñe el
  mock a mitad. Los tests de W1 §7 (ausente, inválido, espacios/no-ASCII,
  varias distros, cero distros, timeout) se cubren con él.
- **Detección de shell (W1 §3, "una vez")**: el trait expone
  `shell_strategy(distro) -> Result<ShellStrategy, WslError>` (o un método
  `detect_shell`) para que el llamador obtenga la estrategia y la pase a
  `run_command` — sin eso, el llamador adivinaría `ShPosix` perdiendo el
  PATH login con nvm/volta (F12). Se añade a la superficie del trait.

## 5. Lo que NO hace este esbozo

- No decide la ubicación final del archivo (p. ej. `src-tauri/src/wsl/mod.rs`
  con `trait` y `real.rs`) — lo fija la Fase A al implementar.
- No añade el plugin shell (decisión verificada: se reutiliza
  `ProcessRunner`/`Containment` — W2-PLAN §3).
- No toca Windows nativo ni macOS (aislamiento de la issue).
- No registra secretos ni tokens (W1 §7; los errores solo tienen el nombre
  de distro/estado, nunca contenido de credenciales).