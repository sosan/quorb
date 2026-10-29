//! Harness WSLENV para el lab W3 (issue #90, revisión del reviewer 2026-10-29).
//!
//! Invoca el CONSTRUCTOR REAL del adaptador (`build_wslenv_entry` de
//! `wsl::real`) en lugar de duplicar la política en PowerShell. El lab pwsh
//! antes fijaba `$env:WSLENV` a mano (política copiada); este binario
//! devuelve la entrada que produce el código de producción, con la allowlist
//! y la preservación de entradas ya presentes.
//!
//! Uso: `wsl-wslenv-probe.exe <key> <value> <current|->`
//!   - key: nombre de la variable (p. ej. `CLAUDE_CONFIG_DIR`)
//!   - value: valor de sesión (ruta Windows, ruta Linux, o no-ruta)
//!   - current: WSLENV ya presente (o `-` si vacío)
//!
//! Emite JSON plano: `{"wslenv":"<resultado>"}` o `{"wslenv":null}` cuando
//! la variable no entra en WSLENV (fuera de allowlist o no es ruta). Sin
//! serde; el resultado es un nombre de variable + flags, sin comillas, así
//! que el escape no aplica. Se compila con cargo (usa la lib real):
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

    match build_wslenv_entry(key, value, current) {
        Some(entry) => println!("{{\"wslenv\":\"{entry}\"}}"),
        None => println!("{{\"wslenv\":null}}"),
    }
    ExitCode::SUCCESS
}
