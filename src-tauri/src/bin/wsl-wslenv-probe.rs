//! Harness WSLENV para el lab W3 (issue #90, revisión del reviewer 2026-10-29).
//!
//! Invoca el CONSTRUCTOR REAL del adaptador (`build_wslenv_entry` de
//! `wsl::real`) en lugar de duplicar la política en PowerShell. El lab pwsh
//! antes fijaba `$env:WSLENV` a mano (política copiada); este binario
//! devuelve el WSLENV final que produce el código de producción, con la
//! allowlist, el flag por tipo de ruta y la preservación de entradas ya
//! presentes.
//!
//! Replica la fusión de `RealWslAdapter::env_with` (real.rs): el WSLENV del
//! proceso arranca en `current` y `build_wslenv_entry` decide si se añade o
//! reemplaza la entrada de esta variable. La POLÍTICA (allowlist, flag,
//! no-duplicar) la decide siempre el constructor real; aquí solo se orquesta
//! la base previa igual que `env_with`. Eso permite medir también los casos
//! de "ya presente": el constructor devuelve `None`, la entrada previa se
//! conserva tal cual (con su flag) y la variable sigue compartiéndose.
//!
//! Uso: `wsl-wslenv-probe.exe <key> <value> <current|->`
//!   - key: nombre de la variable (p. ej. `CLAUDE_CONFIG_DIR`)
//!   - value: valor de sesión (ruta Windows, ruta Linux, o no-ruta)
//!   - current: WSLENV ya presente (o `-` si vacío)
//!
//! Emite JSON plano: `{"wslenv":"<WSLENV final>"}` o `{"wslenv":null}`
//! cuando el WSLENV final queda vacío (nada que compartir: variable fuera
//! de allowlist, valor no-ruta, y sin entradas previas). Sin serde; el
//! resultado es una lista de nombres + flags, sin comillas, así que el
//! escape no aplica. Se compila con cargo (usa la lib real):
//!   cargo build -p quorb --bin wsl-wslenv-probe

use quorb_lib::wsl::build_wslenv_entry;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    // argv[0] = exe; argv[1] = key; argv[2] = value; argv[3] = current o "-"
    if argv.len() < 3 {
        eprintln!("uso: wsl-wslenv-probe <key> <value> [current|->]");
        return ExitCode::from(2);
    }
    let key = argv[1].as_str();
    let value = argv[2].as_str();
    let current = if argv.len() > 3 && argv[3] != "-" {
        argv[3].as_str()
    } else {
        ""
    };

    // `env_with` conserva el WSLENV existente cuando el constructor no
    // añade nada (por ejemplo, si la variable ya estaba presente). Solo
    // queda vacío cuando tampoco había entradas previas.
    match build_wslenv_entry(key, value, current) {
        Some(entry) => println!("{{\"wslenv\":\"{entry}\"}}"),
        None if current.is_empty() => println!("{{\"wslenv\":null}}"),
        None => println!("{{\"wslenv\":\"{current}\"}}"),
    }
    ExitCode::SUCCESS
}
