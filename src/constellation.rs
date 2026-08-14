//! Recovering the ideal constellation hiding behind the measurements, and classifying the
//! irregularities of the real one against it (§3 of the paper).
//!
//! A [`Constellation`] is a set of [`Shell`]s, each a set of [`Orbit`]s, each a ring of
//! [`OrbitalSlot`]s. Building one from measured satellites only groups them (see
//! [`crate::starlink`]); the interesting work happens in two further passes:
//!
//! * [`Constellation::regularize`] infers the ideal spacing between orbits and between satellites,
//!   finds the phase offsets, and then labels every orbit and satellite as *in-place* or
//!   *misaligned*, additionally inserting the *missing* ones as empty slots ("holes"). It reports
//!   the resulting counts as [`ShellIrregularities`].
//! * [`Constellation::idealize`] turns the regularized constellation into the perfectly regular one
//!   it approximates: misaligned elements are dropped and holes are filled with synthetic
//!   satellites.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use itertools::Itertools;
use log::{debug, info};
use serde::Serialize;

use crate::{
    elements::ElementsConstructor,
    satellite::{Satellite, Terminals},
    starlink::{sort_into_orbits, sort_into_shells},
};

/// How many laser terminals a satellite of an idealized constellation is assumed to carry.
const IDEAL_TERMINALS: u32 = 3;
/// Tolerance applied when matching a real element against its ideal counterpart, as a fraction of
/// the ideal spacing. It absorbs measurement inaccuracies (§3.2).
const MATCH_TOLERANCE_FRAC: f64 = 0.2;

#[serde_with::serde_as]
#[derive(Serialize, Clone)]
pub struct Constellation {
    /// All satellites in a shell share the same altitude [km] and inclination [deg]
    #[serde_as(as = "HashMap<serde_with::json::JsonString, _>")]
    pub shells: HashMap<(u32, u32), Shell>,
    /// The common epoch the satellites in the constellation are originally synchronized to
    pub epoch: NaiveDateTime,
}

impl Constellation {
    /// Regularize every shell in this constellation, returning the irregularities found in each of
    /// them. Shells are numbered by increasing altitude, matching the paper's shell IDs.
    pub fn regularize(&mut self) -> Vec<ShellIrregularities> {
        self.shells
            .iter_mut()
            .sorted_by_key(|(k, _)| *k)
            .enumerate()
            .map(|(index, (&(altitude, inclination), shell))| {
                info!("Regularizing shell {} ({}-{})", index, altitude, inclination);
                let (orbits, satellites, orbit_spacing, satellite_spacing) = shell.regularize();
                ShellIrregularities {
                    shell: index,
                    altitude_km: altitude,
                    inclination_deg: inclination,
                    orbit_spacing_deg: orbit_spacing,
                    orbits,
                    satellite_spacing_deg: satellite_spacing,
                    satellites,
                }
            })
            .collect()
    }

    /// A constellation is considered to be regular when each shell and orbit is consistent
    /// in the number of regularised orbital slots
    pub fn is_regular(&self) -> bool {
        self.shells.values().all(|shell| shell.is_regular())
    }

    /// Convert a constellation to the ideal version of itself. This does three things:
    /// 1. Removes all orbits and satellites that are out of place
    /// 2. Fills all orbital slots that are in position but have no satellite
    /// 3. Updates all satellites to have 3 ISLs
    pub fn idealize(&mut self) {
        assert!(
            self.is_regular(),
            "A constellation needs to be regularised before it is idealised"
        );
        let epoch = self.epoch;
        self.shells
            .iter_mut()
            .for_each(|(key, shell)| shell.idealize(*key, epoch));
    }
}

impl From<(Vec<Satellite>, NaiveDateTime)> for Constellation {
    /// Build a Constellation from a real list of satellites
    fn from((real_satellites, epoch): (Vec<Satellite>, NaiveDateTime)) -> Self {
        Constellation {
            shells: sort_into_shells(real_satellites)
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            epoch,
        }
    }
}

/// How many elements (orbits or satellites) of a shell fall into each of the classes of §3.2.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ElementCounts {
    /// Elements that match their ideal counterpart
    pub in_place: usize,
    /// Elements that exist in the real constellation, but not at an ideal position
    pub misaligned: usize,
    /// Elements that exist in the ideal constellation, but not in the real one
    pub missing: usize,
}

/// The irregularities of a single shell: the recovered ideal structure plus the classification of
/// the real elements against it. This is the per-shell row of Table 1 in the paper.
#[derive(Debug, Clone, Serialize)]
pub struct ShellIrregularities {
    /// The shell's index when shells are ordered by increasing altitude
    pub shell: usize,
    pub altitude_km: u32,
    pub inclination_deg: u32,
    /// The recovered ideal spacing ΔΩ* between orbits. More than one value means the spacing
    /// cycles through them (e.g. shell 2 alternates between a short and a long spacing).
    pub orbit_spacing_deg: Vec<f64>,
    pub orbits: ElementCounts,
    /// The recovered ideal intra-orbit spacing Δα* between satellites
    pub satellite_spacing_deg: f64,
    /// Satellites are only classified within the orbits that are themselves in place
    pub satellites: ElementCounts,
}

#[derive(Serialize, Clone)]
pub struct Shell {
    /// This shell's orbits
    pub orbits: Vec<Orbit>,
}

impl Shell {
    /// Compute the most likely spacing (in [deg]) between two neighboring satellites in the same orbit
    /// This seems to be regular within each shell
    pub fn get_satellite_spacing(&self) -> f64 {
        // We compute this based on the distance of each satellite (in [deg]) from the preceding satellite
        let gaps: Vec<f64> = self
            .orbits
            .iter()
            .flat_map(|orbit| {
                assert!(orbit
                    .satellites
                    .is_sorted_by_key(|s| s.argument_of_latitude));
                orbit
                    .satellites
                    .iter()
                    .circular_tuple_windows()
                    .map(|(prev, next)| {
                        (next.argument_of_latitude - prev.argument_of_latitude).rem_euclid(360.0)
                    })
            })
            .collect();

        // Extract the most obvious value from the histogram
        let peaks = find_histogram_modes(&gaps, 0.9, 2).unwrap();
        let coarse_mode = peaks.first().unwrap();

        let refined = refine_by_harmonic_median(&gaps, *coarse_mode);
        snap_spacing_to_integer_slots(refined)
    }

    /// Compute the most likely spacing between two neighboring orbits.
    /// This could very possibly be multimode, where the orbits are arranged along a large gap first,
    /// then close to the first orbits we placed.
    pub fn get_orbit_spacing(&self) -> Vec<f64> {
        // We compute this based on the RAAN difference of each orbit from the preceding one
        let raans: Vec<f64> = self.orbits.iter().map(|o| o.raan).collect();
        assert!(raans.is_sorted());
        let gaps: Vec<f64> = raans
            .iter()
            .circular_tuple_windows()
            .map(|(prev, next)| (next - prev).rem_euclid(360.0))
            .collect();

        // Extract the most obvious value(s) from the histogram.
        // For the common single-mode case, refine and snap to an integer number of slots over 360°.
        let peaks = find_histogram_modes(&gaps, 0.8, 2).unwrap();
        if peaks.len() == 1 {
            let refined = refine_by_harmonic_median(&gaps, peaks[0]);
            vec![snap_spacing_to_integer_slots(refined)]
        } else {
            peaks
        }
    }

    /// Regularize this shell. This means:
    ///     - finding the ideal spacing between the orbits
    ///     - marking the orbits that are misaligned, and remembering where orbits are missing
    ///     - finding the ideal satellite spacing in this shell
    ///     - regularizing the individual in-place orbits
    ///     - adding the missing orbits as placeholders full of holes
    ///
    /// Returns the orbit counts, the satellite counts, and the two recovered spacings.
    fn regularize(&mut self) -> (ElementCounts, ElementCounts, Vec<f64>, f64) {
        let orbit_spacing = self.get_orbit_spacing();
        info!("\tFound an orbit spacing of {:?} degrees", orbit_spacing);
        // Mask the orbits depending on their positions within the shell
        let (mask, missing, _) = classify_element_positions(
            &self.orbits.iter().map(|o| o.raan).collect(),
            orbit_spacing.clone(),
        );
        let orbits = ElementCounts {
            in_place: mask.iter().filter(|in_position| **in_position).count(),
            misaligned: mask.iter().filter(|in_position| !**in_position).count(),
            missing: missing.len(),
        };
        info!(
            "\tFound {} orbits ({} misaligned) and {} missing",
            mask.len(),
            orbits.misaligned,
            orbits.missing,
        );
        // Mark the out of position orbits
        assert_eq!(self.orbits.len(), mask.len());
        self.orbits
            .iter_mut()
            .zip(mask)
            .for_each(|(orbit, in_position)| orbit.in_position = in_position);

        // Find satellite spacing
        let satellite_spacing = self.get_satellite_spacing();
        info!(
            "\tFound a satellite spacing of {:?} degrees",
            satellite_spacing
        );
        // Regularize each orbit that is in place. A satellite is misaligned if it sits in an
        // in-place orbit but not on one of its ideal slots — or if its orbit is itself misaligned,
        // in which case it has no ideal slot to be compared against to begin with.
        let mut satellites = ElementCounts {
            misaligned: self
                .orbits
                .iter()
                .filter(|orbit| !orbit.in_position)
                .map(|orbit| orbit.satellites.len())
                .sum(),
            ..ElementCounts::default()
        };
        for orbit in self.orbits.iter_mut().filter(|orbit| orbit.in_position) {
            let counts = orbit.regularize(satellite_spacing);
            satellites.in_place += counts.in_place;
            satellites.misaligned += counts.misaligned;
            satellites.missing += counts.missing;
        }
        info!(
            "\tFound {} satellites ({} misaligned) and {} missing",
            satellites.in_place + satellites.misaligned,
            satellites.misaligned,
            satellites.missing,
        );

        // Add placeholder orbits - They are orbits with only holes
        for position in missing {
            // NOTE: We have no way of recovering the phase of an orbit that isn't there, so
            //       placeholder orbits start their slot sequence at 0.
            let missing_orbit = Orbit::new_placeholder(satellite_spacing, position);
            let idx = self
                .orbits
                .binary_search_by(|slot| slot.raan.total_cmp(&position))
                .err()
                .expect("Duplicate RAAN detected");

            self.orbits.insert(idx, missing_orbit);
        }

        (orbits, satellites, orbit_spacing, satellite_spacing)
    }

    /// A shell is considered to be regular if each of its orbits has the same amount of
    /// regularised orbital slots
    fn is_regular(&self) -> bool {
        let orbit_numbers = self
            .orbits
            .iter()
            .filter(|orbit| orbit.in_position)
            .map(|orbit| orbit.get_regular_slots().len())
            .collect_vec();
        debug!("{:?}", orbit_numbers);
        orbit_numbers.into_iter().all_equal()
            && self
                .orbits
                .iter()
                .filter(|o| o.in_position)
                .all(|o| o.phase.is_some())
    }

    /// Convert a shell to the ideal version of itself
    pub fn idealize(&mut self, (altitude, inclination): (u32, u32), epoch: NaiveDateTime) {
        // Remove out of place orbits
        self.orbits.retain(|orbit| orbit.in_position);

        self.orbits
            .iter_mut()
            .for_each(|orbit| orbit.idealize(altitude, inclination, epoch));
    }

    /// Get the grid representation of this shell: one row per in-place orbit, one column per ideal
    /// slot. `None` marks a hole, i.e. a slot of the ideal constellation with no real satellite in
    /// it. Misaligned orbits and satellites do not appear in the grid.
    pub fn get_grid_by<T>(&self, f: impl Fn(&Satellite) -> T) -> Vec<Vec<Option<T>>> {
        self.orbits
            .iter()
            .filter(|orbit| orbit.in_position)
            .map(|orbit| {
                orbit
                    .get_regular_slots()
                    .iter()
                    .map(|slot| slot.map(&f))
                    .collect()
            })
            .collect()
    }
}

impl From<Vec<Satellite>> for Shell {
    fn from(shell_satellites: Vec<Satellite>) -> Self {
        Shell {
            orbits: sort_into_orbits(shell_satellites)
                .values()
                .map(|orbit| orbit.into())
                .sorted_by(|a: &Orbit, b: &Orbit| a.raan.total_cmp(&b.raan))
                .collect(),
        }
    }
}

#[derive(Serialize, Clone)]
pub struct Orbit {
    /// All satellites in an orbit share the same RAAN [deg]
    pub raan: f64,
    /// The phase offset of this orbit's slot sequence (only available after regularisation)
    pub phase: Option<f64>,
    /// Whether or not this orbit is in position
    pub in_position: bool,
    /// A vector of orbital slots that make up this orbit
    pub satellites: Vec<OrbitalSlot>,
}

impl Orbit {
    /// Get a new placeholder orbit with a given RAAN and satellite spacing. It stands in for an
    /// orbit that the ideal constellation has and the real one is missing, and is therefore made
    /// up entirely of holes.
    fn new_placeholder(spacing: f64, raan: f64) -> Self {
        let n_holes = (360.0 / spacing).round() as u64;
        let satellites = (0..n_holes)
            .map(|slot_id| OrbitalSlot::empty(slot_id as f64 * spacing))
            .collect();

        Orbit {
            raan,
            phase: Some(0.0),
            in_position: true,
            satellites,
        }
    }

    /// Regularize an orbit based on a given spacing between the satellites within it
    fn regularize(&mut self, spacing: f64) -> ElementCounts {
        assert!(
            self.satellites.iter().all(|slot| slot.in_position),
            "Orbit has already been regularized"
        );

        // Mask the satellites depending on their positions within the orbit
        let (mask, missing, phase) = classify_element_positions(
            &self
                .satellites
                .iter()
                .map(|s| s.argument_of_latitude)
                .collect_vec(),
            vec![spacing],
        );
        self.phase = Some(phase);
        let counts = ElementCounts {
            in_place: mask.iter().filter(|in_position| **in_position).count(),
            misaligned: mask.iter().filter(|in_position| !**in_position).count(),
            missing: missing.len(),
        };
        debug!(
            "\tFound {} satellites ({} misaligned) and {} missing",
            mask.len(),
            counts.misaligned,
            counts.missing,
        );

        // Mark the satellites that are out of position
        assert_eq!(self.satellites.len(), mask.len());
        self.satellites
            .iter_mut()
            .zip(mask)
            .for_each(|(sat, in_position)| sat.in_position = in_position);

        // Add the "holes" to the orbit
        for position in missing {
            let idx = self
                .satellites
                .binary_search_by(|slot| slot.argument_of_latitude.total_cmp(&position))
                .err()
                .expect("Duplicate argument_of_latitude detected");

            self.satellites.insert(idx, OrbitalSlot::empty(position));
        }

        counts
    }

    /// Get the regularised orbital slots in this orbit.
    /// A slot is regular if it is either a hole, or a satellite that is in position.
    fn get_regular_slots(&self) -> Vec<Option<&Satellite>> {
        self.satellites
            .iter()
            .filter(|slot| slot.in_position)
            .map(|slot| slot.satellite.as_ref())
            .collect()
    }

    /// Convert an orbit to the ideal version of itself
    pub fn idealize(&mut self, altitude: u32, inclination: u32, epoch: NaiveDateTime) {
        // Remove out of position satellites
        self.satellites.retain(|slot| slot.in_position);
        // Fill every hole with a synthetic satellite
        self.satellites
            .iter_mut()
            .filter(|slot| slot.satellite.is_none())
            .for_each(|slot| slot.fill_hole(altitude, inclination, self.raan, epoch));
        // In an ideal constellation every satellite carries a full set of laser terminals
        self.satellites.iter_mut().for_each(|slot| {
            slot.satellite.as_mut().unwrap().terminals = Terminals::new(IDEAL_TERMINALS)
        });
    }
}

impl From<&Vec<Satellite>> for Orbit {
    fn from(satellites: &Vec<Satellite>) -> Self {
        Orbit {
            // Get the mean RAAN
            raan: (satellites.iter().map(|s| s.get_raan()).sum::<f64>() / satellites.len() as f64),
            satellites: satellites
                .iter()
                .map(|s| s.into())
                .sorted_by(|a: &OrbitalSlot, b: &OrbitalSlot| {
                    a.argument_of_latitude.total_cmp(&b.argument_of_latitude)
                })
                .collect(),
            phase: None,
            in_position: true,
        }
    }
}

#[derive(Serialize, Clone)]
pub struct OrbitalSlot {
    /// The position of a satellite in an orbit is given by their Argument of Latitude [deg]
    pub argument_of_latitude: f64,
    /// Whether or not this satellite is in position
    pub in_position: bool,
    /// The actual satellite object at this position. `None` marks a hole.
    pub satellite: Option<Satellite>,
}

impl OrbitalSlot {
    /// Create an in-position, but empty orbital slot at a specific point
    fn empty(argument_of_latitude: f64) -> Self {
        Self {
            argument_of_latitude,
            in_position: true,
            satellite: None,
        }
    }

    /// Fill a hole with a synthetic satellite
    fn fill_hole(&mut self, altitude: u32, inclination: u32, raan: f64, epoch: NaiveDateTime) {
        assert!(self.satellite.is_none(), "Satellite already present");
        let elements = ElementsConstructor {
            epoch,
            altitude,
            inclination,
            raan,
            argument_of_latitude: self.argument_of_latitude,
        }
        .into();
        let name = format!(
            "{}-{}-{:.2}-{:.2}",
            altitude, inclination, raan, self.argument_of_latitude
        );
        self.satellite = Some(Satellite::synthetic(name, elements, epoch, IDEAL_TERMINALS))
    }
}

impl From<&Satellite> for OrbitalSlot {
    fn from(satellite: &Satellite) -> Self {
        OrbitalSlot {
            argument_of_latitude: satellite.get_argument_of_latitude(),
            in_position: true,
            satellite: Some(satellite.clone()),
        }
    }
}

// ----------------------------------------------------
//                  HELPER FUNCTIONS
// ----------------------------------------------------

/// Snap an angular spacing to an integer number of slots over 360°.
///
/// Example: 14.85° -> 15.0° (24 slots), 5.81° -> 5.806451...° (62 slots).
pub fn snap_spacing_to_integer_slots(spacing: f64) -> f64 {
    if !spacing.is_finite() || spacing <= 0.0 {
        return spacing;
    }

    let slots = (360.0 / spacing).round().max(1.0);
    360.0 / slots
}

/// Finds the dominant modes in a histogram of values.
///
/// Greedily picks peaks in descending count order. A peak is accepted only if
/// its count is within `peak_ratio_cutoff` of the primary. Bins within
/// `peak_bins_apart` of any accepted peak are masked before the next search.
///
/// This is step 1 of §3.1: the dominant mode of the ΔΩ (resp. Δα) histogram is the ideal spacing
/// between orbits (resp. satellites). A shell whose spacing alternates, like shell 2, shows up as
/// two peaks of comparable height and keeps both.
pub fn find_histogram_modes(
    values: &Vec<f64>,
    peak_ratio_cutoff: f64,
    peak_bins_apart: usize,
) -> Result<Vec<f64>, String> {
    if values.is_empty() {
        return Err("No values provided.".into());
    }

    const BINS: usize = 1200;
    let bin_width = 360.0 / BINS as f64;
    let counts = histogram(values, BINS, 0.0, 360.0);

    // Find primary peak
    let (primary_idx, &primary_count) = counts
        .iter()
        .enumerate()
        .max_by_key(|&(_i, &val)| val)
        .expect("empty iterator");
    if primary_count == 0 {
        return Err("All bins empty.".into());
    }

    // Extract the peaks from the histogram
    let mut blocked = vec![false; BINS];
    let mut peaks: Vec<f64> = vec![];
    // Start with the very first one we found
    let mut current = primary_idx;
    loop {
        let count = counts[current];
        let peak = (current as f64 + 0.5) * bin_width;
        let ratio = count as f64 / primary_count as f64;
        // Break out if this peak is below cutoff
        if !peaks.is_empty() && ratio < peak_ratio_cutoff {
            debug!(
                "The next peak ({}) is below cutoff: {} < {}",
                peak, ratio, peak_ratio_cutoff
            );
            break;
        }

        // Push this peak
        peaks.push(peak);
        // Mask it out to find the others
        let lo = current.saturating_sub(peak_bins_apart - 1);
        let hi = (current + peak_bins_apart).min(BINS);
        blocked[lo..hi].fill(true);
        // Find the next one
        match counts
            .iter()
            .enumerate()
            .filter(|(i, _)| !blocked[*i])
            .max_by_key(|(_, &c)| c)
        {
            Some((idx, &c)) if c > 0 => current = idx,
            _ => break,
        }
    }

    Ok(peaks)
}

/// Refines a coarse mode estimate by decomposing each value into the nearest
/// integer multiple of the current estimate and taking the median implied unit.
/// Two passes are used to correct gross errors before tightening.
///
/// This lets gaps that span several ideal slots — the hallmark of missing elements — contribute to
/// the estimate instead of blurring it.
pub fn refine_by_harmonic_median(values: &Vec<f64>, coarse: f64) -> f64 {
    let valid: Vec<f64> = values
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if valid.is_empty() {
        return coarse;
    }

    let mut g = coarse;
    for _ in 0..2 {
        // Clamp k >= 1 to avoid division by zero for values much smaller than g.
        let mut units: Vec<f64> = valid
            .iter()
            .map(|&x| x / (x / g).round().max(1.0))
            .collect();
        units.sort_unstable_by(f64::total_cmp);
        let n = units.len();
        g = if n % 2 == 1 {
            units[n / 2]
        } else {
            (units[n / 2 - 1] + units[n / 2]) / 2.0
        };
    }
    g
}

pub fn histogram(values: &Vec<f64>, nbins: usize, min: f64, max: f64) -> Vec<u32> {
    let bin_width = (max - min) / nbins as f64;
    values.iter().fold(vec![0u32; nbins], |mut counts, &v| {
        if !v.is_finite() || v < min || v > max {
            return counts;
        }

        let idx = if v >= max {
            nbins - 1
        } else {
            ((v - min) / bin_width) as usize
        };
        counts[idx] += 1;
        counts
    })
}

/// Matches irregular positions against a circular grid with cyclic, multi-mode spacing.
///
/// This is step 2 of §3.1 followed by the classification of §3.2. Knowing the ideal spacing is not
/// enough: we also need to know where the sequence of ideal positions starts. We scan candidate
/// phases, place the ideal slots at each of them, and keep the phase that minimizes the total
/// distance between the real elements and their closest ideal slot (the *misalignment*). Every real
/// element then either claims its closest ideal slot, or is reported as misaligned; ideal slots
/// that nothing claims are reported as missing.
///
/// Returns:
/// - a boolean mask over `values` (`true` = aligned to a slot, `false` = misaligned)
/// - a vector of missing slot positions (in degrees on `[0, 360)`).
/// - the selected phase offset in degrees on `[0, sum(spacing))`.
///
/// The slot pattern starts at `phase`, then advances by `spacing[0]`, `spacing[1]`, ...,
/// cycling through `spacing`. The pattern repeats every `sum(spacing)` degrees.
pub fn classify_element_positions(
    values: &Vec<f64>,
    spacing: Vec<f64>,
) -> (Vec<bool>, Vec<f64>, f64) {
    /// `N` in §3.1: how many candidate phases we try within one period of the slot pattern
    const STEPS: usize = 100;
    let loop_size: f64 = spacing.iter().sum();

    let step_size = loop_size / STEPS as f64;
    let mean_spacing = loop_size / spacing.len() as f64;
    let tolerance = mean_spacing * MATCH_TOLERANCE_FRAC;

    // 0) Build the slots we will try to assign the points to
    let build_slots = |phase: f64| {
        let mut centers = Vec::new();
        let mut p = phase;
        let end = phase + 360.0;
        let mut i = 0usize;

        while p < end - 1e-5 {
            centers.push(p.rem_euclid(360.0));
            p += spacing[i % spacing.len()];
            i += 1;
        }

        centers
    };
    // 1) Scan phase offset and pick the one minimizing point-to-grid distances.
    let (best_phase, _best_score) = (0..STEPS)
        .map(|k| {
            let phase = k as f64 * step_size;
            let centers = build_slots(phase);

            let mut score = 0.0;
            for &v in values {
                let mut best = f64::INFINITY;
                for &c in &centers {
                    let raw = (v - c).abs();
                    let d = raw.min(360.0 - raw);
                    if d < best {
                        best = d;
                    }
                }
                score += best;
            }
            (phase, score)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();

    // 2) For each slot, keep at most one best candidate point.
    let centers = build_slots(best_phase);
    let nslots = centers.len();
    let mut best_for_slot: Vec<Option<(usize, f64)>> = vec![None; nslots];

    for (point_idx, &raw_v) in values.iter().enumerate() {
        if !raw_v.is_finite() {
            continue;
        }
        let v = raw_v.rem_euclid(360.0);
        let (slot_idx, dist) = centers
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let raw = (v - c).abs();
                (i, raw.min(360.0 - raw))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();

        if dist <= tolerance {
            match best_for_slot[slot_idx] {
                Some((_current_idx, current_dist)) if current_dist <= dist => {}
                _ => best_for_slot[slot_idx] = Some((point_idx, dist)),
            }
        }
    }

    // 3) Mark assigned points in a boolean mask (leftovers remain misaligned=false).
    let mut aligned_mask = vec![false; values.len()];
    for (idx, _) in best_for_slot.iter().flatten() {
        aligned_mask[*idx] = true;
    }

    // 4) Return the position of each slot that found no candidate.
    let missing_positions = best_for_slot
        .iter()
        .enumerate()
        .filter_map(|(slot_idx, slot)| slot.is_none().then_some(centers[slot_idx]))
        .collect();

    (aligned_mask, missing_positions, best_phase)
}

#[cfg(test)]
mod test {
    use itertools::Itertools;
    use log::info;
    use test_log::test;

    use crate::{
        constellation::{classify_element_positions, Constellation},
        starlink::load_starlink_satellites,
    };

    /// The shell of Figure 3 (shell 9, 572 km / 70°) has a 10° orbit spacing and a 22.5° satellite
    /// spacing, and is missing two of its orbits.
    #[test]
    fn test_starlink_regularization() {
        let mut constellation: Constellation = load_starlink_satellites(false).into();
        let irregularities = constellation.regularize();

        assert_eq!(irregularities.len(), 10, "Starlink operates 10 shells");
        for shell in &irregularities {
            info!(
                "Shell {} ({}-{}): orbit spacing {:?}, satellite spacing {:.2}, orbits {:?}, satellites {:?}",
                shell.shell,
                shell.altitude_km,
                shell.inclination_deg,
                shell.orbit_spacing_deg,
                shell.satellite_spacing_deg,
                shell.orbits,
                shell.satellites,
            );
        }

        let shell_9 = irregularities
            .iter()
            .find(|s| (s.altitude_km, s.inclination_deg) == (572, 70))
            .expect("shell 9 should exist");
        assert_eq!(shell_9.orbit_spacing_deg.len(), 1);
        assert!((shell_9.orbit_spacing_deg[0] - 10.0).abs() < 0.05);
        assert!((shell_9.satellite_spacing_deg - 22.5).abs() < 0.05);
        assert_eq!(shell_9.orbits.missing, 2);
    }

    /// Reproduces Table 1 of the paper on the CelesTrak/SatCat snapshot shipped with this artifact
    /// (collected on March 6th 2026). Shells 5 and 6 are excluded from the paper's analysis — the
    /// former is made up almost entirely of satellites without laser terminals, the latter has a
    /// layout too peculiar for orbit- or satellite-level irregularities to be well defined — so the
    /// paper reports no counts for them and neither do we here.
    #[test]
    fn test_table_1() {
        // shell -> (orbit spacing, orbits, satellite spacing, satellites), each as (in place,
        // misaligned, missing)
        let expected: [(usize, &[f64], (usize, usize, usize), f64, (usize, usize, usize)); 8] = [
            (0, &[13.85], (26, 0, 0), 30.0, (307, 24, 5)),
            (1, &[15.0], (24, 4, 0), 27.69, (295, 14, 17)),
            (2, &[10.35, 2.55], (56, 0, 0), 5.81, (2397, 5, 1075)),
            (3, &[3.64], (97, 0, 2), 7.06, (1840, 1, 3107)),
            (4, &[5.0], (72, 5, 0), 20.0, (1170, 190, 126)),
            (7, &[12.86], (28, 1, 0), 12.0, (269, 262, 571)),
            (8, &[60.0], (5, 1, 1), 20.0, (80, 26, 10)),
            (9, &[10.0], (34, 0, 2), 22.5, (494, 154, 50)),
        ];

        let mut constellation: Constellation = load_starlink_satellites(false).into();
        let irregularities = constellation.regularize();

        for (index, orbit_spacing, orbits, satellite_spacing, satellites) in expected {
            let shell = &irregularities[index];
            let found_orbit_spacing = shell
                .orbit_spacing_deg
                .iter()
                .sorted_by(|a, b| b.total_cmp(a))
                .collect_vec();

            assert_eq!(
                found_orbit_spacing.len(),
                orbit_spacing.len(),
                "shell {index} orbit spacing"
            );
            for (found, expected) in found_orbit_spacing.iter().zip(orbit_spacing) {
                assert!(
                    (*found - expected).abs() < 0.005,
                    "shell {index} orbit spacing: {found} != {expected}"
                );
            }
            assert!(
                (shell.satellite_spacing_deg - satellite_spacing).abs() < 0.005,
                "shell {index} satellite spacing: {} != {satellite_spacing}",
                shell.satellite_spacing_deg
            );
            assert_eq!(
                (
                    shell.orbits.in_place,
                    shell.orbits.misaligned,
                    shell.orbits.missing
                ),
                orbits,
                "shell {index} orbits"
            );
            assert_eq!(
                (
                    shell.satellites.in_place,
                    shell.satellites.misaligned,
                    shell.satellites.missing
                ),
                satellites,
                "shell {index} satellites"
            );
        }
    }

    /// Idealizing a regularized constellation leaves every in-place orbit full of satellites and
    /// no holes behind.
    #[test]
    fn test_starlink_idealization() {
        let mut constellation: Constellation = load_starlink_satellites(false).into();
        constellation.regularize();
        constellation.idealize();

        for (key, shell) in &constellation.shells {
            let slots_per_orbit = shell
                .orbits
                .iter()
                .map(|orbit| orbit.satellites.len())
                .collect_vec();
            assert!(
                slots_per_orbit.iter().all_equal(),
                "Shell {:?} has orbits of differing lengths: {:?}",
                key,
                slots_per_orbit
            );
            assert!(
                shell
                    .orbits
                    .iter()
                    .flat_map(|orbit| orbit.satellites.iter())
                    .all(|slot| slot.satellite.is_some()),
                "Shell {:?} still contains holes after idealization",
                key
            );
        }
    }

    /// Exercises the phase finding and classification of §3.1/§3.2 on three hand-checked cases,
    /// the last of which uses the alternating spacing of shell 2.
    #[test]
    fn test_grid_matching() {
        let test_cases: Vec<(Vec<f64>, Vec<f64>, usize, usize)> = vec![
            (
                vec![
                    5.366500544748962,
                    20.306717411348536,
                    25.7843187721124,
                    45.50986768745415,
                    65.6037323953358,
                    85.59681155239942,
                    100.423181315117,
                    105.58612239699713,
                    125.65861770882468,
                    145.61043307632278,
                    165.86353000022186,
                    185.73693442731567,
                    220.4983485606609,
                    225.58520313528473,
                    260.01932056976307,
                    265.63972791683443,
                    280.6062019077056,
                    285.62633668609607,
                    305.5887995573091,
                    320.2417489829826,
                    325.54847674937673,
                    345.5208955902974,
                ],
                vec![20.0],
                6, // expected misaligned
                2, // expected missing
            ),
            (
                vec![
                    0.5861500641670683,
                    5.651491891271888,
                    10.55775587807374,
                    15.692010870323447,
                    20.585005099949797,
                    25.640665747078863,
                    30.649826995081664,
                    35.65564892172807,
                    40.66587572227569,
                    45.546348224110226,
                    50.7037522914647,
                    52.3687226571008,
                    55.677282043624345,
                    60.551416794155216,
                    65.68028292234725,
                    70.57100361379527,
                    75.66816289139483,
                    78.15189924864478,
                    80.6850639090591,
                    85.65763776990413,
                    90.68410241439881,
                    95.67318385275215,
                    100.68939377727794,
                    105.6894216335861,
                    110.66444073994975,
                    115.67371065567747,
                    120.56625245391407,
                    125.66937970748214,
                    130.64626867622596,
                    135.6776957351307,
                    140.64432120313435,
                    143.08254027747364,
                    145.63476013185976,
                    150.65636479533944,
                    155.670820210042,
                    160.658009178081,
                    165.65786545129058,
                    170.67152851726533,
                    175.67833594951125,
                    180.67897868122392,
                    185.67660724626546,
                    190.65549378404475,
                    195.6696437086036,
                    200.66423521667357,
                    205.68555619190317,
                    210.6558901304408,
                    215.70510937875704,
                    220.56360521981995,
                    225.63455398974054,
                    230.5423940500266,
                    235.66114808270885,
                    240.59250165453648,
                    245.69744057541507,
                    248.14981617360905,
                    250.53617037534002,
                    255.65790554463103,
                    260.708693703206,
                    265.62893626766004,
                    270.6238250096771,
                    275.67023595912565,
                    280.6008565076708,
                    285.6780931592737,
                    290.6007110876359,
                    295.6618558012312,
                    300.6751928720869,
                    305.626708311114,
                    310.5790828580146,
                    315.6313358450294,
                    320.5416024897436,
                    325.5930694823648,
                    330.622428930036,
                    335.71279864421956,
                    340.6068940488532,
                    345.6904350722127,
                    348.1403444149429,
                    350.57350401118896,
                    355.64327339427973,
                ],
                vec![5.0],
                5, // expected misaligned
                0, // expected missing
            ),
            (
                vec![
                    2.7854706071168667,
                    4.993858685288448,
                    15.593755392391666,
                    16.261619225291554,
                    28.486004242712255,
                    30.995127816492474,
                    41.36792694798112,
                    43.91147628729647,
                    54.17941427543561,
                    56.71947044704527,
                    66.93341594645165,
                    69.61260366067822,
                    79.94387242879371,
                    82.46275761740863,
                    92.69793607087936,
                    95.3382027400258,
                    105.63142992107508,
                    108.17601184981609,
                    118.43811788453698,
                    121.0404072681774,
                    131.04316701883843,
                    133.87327562661406,
                    144.11090031937331,
                    146.72805359581722,
                    157.09856088639017,
                    159.58468226692693,
                    169.86672700297672,
                    172.37473765821596,
                    182.72281103032523,
                    185.2674823693775,
                    195.4274113376943,
                    198.163663205195,
                    208.4754023280602,
                    221.34082642022744,
                    223.9560180765133,
                    234.1265897555399,
                    236.73565217207056,
                    247.09236849891977,
                    249.6414263189651,
                    259.87429737452226,
                    262.4643657815279,
                    272.73031317914297,
                    275.3164329506236,
                    285.67338415735895,
                    288.1665160243835,
                    298.4562652147762,
                    301.0080804953089,
                    311.38129929959086,
                    313.8748162385359,
                    324.2006230004583,
                    326.76386351273214,
                    337.0586928325781,
                    339.55383817961206,
                    349.8762521617427,
                    352.4246554105295,
                ],
                vec![2.55, 10.35],
                1, // expected misaligned
                2, // expected missing
            ),
        ];

        for (values, spacing, expected_misaligned, expected_missing) in test_cases {
            let (aligned_mask, missing_positions, phase_offset) =
                classify_element_positions(&values, spacing.clone());

            let misaligned_count = aligned_mask.iter().filter(|&&ok| !ok).count();
            let missing_count = missing_positions.len();
            let loop_size: f64 = spacing.iter().sum();

            assert_eq!(aligned_mask.len(), values.len());
            assert!(missing_positions.iter().all(|v| v.is_finite()));
            assert!((0.0..loop_size).contains(&phase_offset));
            assert_eq!(misaligned_count, expected_misaligned);
            assert_eq!(missing_count, expected_missing);

            info!("spacing={spacing:?} phase offset: {phase_offset}");
            info!("spacing={spacing:?} misaligned={misaligned_count}, missing={missing_count}");
            info!(
                "spacing={spacing:?} misaligned values: {:?}",
                values
                    .iter()
                    .zip(aligned_mask.iter())
                    .filter_map(|(val, &keep)| (!keep).then_some(val))
                    .collect_vec()
            );
            info!("spacing={spacing:?} missing slots: {:?}", missing_positions);
        }
    }
}
