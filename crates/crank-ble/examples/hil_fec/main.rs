//! HIL harness for FE-C over BLE on a real Tacx NEO (#44, ADR-0002 §9).
//!
//! Run from an Alacritty window (flock panes are denied Bluetooth by TCC; AGENTS.md):
//! `cargo run -p crank-ble --features hil --example hil_fec -- check1 --out DIR`.
//! Each subcommand writes `<check>.fecap` (raw frames, both directions, the
//! `crank-ble` capture format) and `<check>.json` into `--out`. Raw captures carry
//! the device serial; commit only the output of `redact`.
mod checks;
mod cli;
mod frames;
mod session;
mod trace;

use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let default_out = std::env::temp_dir().join("crank-hil");
    let args = match cli::parse(std::env::args().skip(1), &default_out) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    match checks::run(&args).await {
        Ok(checks::Outcome::Done(path)) => {
            println!("wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Ok(checks::Outcome::Blocked(why)) => {
            println!("BLOCKED: {why}");
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
