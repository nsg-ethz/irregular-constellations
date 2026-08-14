//! Imposing regular hexGrid topologies onto an irregular constellation (§4 of the paper).
//!
//! Once a shell has been regularized, its in-place orbits and satellites form a grid: one row per
//! orbit, one column per ideal intra-orbit slot, with holes where the real constellation is missing
//! a satellite. That grid is the scaffold on which we lay out inter-satellite links, following the
//! hexGrid pattern — a hexagonal 3-ISL variant of the +Grid, in which every satellite links to its
//! successor within its own orbit and, on a checkerboard pattern, to one peer in the next orbit.
//!
//! Three variants are built, which differ only in what they do about the irregularities:
//!
//! * [`Variant::Naive`] takes the grid as-is. Where the pattern calls for a neighbor that is
//!   missing, no link is established and the laser terminal stays idle.
//! * [`Variant::Patched`] additionally bridges runs of missing satellites within an orbit, linking
//!   the two satellites that bracket the gap whenever that longer link is physically realizable.
//! * [`Variant::Ideal`] is the same construction applied to an idealized shell (see
//!   [`crate::constellation::Constellation::idealize`]), and therefore has no holes to work around.
//!   It serves as the regular reference topology.
//!
//! Links are only ever established between satellites that have a laser terminal to spare, and only
//! when the link is physically realizable, i.e. when it does not cut through the atmosphere.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use itertools::Itertools;
use log::{debug, info};
use petgraph::{
    graph::{NodeIndex, UnGraph},
    visit::EdgeRef,
};
use serde::{ser::SerializeStruct, Serialize, Serializer};

use crate::{
    constellation::Shell,
    satellite::{Satellite, EARTH_RADIUS},
};

/// The height in `m` below which an ISL is considered to be cutting through the atmosphere, and
/// therefore not realizable (as per "Achieving ⪆99% link uptime").
const ATMOSPHERE_HEIGHT: f64 = 100_000.0;
/// The speed of light in free space in `m/ms`
pub const SPEED_OF_LIGHT: f64 = 299_792_458.0 / 1000.0;

/// Which of the three topologies of §4 to build
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// The grid taken as-is over the real, irregular constellation
    Naive,
    /// The real grid, with runs of missing satellites bridged where possible
    Patched,
    /// The reference grid over the idealized constellation
    Ideal,
}

impl Variant {
    pub fn name(&self) -> &'static str {
        match self {
            Variant::Naive => "naive",
            Variant::Patched => "patched",
            Variant::Ideal => "ideal",
        }
    }
}

/// What role a link plays in the grid
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IslKind {
    /// A link to the next satellite within the same orbit
    IntraOrbit,
    /// A checkerboard link to a peer in the neighboring orbit
    InterOrbit,
    /// A link bridging a run of missing satellites within an orbit (patched topology only)
    Patch,
}

/// A satellite, together with the logical grid position it occupies in its shell
#[derive(Debug, Clone)]
pub struct Node {
    pub satellite: Satellite,
    /// Index of the orbit within the shell, in order of increasing RAAN
    pub orbit: usize,
    /// Index of the ideal slot within the orbit
    pub slot: usize,
}

/// An established inter-satellite link
#[derive(Debug, Clone, Copy)]
pub struct Isl {
    pub kind: IslKind,
    /// The length of the link in `m`
    pub length: f64,
    /// The one-way propagation delay over the link in `ms`
    pub delay: f64,
}

/// The identity of a shell: its index (by increasing altitude) and its defining parameters
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ShellId {
    pub shell: usize,
    pub altitude_km: u32,
    pub inclination_deg: u32,
}

/// The ISL topology of a single shell
pub struct Topology {
    pub shell: ShellId,
    pub variant: Variant,
    pub epoch: NaiveDateTime,
    pub graph: UnGraph<Node, Isl>,
    /// How many links the construction wanted to establish but could not, because one of the two
    /// satellites had no laser terminal left to spare
    pub links_without_terminal: usize,
    /// How many links the construction wanted to establish but could not, because the link would
    /// have cut through the atmosphere
    pub links_blocked_by_atmosphere: usize,
}

impl Topology {
    /// Build the hexGrid topology of `variant` over an already regularized `shell`.
    ///
    /// [`Variant::Ideal`] expects a shell that has also been idealized; the other two expect the
    /// real one. Only in-place orbits and satellites take part: misaligned elements have no grid
    /// position, and are therefore left out of the topology entirely.
    pub fn hex_grid(
        shell_id: ShellId,
        shell: &Shell,
        epoch: NaiveDateTime,
        variant: Variant,
    ) -> Self {
        let mut topology = Topology {
            shell: shell_id,
            variant,
            epoch,
            graph: UnGraph::new_undirected(),
            links_without_terminal: 0,
            links_blocked_by_atmosphere: 0,
        };

        // The grid of the shell: one row per in-place orbit, one column per ideal slot, holes where
        // the real constellation has no satellite.
        let grid = shell.get_grid_by(|satellite| satellite.clone());
        let node_grid: Vec<Vec<Option<NodeIndex>>> = grid
            .iter()
            .enumerate()
            .map(|(orbit, row)| {
                row.iter()
                    .enumerate()
                    .map(|(slot, cell)| {
                        cell.as_ref().map(|satellite| {
                            topology.graph.add_node(Node {
                                satellite: satellite.clone(),
                                orbit,
                                slot,
                            })
                        })
                    })
                    .collect()
            })
            .collect();

        topology.wire_hex_grid(&node_grid);
        if variant == Variant::Patched {
            topology.patch_holes(&node_grid);
        }

        info!(
            "Shell {} ({}-{}), {} grid: {} satellites, {} ISLs, {:.0}% of the laser terminals in use ({} links blocked by the atmosphere)",
            shell_id.shell,
            shell_id.altitude_km,
            shell_id.inclination_deg,
            variant.name(),
            topology.graph.node_count(),
            topology.graph.edge_count(),
            100.0 * topology.terminals_in_use() as f64 / topology.terminals_available().max(1) as f64,
            topology.links_blocked_by_atmosphere,
        );

        topology
    }

    /// Lay the hexGrid pattern over the grid:
    ///   1. every satellite links to the next one in its own orbit (wrapping around), and
    ///   2. checkerboard-selected satellites additionally link to the next orbit at the same slot
    ///      (also wrapping around).
    ///
    /// Where the pattern calls for a satellite that isn't there, no link is established.
    fn wire_hex_grid(&mut self, grid: &[Vec<Option<NodeIndex>>]) {
        let orbit_count = grid.len();
        if orbit_count == 0 {
            return;
        }
        assert!(grid.iter().map(|orbit| orbit.len()).all_equal());
        let slot_count = grid[0].len();
        debug!("This grid is {orbit_count} by {slot_count}");

        for orbit_idx in 0..orbit_count {
            for slot_idx in 0..slot_count {
                let Some(&a) = grid[orbit_idx][slot_idx].as_ref() else {
                    continue;
                };

                // --- 1) Intra-orbit link to the next slot.
                let next_slot = (slot_idx + 1) % slot_count;
                if let Some(&b) = grid[orbit_idx][next_slot].as_ref() {
                    self.add_isl(a, b, IslKind::IntraOrbit);
                }

                // --- 2) Inter-orbit link only for checkerboard-selected nodes.
                // Using (orbit + slot) parity ensures a single intended inter-orbit choice.
                let next_orbit = (orbit_idx + 1) % orbit_count;
                // Flip parity on the seam when orbit_count is odd, so the wrap-around pair uses
                // the slots the interior pattern left uncovered.
                let is_seam = next_orbit == 0 && orbit_count % 2 == 1;

                if (orbit_idx + slot_idx) % 2 == 0 {
                    let target_slot = if is_seam {
                        (slot_idx + 1) % slot_count
                    } else {
                        slot_idx
                    };
                    if let Some(&c) = grid[next_orbit][target_slot].as_ref() {
                        self.add_isl(a, c, IslKind::InterOrbit);
                    }
                }
            }
        }
    }

    /// Bridge the holes of each orbit: wherever a run of missing satellites interrupts the
    /// intra-orbit chain, connect the two satellites bracketing it with a single longer link.
    ///
    /// First generation satellites count as holes too: they carry no laser terminals, so the chain
    /// is just as broken by them as it is by a satellite that isn't there.
    fn patch_holes(&mut self, grid: &[Vec<Option<NodeIndex>>]) {
        // Avoid spending terminals on duplicate links for the same pair.
        let mut attempted_pairs: HashSet<(usize, usize)> = HashSet::new();
        let mut attempts = 0usize;

        for orbit in grid {
            let slot_count = orbit.len();
            if slot_count == 0 {
                continue;
            }

            let slot_is_hole: Vec<bool> = orbit
                .iter()
                .map(|slot| match slot {
                    None => true,
                    Some(node) => self.graph[*node].satellite.terminals.max_available == 0,
                })
                .collect();

            if slot_is_hole.iter().all(|is_hole| *is_hole) {
                continue;
            }

            for hole_start in 0..slot_count {
                if !slot_is_hole[hole_start] {
                    continue;
                }

                // Start processing only when this index is the first hole in a run.
                let prev_idx = (hole_start + slot_count - 1) % slot_count;
                if slot_is_hole[prev_idx] {
                    continue;
                }

                // Walk forward to the first non-hole after this hole run.
                let mut next_idx = hole_start;
                while slot_is_hole[next_idx] {
                    next_idx = (next_idx + 1) % slot_count;
                    if next_idx == hole_start {
                        break;
                    }
                }
                if slot_is_hole[next_idx] {
                    continue;
                }

                let (Some(&before), Some(&after)) = (orbit[prev_idx].as_ref(), orbit[next_idx].as_ref())
                else {
                    continue;
                };
                if before == after {
                    continue;
                }

                let pair = (before.index().min(after.index()), before.index().max(after.index()));
                if attempted_pairs.insert(pair) {
                    self.add_isl(before, after, IslKind::Patch);
                    attempts += 1;
                }
            }
        }

        debug!(
            "Patched the grid of shell {} with {} hole-jump attempts",
            self.shell.shell, attempts
        );
    }

    /// Try to establish an ISL between two satellites. The link is only established if both ends
    /// still have a free laser terminal and the link is physically realizable; otherwise it is
    /// counted as rejected and the terminals stay idle.
    fn add_isl(&mut self, a: NodeIndex, b: NodeIndex, kind: IslKind) -> bool {
        assert!(a != b, "Trying to add a self link");

        if self.graph[a].satellite.terminals.available() == 0
            || self.graph[b].satellite.terminals.available() == 0
        {
            self.links_without_terminal += 1;
            return false;
        }

        let Some(isl) = self.build_isl(&self.graph[a].satellite, &self.graph[b].satellite, kind)
        else {
            debug!(
                "ISL {}-{} would cut through the atmosphere",
                self.graph[a].satellite.name, self.graph[b].satellite.name
            );
            self.links_blocked_by_atmosphere += 1;
            return false;
        };

        self.graph[a].satellite.terminals.claim();
        self.graph[b].satellite.terminals.claim();
        self.graph.add_edge(a, b, isl);
        true
    }

    /// Describe the link between two satellites, or `None` if it is not physically realizable.
    ///
    /// A link is realizable when the chord between the two satellites stays clear of the
    /// atmosphere. Comparing the sagitta of that chord against the orbit's altitude tells us how
    /// deep the link dips towards the Earth.
    fn build_isl(&self, a: &Satellite, b: &Satellite, kind: IslKind) -> Option<Isl> {
        let length = a.position.euclidean_distance(&b.position);
        let orbit_altitude = a.get_altitude();
        let orbit_radius = orbit_altitude + EARTH_RADIUS;
        let sagitta = orbit_radius - f64::sqrt(orbit_radius.powi(2) - (length / 2.0).powi(2));

        (sagitta <= orbit_altitude - ATMOSPHERE_HEIGHT).then_some(Isl {
            kind,
            length,
            delay: length / SPEED_OF_LIGHT,
        })
    }

    /// The total number of laser terminals carried by the satellites of this topology
    pub fn terminals_available(&self) -> u32 {
        self.graph
            .node_weights()
            .map(|node| node.satellite.terminals.max_available)
            .sum()
    }

    /// How many of those terminals ended up carrying an ISL. The complement is the capacity that
    /// irregularities leave unused (§5.1).
    pub fn terminals_in_use(&self) -> u32 {
        self.graph
            .node_weights()
            .map(|node| node.satellite.terminals.max_available - node.satellite.terminals.available())
            .sum()
    }
}

impl Serialize for Topology {
    /// Serializes the topology into the node-link format read by NetworkX's `node_link_graph`.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("NetworkXGraph", 5)?;

        state.serialize_field("directed", &false)?;
        state.serialize_field("multigraph", &false)?;

        // Graph level metadata, which NetworkX exposes as `G.graph`
        let mut metadata: HashMap<&str, serde_json::Value> = HashMap::new();
        metadata.insert("shell", self.shell.shell.into());
        metadata.insert("altitude_km", self.shell.altitude_km.into());
        metadata.insert("inclination_deg", self.shell.inclination_deg.into());
        metadata.insert("variant", self.variant.name().into());
        metadata.insert("epoch", self.epoch.to_string().into());
        metadata.insert("terminals_available", self.terminals_available().into());
        metadata.insert("terminals_in_use", self.terminals_in_use().into());
        state.serialize_field("graph", &metadata)?;

        let nodes = self
            .graph
            .node_weights()
            .map(|node| {
                let satellite = &node.satellite;
                let (lat, lon): (f64, f64) = satellite.position.into();
                serde_json::json!({
                    "id": satellite.name,
                    "orbit": node.orbit,
                    "slot": node.slot,
                    "raan_deg": satellite.get_raan(),
                    "arg_lat_deg": satellite.get_argument_of_latitude(),
                    "lat_deg": lat.to_degrees(),
                    "lon_deg": lon.to_degrees(),
                    "altitude_km": satellite.get_altitude() / 1000.0,
                    "isl_terminals": satellite.terminals.max_available,
                    "isl_terminals_in_use":
                        satellite.terminals.max_available - satellite.terminals.available(),
                    "synthetic": satellite.synthetic,
                })
            })
            .collect_vec();
        state.serialize_field("nodes", &nodes)?;

        let links = self
            .graph
            .edge_references()
            .map(|edge| {
                let isl = edge.weight();
                serde_json::json!({
                    "source": self.graph[edge.source()].satellite.name,
                    "target": self.graph[edge.target()].satellite.name,
                    "kind": isl.kind,
                    "length_km": isl.length / 1000.0,
                    "delay_ms": isl.delay,
                })
            })
            .collect_vec();
        state.serialize_field("links", &links)?;

        state.end()
    }
}

#[cfg(test)]
mod test {
    use itertools::Itertools;
    use test_log::test;

    use super::*;
    use crate::{constellation::Constellation, starlink::load_starlink_satellites};

    /// No satellite may carry more ISLs than it has laser terminals, in any of the three variants.
    #[test]
    fn test_terminal_budget_is_respected() {
        let (satellites, epoch) = load_starlink_satellites(false);
        let mut real: Constellation = (satellites, epoch).into();
        real.regularize();
        let mut ideal = real.clone();
        ideal.idealize();

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
                let topology = Topology::hex_grid(
                    shell_id,
                    &constellation.shells[key],
                    epoch,
                    variant,
                );

                for node in topology.graph.node_indices() {
                    let satellite = &topology.graph[node].satellite;
                    let degree = topology.graph.edges(node).count();
                    assert!(
                        degree <= satellite.terminals.max_available as usize,
                        "{} has {} ISLs but only {} terminals",
                        satellite.name,
                        degree,
                        satellite.terminals.max_available
                    );
                    assert_eq!(
                        degree,
                        (satellite.terminals.max_available - satellite.terminals.available())
                            as usize,
                        "terminal accounting disagrees with the graph for {}",
                        satellite.name
                    );
                }
            }
        }
    }
}
