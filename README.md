# All But Regular: Revisiting the Starlink Constellation

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://github.com/user-attachments/assets/6427a3c5-8343-4b8c-a631-3433a5552413">
  <source media="(prefers-color-scheme: light)" srcset="https://github.com/user-attachments/assets/6427a3c5-8343-4b8c-a631-3433a5552413">
  <img alt="Hero image" src="https://github.com/user-attachments/assets/6427a3c5-8343-4b8c-a631-3433a5552413">
</picture>

Code and data accompanying the paper *All But Regular: Revisiting the Starlink Constellation*
(LEO-NET '26) by Pietro Ronchetti, Sushovan Das, Laurent Vanbever and Stefano Vissicchio.

Starting from public Starlink measurements, this artifact reconstructs the constellation, recovers
the regular structure that best explains it, classifies how the real constellation deviates from
that ideal, and builds the inter-satellite topologies those deviations produce. It covers §2–§4 of
the paper: the reconstruction and the irregularity classification (Table 1), and the three hexGrid
topologies laid over the result. The network simulations of §5 are run on those topologies and are
not part of this artifact.

## Citation

```bibtex
@inproceedings{ronchetti2026allbutregular,
  title     = {All But Regular: Revisiting the Starlink Constellation},
  author    = {Ronchetti, Pietro and Das, Sushovan and Vanbever, Laurent and Vissicchio, Stefano},
  booktitle = {Proceedings of the 4th Workshop on LEO Networking and Communication (LEO-NET '26)},
  year      = {2026},
  address   = {Denver, CO, USA},
  publisher = {Association for Computing Machinery},
  doi       = {10.1145/3789240.3827597},
  isbn      = {979-8-4007-2467-1/26/08}
}
```

## Running it

```sh
cargo run --release
```

Takes a few seconds and writes everything into `out/`. Options:

| Flag | Effect |
| --- | --- |
| `--refresh` | Download fresh data from CelesTrak and SatCat before running, replacing the snapshot in `data/` |
| `--out DIR` | Write the outputs somewhere other than `out/` |

Set `RUST_LOG=debug` for a per-orbit account of the classification. `cargo test --release` checks
the pipeline end to end, including a regression test that pins Table 1.

## Input data

Both sources are shipped with the artifact, so a plain run is fully offline and reproducible. They
are snapshots taken on **March 6th 2026**, the date the paper's results refer to.

| File | Source | What we use it for |
| --- | --- | --- |
| `data/CelesTrak/starlink.json` | [CelesTrak](https://celestrak.org/NORAD/elements/gp.php?GROUP=starlink&FORMAT=json) | Orbital elements of every Starlink satellite, from which we derive altitude, inclination, RAAN (Ω) and argument of latitude (α) |
| `data/SatCat/satcat.tsv` | [GCAT / Jonathan's Space Report](https://planet4589.org/space/gcat/tsv/cat/satcat.tsv) | Which satellites are operational, and which bus they fly — the bus decides how many laser terminals, and therefore how many ISLs, a satellite has |

`--refresh` re-fetches both from those URLs, so the same pipeline can be re-run on data collected at
any other time.

## Outputs

### `out/irregularities.csv`

One row per shell — the paper's Table 1. For orbits and for satellites alike, it reports the
recovered ideal spacing and how many elements are *in place*, *misaligned* (present in the real
constellation, but not at an ideal position) and *missing* (present in the ideal constellation, but
absent from the real one).

```
shell,altitude_km,inclination_deg,orbit_spacing_deg,orbits_in_place,orbits_misaligned,orbits_missing,satellite_spacing_deg,satellites_in_place,satellites_misaligned,satellites_missing
0,356,43,13.85,26,0,0,30.00,307,24,5
1,360,53,15.00,24,4,0,27.69,295,14,17
2,475,53,10.35/2.55,56,0,0,5.81,2397,5,1075
...
```

A shell whose orbits alternate between two spacings, like shell 2, reports both values.

Two shells are excluded from the paper's analysis but still appear here, since the pipeline has no
reason to hide them: shell 5 flies almost only first-generation satellites, which carry no laser
terminals and can form no ISL topology at all, and shell 6 has a layout so peculiar — its orbital
planes cluster into two groups separated by wide empty spaces — that orbit- and satellite-level
irregularities are ill defined for it. Treat their rows with care.

### `out/networks/shell_<id>_<altitude>_<inclination>_<variant>.json`

The ISL topology of each shell, in the three variants of §4, in NetworkX's node-link JSON format:

```python
import json, networkx as nx
graph = nx.node_link_graph(json.load(open("out/networks/shell_9_572_70_naive.json")), edges="links")
```

* **`naive`** — the hexGrid taken as-is over the real constellation. Each in-place satellite links to
  its successor within its orbit and, on a checkerboard pattern, to a peer in the adjacent orbit.
  Where the pattern calls for a missing satellite, no link is established and the laser terminal
  stays idle.
* **`patched`** — the same, plus a bridge across each run of missing satellites within an orbit,
  linking the two satellites that bracket the gap whenever that longer link is physically
  realizable.
* **`ideal`** — the same construction over the idealized constellation, in which missing positions
  have been filled with synthetic satellites and misaligned ones removed. The regular reference.

Nodes carry their grid position (`orbit`, `slot`), their orbital parameters, their geodetic
position at the snapshot epoch, their laser terminal budget and how much of it is in use, and
whether they are `synthetic`. Links carry their length and one-way propagation delay, and whether
they are an `intra_orbit`, `inter_orbit` or `patch` link. Graph-level metadata sits in `graph`.

Misaligned satellites have no grid position, and so — as in the paper — take no part in any
topology. They are counted in `irregularities.csv`, not present in these graphs.

## How it works

| Stage | Where | What happens |
| --- | --- | --- |
| Load and group (§2) | `src/starlink.rs`, `src/satcat.rs` | Keep the operational satellites, synchronize them to a common epoch with SGP4, cluster them into shells by (altitude, inclination) and into orbital planes by RAAN |
| Recover the ideal spacing (§3.1, step 1) | `Shell::get_orbit_spacing`, `Shell::get_satellite_spacing` | Take the dominant mode of the ΔΩ and Δα histograms, refine it against the harmonics that gaps of several slots produce, and snap it to an integer number of slots over 360° |
| Find the phase offsets (§3.1, step 2) | `classify_element_positions` | Scan candidate phases and keep the one minimizing the total distance from the real elements to their closest ideal slot |
| Classify the irregularities (§3.2) | `Shell::regularize` | Match every real element to its ideal slot within a tolerance; unmatched elements are misaligned, unclaimed slots are missing |
| Idealize (§4) | `Constellation::idealize` | Drop the misaligned elements, fill every hole with a synthetic satellite |
| Impose the grids (§4) | `src/topology.rs` | Lay the hexGrid pattern over the recovered grid, respecting each satellite's laser terminal budget and establishing only physically realizable links |

## Notes and known deviations

* **Physically realizable links.** An ISL is only established when it does not cut through the
  atmosphere (modelled at 100 km). Attempts that fail this test are dropped without spending a laser
  terminal, which is what §4 describes. The simulator that produced the paper's Table 2 instead
  charged the terminals for those attempts, so the *wasted link* fractions derived from these
  outputs are higher than the ones the table reports — most visibly for shell 8, whose 60° orbit
  spacing makes most of its inter-orbit links unrealizable, and for the patched grids, whose long
  hole-bridging links are the ones most often blocked. The set of *usable* links is unaffected.
* **Shell 2's wasted links.** The naive grid of shell 2 leaves 35% of its terminals idle here
  against the 31% reported in Table 2. The hexGrid construction was reworked after the paper was
  submitted, and shell 2 — the one shell with an alternating orbit spacing — is the one affected.
* **Placeholder orbits.** There is no way to recover the phase of an orbit that isn't there, so
  orbits reported as missing start their slot sequence at 0° when they are filled in for the ideal
  topology.
* **Index stability.** The `orbit` and `slot` indices are relative to a single snapshot. RAAN and
  argument of latitude drift over time, so they are not stable across snapshots taken on different
  days.

## Layout

```
src/starlink.rs        loading the measurements, grouping into shells and orbits (§2)
src/satcat.rs          parser for the GCAT satellite catalogue
src/satellite.rs       a single satellite: SGP4 propagation, orbital parameters, laser terminals
src/elements.rs        synthesizing the orbital elements of an ideal satellite
src/constellation.rs   recovering the ideal constellation, classifying irregularities (§3)
src/topology.rs        the naive, patched and ideal hexGrid topologies (§4)
src/main.rs            the end-to-end pipeline
```

## License

Released under the [Affero GNU General Public License v3.0](LICENSE).

## Contact

Pietro Ronchetti — `pietroro@ethz.ch` — Networked Systems Group, ETH Zurich
