//! `generate-headphone-meta`: headphone catalogue to `dist/json` metadata files.
//!
//! Rust replacement for Python `scripts/generate_headphone_meta.py`.
//! Reads the Python-literal catalogue in `datas/headphone_*.py` plus
//! the EQ/score files under `datas/headphone_eq`, and writes
//! `headphone_metadata.json` and `headphone_eqdata.json` with the
//! same bytes as the Python script (`json.dumps` defaults).
//! Python remains in place as reference/fallback until the pipeline
//! wiring step switches to this binary.

// Rust guideline compliant 2026-10-09

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use mimalloc::MiMalloc;
use spinorama_generate::headphone_meta::{
    add_headphone_eq, add_headphone_scores, dumps_python, load_catalogue, split_outputs,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

struct Args {
    datas_dir: PathBuf,
    output_dir: PathBuf,
    headphone: Option<String>,
    dry_run: bool,
    force: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        datas_dir: PathBuf::from("./datas"),
        output_dir: PathBuf::from("dist/json"),
        headphone: None,
        dry_run: false,
        force: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let value = |it: &mut dyn Iterator<Item = String>| -> Result<String> {
            it.next().with_context(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--datas-dir" => args.datas_dir = PathBuf::from(value(&mut it)?),
            "--output-dir" => args.output_dir = PathBuf::from(value(&mut it)?),
            "--headphone" => args.headphone = Some(value(&mut it)?),
            "--dry-run" => args.dry_run = true,
            "--force" => args.force = true,
            "--help" | "-h" => {
                println!(
                    "usage: generate-headphone-meta [--datas-dir DIR] [--output-dir DIR] \
                     [--headphone NAME] [--dry-run] [--force]"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other} (try --help)"),
        }
    }
    Ok(args)
}

fn write_if_different(path: &std::path::Path, content: &str, force: bool) -> Result<bool> {
    if !force
        && let Ok(existing) = std::fs::read_to_string(path)
        && existing == content
    {
        return Ok(false);
    }
    std::fs::write(path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

fn run(args: &Args) -> Result<()> {
    let mut catalogue = load_catalogue(&args.datas_dir)?;
    let total = catalogue.len();
    if let Some(want) = &args.headphone {
        catalogue.retain(|(name, _)| name == want);
        if catalogue.is_empty() {
            bail!("headphone not found: {want}");
        }
    }
    let eq_dir = args.datas_dir.join("headphone_eq");
    add_headphone_eq(&mut catalogue, &eq_dir);
    add_headphone_scores(&mut catalogue, &eq_dir);
    let (meta, eqdata) = split_outputs(&catalogue);

    if args.dry_run {
        println!(
            "generate-headphone-meta: {} headphones ({} in catalogue), dry-run: \
             headphone_metadata.json {} bytes, headphone_eqdata.json {} bytes",
            catalogue.len(),
            total,
            dumps_python(&meta).len(),
            dumps_python(&eqdata).len()
        );
        return Ok(());
    }

    std::fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("creating {}", args.output_dir.display()))?;
    for (file_name, data) in [
        ("headphone_metadata.json", &meta),
        ("headphone_eqdata.json", &eqdata),
    ] {
        let path = args.output_dir.join(file_name);
        let js = dumps_python(data);
        if write_if_different(&path, &js, args.force)? {
            println!(
                "generate-headphone-meta: generated {} ({} entries)",
                path.display(),
                catalogue.len()
            );
        }
    }
    println!(
        "generate-headphone-meta: wrote headphone metadata for {} headphones",
        catalogue.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    match parse_args().and_then(|args| run(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("generate-headphone-meta: {error:#}");
            ExitCode::FAILURE
        }
    }
}
