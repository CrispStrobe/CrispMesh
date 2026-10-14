//! Command line of the native dense pipeline. Subcommands mirror the Python modules.

use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("defaults") => {
            println!("{}", serde_json::to_string_pretty(&crisp3ds_dense::config::DenseConfig::default()).unwrap());
            ExitCode::SUCCESS
        }
        Some("settings") => {
            // Name, group, meaning, kind and default of every setting, or (without --schema) the defaults by name.
            if arguments.iter().any(|a| a == "--schema") {
                println!("{}", serde_json::to_string_pretty(&crisp3ds_dense::config::settings_schema()).unwrap());
            } else {
                println!("{}", serde_json::to_string_pretty(&crisp3ds_dense::config::DenseConfig::default()).unwrap());
            }
            ExitCode::SUCCESS
        }
        Some("mesh") => crisp3ds_dense::mesh::command(&arguments[1..]),
        #[cfg(not(target_arch = "wasm32"))]
        Some("photos") => crisp3ds_dense::photos::command(&arguments[1..]),
        Some("mat") => match crisp3ds_dense::photos::markers::command(&arguments[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("crisp3ds-dense mat: {error:#}");
                ExitCode::from(1)
            }
        },
        #[cfg(not(target_arch = "wasm32"))]
        Some("render") => match crisp3ds_dense::photos::rendered::main(&arguments[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("crisp3ds-dense render: {error:#}");
                ExitCode::from(1)
            }
        },
        Some("--version") => {
            println!("crisp3ds-dense {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(name @ ("stereo" | "inputs" | "check" | "run" | "synthetic" | "inspect" | "texture" | "refine" | "surface" | "splats")) => {
            let result = match name {
                "splats" => crisp3ds_dense::splats::train::main(&arguments[1..]),
                "surface" => crisp3ds_dense::surface::train::main(&arguments[1..]),
                "texture" => crisp3ds_dense::texture::main(&arguments[1..]),
                "refine" => crisp3ds_dense::refine::main(&arguments[1..]),
                "stereo" => crisp3ds_dense::stereo::main(&arguments[1..]),
                "check" => crisp3ds_dense::check::main(&arguments[1..]),
                "run" => crisp3ds_dense::run::main(&arguments[1..]),
                "synthetic" => crisp3ds_dense::stereo::synthetic::main(&arguments[1..]),
                "inspect" => crisp3ds_dense::inspect::main(&arguments[1..]),
                _ => crisp3ds_dense::scene::main(&arguments[1..]),
            };
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("crisp3ds-dense {name}: {error:#}");
                    ExitCode::from(1)
                }
            }
        }
        _ => {
            eprintln!("usage: crisp3ds-dense <run|photos|mat|render|inputs|stereo|mesh|texture|refine|surface|splats|check|inspect|synthetic|settings|defaults|--version> ...   (see crates/dense/README.md)");
            ExitCode::from(2)
        }
    }
}
