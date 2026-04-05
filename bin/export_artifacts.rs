use std::{env, path::PathBuf};

fn main() -> anyhow::Result<()> {
    let output_dir = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("artifacts"));

    let paths = sonar_prover::export_registered_artifacts(&output_dir)?;
    for path in paths {
        println!("{}", path.display());
    }

    Ok(())
}
