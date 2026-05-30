use std::sync::Arc;
use std::thread;

use rarog::engine::Engine;
use rarog::engine_command::{EngineCommandQueue, EngineControl};
use rarog::infra::capitalize_first_letter;
use rarog::uci_protocol::UciProtocol;

fn main() {
    if !pext_build_is_supported() {
        eprintln!(
            "{} PEXT build requires a CPU with BMI2/PEXT support. Use the AVX2 build on this machine.",
            capitalize_first_letter(env!("CARGO_PKG_NAME"))
        );
        std::process::exit(1);
    }

    println!(
        "{} {} by {}",
        capitalize_first_letter(env!("CARGO_PKG_NAME")),
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_AUTHORS").replace(':', ", ")
    );

    let commands = EngineCommandQueue::default();
    let control = Arc::new(EngineControl::default());
    let mut engine = Engine::new(commands.clone(), Arc::clone(&control));
    let engine_thread = thread::spawn(move || engine.start());

    UciProtocol::new(commands, control).uci_loop();
    engine_thread.join().expect("Engine thread failed.");
}

#[cfg(all(rarog_pext, target_arch = "x86_64"))]
fn pext_build_is_supported() -> bool {
    std::is_x86_feature_detected!("bmi2")
}

#[cfg(all(rarog_pext, target_arch = "x86"))]
fn pext_build_is_supported() -> bool {
    std::is_x86_feature_detected!("bmi2")
}

#[cfg(not(all(rarog_pext, any(target_arch = "x86", target_arch = "x86_64"))))]
fn pext_build_is_supported() -> bool {
    true
}
