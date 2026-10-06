//! The `transpiler` CLI: reads a RISC-V ELF, transpiles it for the selected (or host)
//! architecture, and writes the native ELF.
use clap::Parser;
use log::debug;
use std::collections::HashSet;
use std::error::Error;
use std::fs::{File, read};
use std::io::Write;

use transpiler::{TranspilationMode, TranspilerForArchitecture, TranspilerInput};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Input RISC-V ELF file
    #[arg(long)]
    object: String,

    /// Comma-separated list of entry-point symbol names
    #[arg(long)]
    comma_separated_entry_points: String,

    /// Target architecture: arm64 (or aarch64) or x86_64. Defaults to the host's.
    #[arg(long)]
    platform: Option<TranspilerForArchitecture>,

    /// Output path for the native ELF file
    #[arg(long)]
    output_elf: String,

    /// Emit the bare-bones "trusted" variant (no gas metering, no bounds checks,
    /// 4-byte jump table) instead of the default gas-metered output.
    #[arg(long, default_value_t = false)]
    trusted: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    // A logger that is already installed is fine; keep the one that is.
    let _ = env_logger::builder().try_init();
    let args = Args::parse();
    let transpiler = match args.platform {
        Some(platform) => platform,
        None => TranspilerForArchitecture::get_native()
            .ok_or("no --platform given and the host architecture is not supported")?,
    };
    let entry_points: HashSet<String> = args
        .comma_separated_entry_points
        .split(',')
        .map(|s| s.to_string())
        .collect();
    #[expect(
        clippy::disallowed_methods,
        reason = "The CLI has no async runtime, so blocking file I/O is fine."
    )]
    let riscv_elf_bytes = read(&args.object)?;
    let mode = if args.trusted {
        TranspilationMode::Trusted
    } else {
        TranspilationMode::default()
    };
    let output = transpiler.transpile_to_native_elf(
        TranspilerInput {
            riscv_elf_bytes: &riscv_elf_bytes,
            entry_points,
        },
        &mode,
    )?;
    #[expect(
        clippy::disallowed_methods,
        reason = "The CLI has no async runtime, so blocking file I/O is fine."
    )]
    let mut file = File::create(&args.output_elf)?;
    file.write_all(&output.elf_bytes)?;
    debug!("Wrote transpiled ELF file to {}", args.output_elf);
    debug!("Entry point mapping: {:?}", output.entry_point_mapping);
    Ok(())
}
