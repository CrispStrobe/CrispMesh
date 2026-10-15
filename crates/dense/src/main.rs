//! Command line of the native dense pipeline. Subcommands mirror the Python modules.

use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        Some("defaults") => {
            println!("{}", serde_json::to_string_pretty(&crispmesh_dense::config::DenseConfig::default()).unwrap());
            ExitCode::SUCCESS
        }
        Some("settings") => {
            // Name, group, meaning, kind and default of every setting, or (without --schema) the defaults by name.
            if arguments.iter().any(|a| a == "--schema") {
                println!("{}", serde_json::to_string_pretty(&crispmesh_dense::config::settings_schema()).unwrap());
            } else {
                println!("{}", serde_json::to_string_pretty(&crispmesh_dense::config::DenseConfig::default()).unwrap());
            }
            ExitCode::SUCCESS
        }
        Some("mesh") => crispmesh_dense::mesh::command(&arguments[1..]),
        #[cfg(not(target_arch = "wasm32"))]
        Some("photos") => crispmesh_dense::photos::command(&arguments[1..]),
        Some("mat") => match crispmesh_dense::photos::markers::command(&arguments[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("crispmesh-dense mat: {error:#}");
                ExitCode::from(1)
            }
        },
        #[cfg(not(target_arch = "wasm32"))]
        Some("render") => match crispmesh_dense::photos::rendered::main(&arguments[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("crispmesh-dense render: {error:#}");
                ExitCode::from(1)
            }
        },
        Some("--version") => {
            println!("crispmesh-dense {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(name @ ("stereo" | "inputs" | "check" | "run" | "synthetic" | "inspect" | "texture" | "refine" | "surface" | "splats")) => {
            let result = match name {
                "splats" => crispmesh_dense::splats::train::main(&arguments[1..]),
                "surface" => crispmesh_dense::surface::train::main(&arguments[1..]),
                "texture" => crispmesh_dense::texture::main(&arguments[1..]),
                "refine" => crispmesh_dense::refine::main(&arguments[1..]),
                "stereo" => crispmesh_dense::stereo::main(&arguments[1..]),
                "check" => crispmesh_dense::check::main(&arguments[1..]),
                "run" => crispmesh_dense::run::main(&arguments[1..]),
                "synthetic" => crispmesh_dense::stereo::synthetic::main(&arguments[1..]),
                "inspect" => crispmesh_dense::inspect::main(&arguments[1..]),
                _ => crispmesh_dense::scene::main(&arguments[1..]),
            };
            match result {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("crispmesh-dense {name}: {error:#}");
                    ExitCode::from(1)
                }
            }
        }
        _ => {
            eprintln!("usage: crispmesh-dense <run|photos|mat|render|inputs|stereo|mesh|texture|refine|surface|splats|check|inspect|synthetic|settings|defaults|--version> ...   (see crates/dense/README.md)");
            ExitCode::from(2)
        }
    }
}
