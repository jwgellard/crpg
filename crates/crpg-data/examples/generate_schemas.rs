#![forbid(unsafe_code)]
//! Explicit schema-baseline authoring tool; ordinary builds never invoke it.

use std::{env, error::Error, fs, path::PathBuf, process::ExitCode};

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let directory = args
        .next()
        .map(PathBuf::from)
        .ok_or("usage: generate_schemas <existing-output-directory>")?;
    if args.next().is_some() || !directory.is_dir() {
        return Err("usage: generate_schemas <existing-output-directory>".into());
    }
    for (name, bytes) in crpg_data::generated_schemas()? {
        fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
