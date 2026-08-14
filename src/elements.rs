use chrono::NaiveDateTime;
use std::f64::consts::PI;

use sgp4::{Classification, Elements, WGS84};

pub struct ElementsConstructor {
    /// The epoch of these Elements - satellite is in this exact position as of this time
    pub epoch: NaiveDateTime,
    /// Altitude of the orbit in [km]
    pub altitude: u32,
    /// Inclination of this orbit in [deg]
    pub inclination: u32,
    /// RAAN of this orbit in [deg]
    pub raan: f64,
    /// Argument of latitude for this satellite in [deg]
    pub argument_of_latitude: f64,
}

impl From<ElementsConstructor> for Elements {
    fn from(orbit_position: ElementsConstructor) -> Self {
        // Mean motion from circular orbit radius.
        // Inverse of the altitude math used in `satellite::get_orbit_altitude`
        let semi_major_axis_km = WGS84.ae + orbit_position.altitude as f64;
        let semi_major_axis_earth_radii = semi_major_axis_km / WGS84.ae;
        let n_rad_per_min = WGS84.ke / semi_major_axis_earth_radii.powf(1.5);
        let mean_motion_rev_per_day = n_rad_per_min * (24.0 * 60.0) / (2.0 * PI);

        // Circular orbit convention: e = 0, set ω = 0 and store u in M.
        // For circular orbits, argument of latitude is u = ω + ν and M ≈ ν.
        // Hence with ω = 0, M = u.
        let mean_anomaly = orbit_position.argument_of_latitude.rem_euclid(360.0);

        Elements {
            object_name: None,
            international_designator: None,
            norad_id: 0,
            classification: Classification::Unclassified,
            datetime: orbit_position.epoch,
            mean_motion_dot: 0.0,
            mean_motion_ddot: 0.0,
            drag_term: 0.0,
            element_set_number: 1,
            inclination: orbit_position.inclination as f64,
            right_ascension: orbit_position.raan.rem_euclid(360.0),
            eccentricity: 0.0,
            argument_of_perigee: 0.0,
            mean_anomaly,
            mean_motion: mean_motion_rev_per_day,
            revolution_number: 0,
            ephemeris_type: 0,
        }
    }
}
