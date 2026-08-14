//! The end-to-end pipeline of "All But Regular: Revisiting the Starlink Constellation".
//!
//! Reads the public Starlink measurements shipped under `data/`, recovers the ideal constellation
//! behind them, classifies the irregularities of the real one, and builds the three hexGrid
//! topologies of every shell. It writes two kinds of output into `out/`:
//!
//!   * `out/irregularities.csv` — one row per shell, holding the recovered ideal structure and the
//!     counts of in-place, misaligned and missing orbits and satellites (Table 1 of the paper).
//!   * `out/networks/shell_<id>_<altitude>_<inclination>_<variant>.json` — the ISL topology of each
//!     shell in each of the three variants, in NetworkX's node-link JSON format.
//!
//! Run it with `cargo run --release`, optionally passing `--refresh` to pull fresh data from
//! CelesTrak and SatCat first, and `--out <dir>` to write somewhere other than `out/`.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

use itertools::Itertools;
use log::info;

use irregular_constellations::{
    constellation::{Constellation, ShellIrregularities},
    starlink::load_starlink_satellites,
    topology::{ShellId, Topology, Variant},
};

fn main() -> anyhow::Result<()> {
    pretty_env_logger::formatted_builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let (refresh, out_dir) = parse_arguments()?;
    let networks_dir = out_dir.join("networks");
    fs::create_dir_all(&networks_dir)?;

    // 1) Load the measurements, keep the operational satellites, and group them into shells and
    //    orbital planes (§2).
    let (satellites, epoch) = load_starlink_satellites(refresh);
    info!(
        "Loaded {} operational Starlink satellites, synchronized to {}",
        satellites.len(),
        epoch
    );
    let mut real: Constellation = (satellites, epoch).into();

    // 2) Recover the ideal structure of every shell and classify its irregularities (§3).
    let irregularities = real.regularize();

    // 3) Build the idealized constellation the real one approximates (§4).
    let mut ideal = real.clone();
    ideal.idealize();

    // 4) Build the three hexGrid topologies of every shell (§4).
    for (index, key) in real.shells.keys().sorted().enumerate() {
        let shell_id = ShellId {
            shell: index,
            altitude_km: key.0,
            inclination_deg: key.1,
        };
        for (constellation, variant) in [
            (&real, Variant::Naive),
            (&real, Variant::Patched),
            (&ideal, Variant::Ideal),
        ] {
            let topology = Topology::hex_grid(shell_id, &constellation.shells[key], epoch, variant);
            let path = networks_dir.join(format!(
                "shell_{}_{}_{}_{}.json",
                shell_id.shell,
                shell_id.altitude_km,
                shell_id.inclination_deg,
                variant.name()
            ));
            serde_json::to_writer(File::create(&path)?, &topology)?;
        }
    }
    info!("Wrote the shell topologies to {}", networks_dir.display());

    // 5) Report the irregularities of every shell.
    let irregularities_path = out_dir.join("irregularities.csv");
    write_irregularities(&irregularities_path, &irregularities)?;
    info!("Wrote the irregularities to {}", irregularities_path.display());

    Ok(())
}

/// Write the per-shell irregularity counts, i.e. Table 1 of the paper.
fn write_irregularities(path: &Path, irregularities: &[ShellIrregularities]) -> anyhow::Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record([
        "shell",
        "altitude_km",
        "inclination_deg",
        "orbit_spacing_deg",
        "orbits_in_place",
        "orbits_misaligned",
        "orbits_missing",
        "satellite_spacing_deg",
        "satellites_in_place",
        "satellites_misaligned",
        "satellites_missing",
    ])?;

    for shell in irregularities {
        writer.write_record([
            shell.shell.to_string(),
            shell.altitude_km.to_string(),
            shell.inclination_deg.to_string(),
            // A shell whose orbit spacing alternates keeps all of its values, longest first
            shell
                .orbit_spacing_deg
                .iter()
                .sorted_by(|a, b| b.total_cmp(a))
                .map(|spacing| format!("{:.2}", spacing))
                .join("/"),
            shell.orbits.in_place.to_string(),
            shell.orbits.misaligned.to_string(),
            shell.orbits.missing.to_string(),
            format!("{:.2}", shell.satellite_spacing_deg),
            shell.satellites.in_place.to_string(),
            shell.satellites.misaligned.to_string(),
            shell.satellites.missing.to_string(),
        ])?;
    }
    writer.flush()?;

    Ok(())
}

fn parse_arguments() -> anyhow::Result<(bool, PathBuf)> {
    let mut refresh = false;
    let mut out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out");

    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--refresh" => refresh = true,
            "--out" => {
                out_dir = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("--out expects a directory"))?
            }
            other => anyhow::bail!("Unexpected argument '{other}'. Usage: [--refresh] [--out DIR]"),
        }
    }

    Ok((refresh, out_dir))
}
