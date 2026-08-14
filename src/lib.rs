//! Reconstruction and analysis of the (irregular) Starlink constellation.
//!
//! The pipeline implemented by this crate is the one described in "All But Regular: Revisiting the
//! Starlink Constellation" (LEO-NET '26), and runs in three stages:
//!
//! 1. [`starlink`] loads public measurements (CelesTrak orbital elements + SatCat metadata), keeps
//!    the operational satellites, synchronizes them to a common epoch and groups them into shells
//!    and orbital planes.
//! 2. [`constellation`] recovers the *ideal* regular constellation hiding behind those
//!    measurements — the orbit spacing, the intra-orbit satellite spacing and the phase offsets —
//!    and classifies every orbit and satellite as in-place, misaligned or missing.
//! 3. [`topology`] uses the recovered grid as a scaffold to build hexGrid inter-satellite
//!    topologies on top of the real (naive, patched) and idealized (ideal) constellations.
pub mod constellation;
pub mod elements;
pub mod satcat;
pub mod satellite;
pub mod starlink;
pub mod topology;
