//! HIL harness for FE-C over BLE on a real Tacx NEO (#44, ADR-0002 §9).
//!
//! Run from an Alacritty window (flock panes are denied Bluetooth by TCC; AGENTS.md):
//! `cargo run -p crank-ble --features hil --example hil_fec -- check1 --out DIR`.
//! The Part B control checks (`check5`, `check6`, `check7`, `check8`, `check9`, `check12`) drive the
//! trainer under a rider and always release it (`rider.rs`); `check10` holds the link while the
//! operator tries a second central (`check10.rs`); `check11b` drops and reconnects the link
//! (`check11.rs`); `docs/hil/part-b.md` is the runbook.
//! Each subcommand writes `<check>.fecap` (raw frames, both directions, the
//! `crank-ble` capture format) and `<check>.json` into `--out`. Raw captures carry
//! the device serial; commit only the output of `redact`.
//!
//! Exit status: 0 done; 1 error; 2 bad arguments; 3 blocked (the owner must act); 4 a Part B
//! procedure stopped early (no result; the `STOPPED:` line says whether the release was
//! confirmed); 5 a Part B procedure completed but its release was NOT confirmed (the last line
//! says so, with what the rider does; expected after an 11a or 11b drop).
mod check10;
mod check11;
mod check11ac;
mod checks;
mod cli;
mod cps;
#[cfg(test)]
mod desktop_capture;
mod frames;
mod partb;
mod reuse;
mod rider;
mod session;
#[cfg(test)]
mod sim;
mod timeline;
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
        Ok(outcome) => {
            let (lines, status) = outcome.lines();
            for line in lines {
                println!("{line}");
            }
            ExitCode::from(status)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
