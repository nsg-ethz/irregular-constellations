use std::f64::consts::PI;
use std::fmt::Debug;

use chrono::NaiveDateTime;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use sgp4::{Elements, Prediction, WGS84};

/// The Earth equatorial radius in `m` (WGS84), kept consistent with SGP4 constants.
pub const EARTH_RADIUS: f64 = WGS84.ae * 1000.0;

/// Get the altitude of the orbit in `m`
///
/// **Warning**: There are many ways we could be defining the "altitude" of an orbit.
/// Here, we mean the semi-major axis minus the earth radius
pub fn get_orbit_altitude(elements: &Elements) -> f64 {
    // First, convert mean motion from [rev/day] to [rad/min]
    let n = elements.mean_motion * 2.0 * PI / (24.0 * 60.0);
    let a_earth_radii = (WGS84.ke / n).powf(2.0 / 3.0);
    // Convert to km
    let semi_majox_axis_km = a_earth_radii * WGS84.ae;
    (semi_majox_axis_km - WGS84.ae) * 1000.0
}

/// A 3D position in Earth-Centered Earth-Fixed (ECEF) coordinates
#[derive(Debug, Clone, Copy)]
pub struct Position {
    /// X coordinate in `m`
    pub x: f64,
    /// Y coordinate in `m`
    pub y: f64,
    /// Z coordinate in `m`
    pub z: f64,
}

impl Position {
    /// Returns the euclidean distance in `m` between `self` and `other`
    pub fn euclidean_distance(&self, other: &Self) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2) + (self.z - other.z).powi(2))
            .sqrt()
    }

    /// Returns the length of the unit vector described by `self` in `m`
    pub fn magnitude(&self) -> f64 {
        ((self.x).powi(2) + (self.y).powi(2) + (self.z).powi(2)).sqrt()
    }
}

impl From<[f64; 3]> for Position {
    fn from(value: [f64; 3]) -> Self {
        Position {
            x: value[0] * 1000.0,
            y: value[1] * 1000.0,
            z: value[2] * 1000.0,
        }
    }
}

impl From<Position> for (f64, f64) {
    /// Converts from ECEF (x, y, z) to geodetic latitude and longitude in `rad`,
    /// assuming a spherical Earth
    fn from(position: Position) -> (f64, f64) {
        let lat = (position.z / position.magnitude()).clamp(-1.0, 1.0).asin();
        let lon = position.y.atan2(position.x);
        (lat, lon)
    }
}

impl Serialize for Position {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Position", 2)?;

        s.serialize_field("cartesian", &(self.x, self.y, self.z))?;

        let (lat, lon): (f64, f64) = (*self).into();
        s.serialize_field("geodetic", &(lat.to_degrees(), lon.to_degrees()))?;

        s.end()
    }
}

/// A single satellite in the constellation
#[derive(Debug, Clone, Serialize)]
pub struct Satellite {
    /// The international designator of this satellite (e.g. `2019-029A`), which is also the key
    /// under which it is listed in the SatCat. Satellites synthesized to fill a hole in an
    /// idealized constellation are instead named `<altitude>-<inclination>-<raan>-<arg_lat>`.
    pub name: String,
    #[serde(skip_serializing)]
    pub elements: Elements,
    /// How many laser terminals — and therefore ISLs — this satellite can support
    pub terminals: Terminals,
    /// The position of this satellite at the constellation's common epoch
    pub position: Position,
    /// Velocity of the satellite in km/s at the constellation's common epoch
    pub velocity: [f64; 3],
    /// Whether this satellite is a real, measured one, or one synthesized to fill a hole while
    /// idealizing a constellation
    pub synthetic: bool,
}

impl Satellite {
    pub fn new(name: String, elements: Elements, time: NaiveDateTime, max_terminals: u32) -> Self {
        let Prediction { position, velocity } = Self::propagate_elements(&elements, time);
        Self {
            name,
            elements,
            // WARN: Conversion from [km] to [m]
            position: position.into(),
            velocity,
            terminals: Terminals::new(max_terminals),
            synthetic: false,
        }
    }

    /// Create a satellite that stands in for one the real constellation is missing
    pub fn synthetic(
        name: String,
        elements: Elements,
        time: NaiveDateTime,
        max_terminals: u32,
    ) -> Self {
        Self {
            synthetic: true,
            ..Self::new(name, elements, time, max_terminals)
        }
    }

    /// Compute the altitude in `m` of this satellite based on its position.
    /// NOTE: Here we assume the earth is a perfect sphere, which is not the case
    pub fn get_altitude(&self) -> f64 {
        self.position.magnitude() - EARTH_RADIUS
    }

    /// Compute the RAAN of this object in `deg` based on its position.
    /// There is a difference between the TLE's RAAN and the one we compute on the fly.
    pub fn get_raan(&self) -> f64 {
        let [hx, hy, _] = self.get_angular_momentum();

        // Node vector  N = ẑ × h = (-h_y, h_x, 0)
        // RAAN = atan2(N_y, N_x) = atan2(h_x, -h_y)
        let mut raan_deg = hx.atan2(-hy).to_degrees();
        if raan_deg < 0.0 {
            raan_deg += 360.0;
        }

        raan_deg
    }

    /// Compute the argument of latitude of this object in `deg` based on its position.
    /// It is defined as the sum of the argument of periapsis and true anomaly.
    pub fn get_argument_of_latitude(&self) -> f64 {
        // Argument of latitude  u = ω + ν  (angle from ascending node to satellite)
        let raan_deg = self.get_raan();
        let sin_raan = raan_deg.to_radians().sin();
        let cos_raan = raan_deg.to_radians().cos();

        let inclination_deg = self.get_inclination();
        let sin_incl = inclination_deg.to_radians().sin();

        let sin_u = self.position.z / (self.position.magnitude() * sin_incl);
        let cos_u =
            (self.position.x * cos_raan + self.position.y * sin_raan) / self.position.magnitude();
        let mut arg_lat_deg = sin_u.atan2(cos_u).to_degrees();
        if arg_lat_deg < 0.0 {
            arg_lat_deg += 360.0;
        }

        arg_lat_deg
    }

    /// Compute the inclination of this object in `deg` based on its position.
    /// There is a difference between the TLE's inclination and the one we compute on the fly.
    fn get_inclination(&self) -> f64 {
        let [hx, hy, hz] = self.get_angular_momentum();
        let h_mag = (hx * hx + hy * hy + hz * hz).sqrt();

        // Inclination  i = arccos(h_z / |h|)
        (hz / h_mag).acos().to_degrees()
    }

    /// Get the angular momentum of this object at its current position
    fn get_angular_momentum(&self) -> [f64; 3] {
        let hx = self.position.y * self.velocity[2] - self.position.z * self.velocity[1];
        let hy = self.position.z * self.velocity[0] - self.position.x * self.velocity[2];
        let hz = self.position.x * self.velocity[1] - self.position.y * self.velocity[0];

        [hx, hy, hz]
    }

    fn propagate_elements(elements: &Elements, time: NaiveDateTime) -> Prediction {
        // Build SGP4 propagator constants from the TLE
        let constants = sgp4::Constants::from_elements(elements).unwrap();
        // Minutes from this satellite's TLE epoch to the common target epoch
        let minutes = elements.datetime_to_minutes_since_epoch(&time).unwrap();
        // SGP4 propagation → position & velocity in TEME (km, km/s)
        constants.propagate(minutes).unwrap()
    }
}

/// The laser terminal budget of a satellite. First generation Starlink satellites have no laser
/// terminals at all; the later generations carry enough for 3 ISLs.
#[derive(Debug, Clone, Serialize)]
pub struct Terminals {
    in_use: u32,
    pub max_available: u32,
}

impl Terminals {
    pub fn new(max_available: u32) -> Self {
        Self {
            in_use: 0,
            max_available,
        }
    }

    /// How many terminals are still available
    pub fn available(&self) -> u32 {
        self.max_available - self.in_use
    }

    /// Claim one terminal for a new ISL
    pub fn claim(&mut self) {
        self.in_use += 1;
        assert!(self.in_use <= self.max_available)
    }
}
