//! Loading the public satellite measurements and grouping them into shells and orbital planes.
//!
//! Two sources are combined (§2 of the paper):
//!   * **CelesTrak** provides the orbital elements of every Starlink satellite, from which we derive
//!     altitude, inclination, RAAN (Ω) and argument of latitude (α).
//!   * **SatCat** (Jonathan's Space Report / GCAT) tells us which satellites are operational and
//!     which satellite bus they use, which in turn determines how many laser terminals they carry.
//!
//! Both are shipped with this artifact under `data/`, so the pipeline is reproducible offline. The
//! same pipeline can be re-run against freshly downloaded data by passing `refresh = true`, which
//! re-fetches both sources and overwrites the local copies.

use anyhow::Result;
use chrono::NaiveDateTime;
use itertools::Itertools;
use log::{debug, info};
use sgp4::Elements;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use crate::satcat::{parse_satcat_reader, FailureFlag, SatcatEntry};
use crate::satellite::{get_orbit_altitude, Satellite};

/// Snapshot of <https://celestrak.org/NORAD/elements/gp.php?GROUP=starlink&FORMAT=json>
pub const CELESTRAK_CACHE_PATH: &str = "data/CelesTrak/starlink.json";
/// Snapshot of <https://planet4589.org/space/gcat/tsv/cat/satcat.tsv>
pub const SATCAT_CACHE_PATH: &str = "data/SatCat/satcat.tsv";

/// Resolve a data path relative to the crate root, so the pipeline can be run from anywhere.
fn data_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Get the starlink satellite orbital elements from the CelesTrak service or from the local copy.
pub fn fetch_celestrak_data(refresh: bool) -> Result<Vec<Elements>> {
    let cache_path = data_path(CELESTRAK_CACHE_PATH);
    let cache_path = Path::new(&cache_path);

    if !refresh && cache_path.exists() {
        info!("Loading CelesTrak data from {:?}", cache_path);
        let cached_data = fs::read_to_string(cache_path)?;
        let elements_vec: Vec<Elements> = serde_json::from_str(&cached_data)?;
        info!("Loaded {} starlink entries", elements_vec.len());
        return Ok(elements_vec);
    }

    info!("Fetching CelesTrak data from API");
    let mut response = ureq::get("https://celestrak.org/NORAD/elements/gp.php")
        .query("GROUP", "starlink")
        .query("FORMAT", "json")
        .call()?;

    let raw_json = response.body_mut().read_to_string()?;
    let elements_vec: Vec<Elements> = serde_json::from_str(&raw_json)?;

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(cache_path, &raw_json)?;
    info!("Stored CelesTrak data in {:?}", cache_path);

    info!(
        "Parsed {} starlink entries from CelesTrak",
        elements_vec.len()
    );
    Ok(elements_vec)
}

/// Fetch and parse the full GCAT satcat catalog from planet4589.org or from the local copy.
pub fn fetch_satcat_data(refresh: bool) -> Result<Vec<SatcatEntry>> {
    let cache_path = data_path(SATCAT_CACHE_PATH);
    let cache_path = Path::new(&cache_path);

    if !refresh && cache_path.exists() {
        info!("Loading SatCat data from {:?}", cache_path);
        let file = fs::File::open(cache_path)?;
        let entries = parse_satcat_reader(file)
            .map_err(|e| anyhow::anyhow!("Failed to parse satcat: {}", e))?;
        info!("Loaded {} satcat entries", entries.len());
        return Ok(entries);
    }

    info!("Fetching satcat data from planet4589.org");
    let response = ureq::get("https://planet4589.org/space/gcat/tsv/cat/satcat.tsv").call()?;

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = BufWriter::new(File::create(cache_path)?);
    io::copy(&mut response.into_body().as_reader(), &mut file)?;
    drop(file);
    info!("Stored SatCat data in {:?}", cache_path);

    let file = fs::File::open(cache_path)?;
    let entries =
        parse_satcat_reader(file).map_err(|e| anyhow::anyhow!("Failed to parse satcat: {}", e))?;

    info!("Parsed {} satcat entries", entries.len());
    Ok(entries)
}

/// Load the operational starlink satellites from real world data and synchronize them to a
/// common epoch.
pub fn load_starlink_satellites(refresh: bool) -> (Vec<Satellite>, NaiveDateTime) {
    let celestrak_data = fetch_celestrak_data(refresh).unwrap();
    let satcat_data: HashMap<String, SatcatEntry> = fetch_satcat_data(refresh)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.piece.clone(), entry))
        .collect();

    build_starlink_satellites(celestrak_data, &satcat_data)
}

/// Build a list of operational starlink satellites from a set of orbital elements and a satcat
/// lookup, synchronizing them to a common epoch.
///
/// Only satellites that are present in the satcat and flagged as operational are kept. Each one
/// carries the number of laser terminals implied by its satellite bus, which is what limits its
/// ISL degree later on.
pub fn build_starlink_satellites(
    elements_data: Vec<Elements>,
    satcat_data: &HashMap<String, SatcatEntry>,
) -> (Vec<Satellite>, NaiveDateTime) {
    // Only collect operational satellites
    let operational: Vec<(Elements, u32)> = elements_data
        .into_iter()
        .filter_map(|element| {
            let sat_data = satcat_data.get(element.international_designator.as_ref()?)?;
            (sat_data.sat_type.failure_flag == FailureFlag::Operational)
                .then(|| (element, sat_data.get_isl_count()))
        })
        .collect();

    // Because the tracking data of different satellites was taken at different times, we need to
    // synchronize the satellites to a common epoch
    let common_time = operational.iter().map(|(e, _)| e.datetime).max().unwrap();
    debug!("Synchronizing all satellites to {}", common_time);
    let satellites: Vec<Satellite> = operational
        .into_iter()
        .map(|(elements, isl_number)| {
            Satellite::new(
                elements
                    .international_designator
                    .as_ref()
                    .unwrap()
                    .to_string(),
                elements,
                common_time,
                isl_number,
            )
        })
        .collect();
    (satellites, common_time)
}

/// Sort a list of starlink satellites into orbital shells.
/// A shell is defined by an (altitude [km], inclination [deg]) tuple: satellites are in the same
/// shell if they share the same altitude and inclination (§2).
pub fn sort_into_shells(satellites: Vec<Satellite>) -> HashMap<(u32, u32), Vec<Satellite>> {
    let mut shells = HashMap::new();
    let by_inclination = satellites
        .into_iter()
        .into_group_map_by(|s| s.elements.inclination.round() as u32);

    for (inclination, satellites_by_inclination) in by_inclination {
        let clusters = form_integer_clusters_by(
            satellites_by_inclination,
            |satellite| get_orbit_altitude(&satellite.elements) / 1000.0,
            1.0,
            50,
        );

        for (representative_altitude, cluster) in clusters {
            shells.insert((representative_altitude, inclination), cluster);
        }
    }

    debug!("Found {} shells", shells.len());
    shells
}

/// Find and assign satellites to orbital planes within a shell. Satellites sharing a RAAN (Ω)
/// share an orbital plane.
pub fn sort_into_orbits(satellites: Vec<Satellite>) -> HashMap<u32, Vec<Satellite>> {
    let orbits = form_integer_clusters_by(satellites, |satellite| satellite.get_raan(), 0.75, 1);
    debug!("Found {} orbits", orbits.len());
    orbits
}

/// Clusters elements based on a key function and groups them by the rounded
/// mean of the key within each cluster.
///
/// Internally calls `form_clusters_by`, then assigns each cluster a `u32`
/// representative equal to the rounded mean of `f(item)` over the cluster.
fn form_integer_clusters_by<I, F>(
    data: I,
    f: F,
    gap: f64,
    min_size: usize,
) -> HashMap<u32, Vec<I::Item>>
where
    I: IntoIterator,
    F: Fn(&I::Item) -> f64,
{
    // First find and cluster data
    let clusters = form_clusters_by(data, |x| f(x), gap, min_size);

    // Then compute the integer representatives
    clusters
        .into_iter()
        .map(|cluster| {
            let mean: f64 = cluster.iter().map(|x| f(x)).sum::<f64>() / cluster.len() as f64;
            let center = mean.round() as u32;
            (center, cluster)
        })
        .collect()
}

/// Groups elements into clusters based on sorted key distance.
///
/// Elements are sorted by `f`, then consecutive items are grouped into clusters
/// whenever their key difference is at most `gap`. Clusters smaller than
/// `min_size` are discarded as outliers.
fn form_clusters_by<I, F>(data: I, mut f: F, gap: f64, min_size: usize) -> Vec<Vec<I::Item>>
where
    I: IntoIterator,
    F: FnMut(&I::Item) -> f64,
{
    // Sort data according to the provided 'by' function
    let mut sorted_data: Vec<(f64, I::Item)> = data
        .into_iter()
        .map(|item| {
            let value = f(&item);
            (value, item)
        })
        .collect();

    sorted_data.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut clusters: Vec<Vec<I::Item>> = Vec::new();
    let mut outliers: usize = 0;
    let mut current_cluster: Vec<I::Item> = Vec::new();
    let mut previous_value: Option<f64> = None;

    for (value, item) in sorted_data {
        if let Some(prev) = previous_value {
            if value - prev > gap {
                // We should be considering a new cluster
                if current_cluster.len() >= min_size {
                    // Pinch off a new cluster
                    let cluster = std::mem::take(&mut current_cluster);
                    clusters.push(cluster);
                } else {
                    // This attempt at making a cluster contains less items than we expect
                    // so these are outliers
                    outliers += current_cluster.len();
                    current_cluster.clear();
                }
            }
        }

        current_cluster.push(item);
        previous_value = Some(value);
    }

    // Finish the last cluster
    if current_cluster.len() >= min_size {
        clusters.push(current_cluster);
    } else {
        outliers += current_cluster.len();
    }

    debug!("Found {} outliers and {} clusters", outliers, clusters.len());

    clusters
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::satcat::CoarseType;
    use test_log::test;

    #[test]
    fn test_load_satcat() {
        let entries = fetch_satcat_data(false).expect("satcat data should be readable");
        assert!(
            entries.len() > 1000,
            "Expected many entries, got {}",
            entries.len()
        );

        let payloads = entries
            .iter()
            .filter(|e| e.sat_type.coarse_type == CoarseType::Payload)
            .count();
        info!("Parsed {} satcat entries, {} of them payloads", entries.len(), payloads);
    }

    #[test]
    fn test_load_celestrak() {
        let entries = fetch_celestrak_data(false).expect("celestrak data should be readable");
        assert!(
            entries.len() > 1000,
            "Expected many entries, got {}",
            entries.len()
        );
    }

    /// Every operational satellite sits within a few km of the nominal altitude of its shell, and
    /// satellites at the same altitude share an inclination — the observation that lets us treat
    /// (altitude, inclination) as a shell identifier (§2).
    #[test]
    fn test_shells_are_well_separated() {
        let (satellites, _) = load_starlink_satellites(false);
        let shells = sort_into_shells(satellites);

        for (shell_key, sats) in &shells {
            let altitudes_km: Vec<f64> = sats
                .iter()
                .map(|sat| get_orbit_altitude(&sat.elements) / 1000.0)
                .collect();
            let deviation = altitudes_km
                .iter()
                .map(|altitude| (altitude - shell_key.0 as f64).abs())
                .fold(0.0_f64, f64::max);

            info!(
                "Shell {:?} holds {} satellites, all within {:.2} km of the nominal altitude",
                shell_key,
                sats.len(),
                deviation
            );
            assert!(
                deviation < 4.0,
                "Shell {:?} spreads {:.2} km around its nominal altitude",
                shell_key,
                deviation
            );
        }
    }
}
