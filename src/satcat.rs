//! # SATCAT Parser Module
//!
//! This module provides structures and parsing utilities for the GCAT (General Catalog
//! of Artificial Space Objects) satcat.tsv file format.
//!
//! Based on the documentation at <https://planet4589.org/space/gcat/web/intro/type.html>
//! and related pages by Jonathan C. McDowell.
//!
//! ## Overview
//!
//! The satcat file contains a comprehensive catalog of space objects including:
//! - Payloads (satellites, spacecraft)
//! - Rocket stages
//! - Debris (components and fragmentation debris)
//! - Suborbital payloads
//!
//! ## Data Format
//!
//! The file is tab-separated (TSV) with the first two lines being headers/comments.

use serde::{Deserialize, Deserializer, Serialize};
use std::io::Read;
use std::str::FromStr;

// ============================================================================
// JCAT IDENTIFIER
// ============================================================================

/// JCAT (Jonathan's Catalog ID) catalog type prefix.
///
/// The JCAT identifier uniquely tags space objects in the General Catalog.
/// The identifier consists of a single prefix letter followed by a 5 or 9 digit
/// sequence number.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/jcat.html>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JcatCatalog {
    /// `A` - Auxiliary catalog (auxcat)
    /// Contains objects not in the standard US SATCAT, such as permanently
    /// attached payloads, pre-1963 objects, and other special cases.
    Auxiliary,

    /// `C` - Complementary catalog (csocat)
    /// Contains complementary space objects.
    Complementary,

    /// `D` - Deep space catalog (deepcat)
    /// Contains objects in deep space or on escape trajectories.
    /// Note: Objects with D designation also have S or A designation for near-Earth phases.
    DeepSpace,

    /// `F` - Failed to orbit catalog (ftocat)
    /// Contains objects from failed orbital launch attempts.
    FailedToOrbit,

    /// `L` - Low altitude catalog (lcat)
    /// Contains low altitude objects.
    LowAltitude,

    /// `R` - Suborbital catalog (rcat)
    /// Contains suborbital objects.
    Suborbital,

    /// `S` - Standard catalog (stdcat)
    /// The main catalog, with sequence numbers corresponding one-to-one with US SATCAT numbers.
    /// For example, S46112 corresponds to SATCAT satellite 46112 (2020-056A).
    Standard,

    /// `T` - Temporary catalog (tmpcat)
    /// Contains temporary entries.
    Temporary,
}

impl JcatCatalog {
    /// Parse a JCAT catalog type from its prefix character.
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'A' => Some(Self::Auxiliary),
            'C' => Some(Self::Complementary),
            'D' => Some(Self::DeepSpace),
            'F' => Some(Self::FailedToOrbit),
            'L' => Some(Self::LowAltitude),
            'R' => Some(Self::Suborbital),
            'S' => Some(Self::Standard),
            'T' => Some(Self::Temporary),
            _ => None,
        }
    }

    /// Get the prefix character for this catalog type.
    pub fn as_char(&self) -> char {
        match self {
            Self::Auxiliary => 'A',
            Self::Complementary => 'C',
            Self::DeepSpace => 'D',
            Self::FailedToOrbit => 'F',
            Self::LowAltitude => 'L',
            Self::Suborbital => 'R',
            Self::Standard => 'S',
            Self::Temporary => 'T',
        }
    }
}

/// A JCAT identifier consisting of a catalog prefix and sequence number.
///
/// Supports both simple JCAT IDs (`S00001`, `A00022`) and extended JCAT IDs
/// with port locations (`S11727  A06`) or markers (`S11668*`).
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/jcat.html>
///
/// ## Extended JCAT Identifier
///
/// The Parent field supports an "extended JCAT ID" which consists of a JCAT ID
/// and a port location separated by one or more spaces. The port location defines
/// a specific part of the parent object where the object is attached.
///
/// Examples of port locations:
/// - Docking ports: `A07305 N` (Harmony nadir CBM)
/// - JEM Exposed Facility locations: `A07559 EFU5`
/// - ISS truss battery ORU locations: `A07476 1B3`
///
/// An asterisk may be appended to indicate the launch designation mismatch flag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JcatId {
    /// The catalog this object belongs to.
    pub catalog: JcatCatalog,
    /// The sequence number within the catalog (5 or 9 digits, stored as u32).
    pub sequence: u32,
    /// Optional port location for extended JCAT IDs (e.g., "A06", "EFU5", "N").
    pub port_location: Option<String>,
    /// Whether an asterisk marker is present (indicates launch designation mismatch).
    pub has_asterisk: bool,
}

impl FromStr for JcatId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() || s == "-" {
            return Err("Empty JCAT ID".to_string());
        }

        // Check for asterisk marker at the end of the base JCAT ID
        // The asterisk appears right after the sequence number, before any port location
        // e.g., "S11668*" or possibly "S11668* A06"

        // Split on whitespace to separate JCAT ID from port location
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.is_empty() {
            return Err("Empty JCAT ID".to_string());
        }

        let base_jcat = parts[0];
        let port_location = if parts.len() > 1 {
            Some(parts[1..].join(" "))
        } else {
            None
        };

        // Parse the base JCAT ID (may have asterisk)
        let has_asterisk = base_jcat.ends_with('*');
        let base_jcat = base_jcat.trim_end_matches('*');

        let mut chars = base_jcat.chars();
        let prefix = chars
            .next()
            .ok_or_else(|| "Empty JCAT string".to_string())?;
        let catalog = JcatCatalog::from_char(prefix)
            .ok_or_else(|| format!("Invalid JCAT prefix: {prefix}"))?;

        let seq_str: String = chars.collect();
        let sequence = seq_str
            .parse::<u32>()
            .map_err(|e| format!("Invalid JCAT sequence number '{seq_str}': {e}"))?;

        Ok(JcatId {
            catalog,
            sequence,
            port_location,
            has_asterisk,
        })
    }
}

impl std::fmt::Display for JcatId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{:05}", self.catalog.as_char(), self.sequence)?;
        if self.has_asterisk {
            write!(f, "*")?;
        }
        if let Some(ref port) = self.port_location {
            write!(f, " {}", port)?;
        }
        Ok(())
    }
}

/// Custom deserializer for JcatId that handles the string format.
fn deserialize_jcat_id<'de, D>(deserializer: D) -> Result<JcatId, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    JcatId::from_str(&s).map_err(serde::de::Error::custom)
}

/// Custom deserializer for optional JcatId (Parent field can be "-").
fn deserialize_optional_jcat_id<'de, D>(deserializer: D) -> Result<Option<JcatId>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let s = s.trim();
    if s.is_empty() || s == "-" {
        return Ok(None);
    }
    JcatId::from_str(s)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// Custom deserializer for optional string fields (treats "-" as None).
fn deserialize_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let s = s.trim();
    if s.is_empty() || s == "-" {
        Ok(None)
    } else {
        Ok(Some(s.to_string()))
    }
}

// ============================================================================
// SATTYPE SCHEME - 12-byte classification string
// ============================================================================
// Reference: https://planet4589.org/space/gcat/web/intro/type.html

/// Byte 1: Coarse Type
///
/// Divides the catalog into payloads, rocket stages, and debris.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-1-coarse-type>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CoarseType {
    /// `P` - Payload (for orbital attempt)
    /// The primary objects of interest: satellites, spacecraft, etc.
    Payload,

    /// `C` - Component
    /// Subsystems of the parent object, usually but not necessarily designed to separate.
    /// Examples: fairings, adapters, yo-yo despin weights.
    Component,

    /// `R` - Launch vehicle stage
    /// Rocket stages (R1 through R5 indicate stage number via Byte 2).
    RocketStage,

    /// `D` - Fragmentation debris
    /// Pieces resulting from object breakup (explosions, collisions, ASAT tests, etc.).
    FragmentationDebris,

    /// `S` - Suborbital payload
    /// Sounding rocket payloads or missile reentry vehicles that don't achieve orbit.
    SuborbitalPayload,

    /// `X` - Deleted catalog entry
    /// Entry that has been deleted (used in auxcat, etc.).
    Deleted,

    /// `Z` - Spurious catalog entry
    /// Was in SATCAT/TLEs but there was no real object.
    Spurious,

    /// Unknown or blank value
    #[default]
    Unknown,
}

impl CoarseType {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'P' => Self::Payload,
            'C' => Self::Component,
            'R' => Self::RocketStage,
            'D' => Self::FragmentationDebris,
            'S' => Self::SuborbitalPayload,
            'X' => Self::Deleted,
            'Z' => Self::Spurious,
            ' ' | '-' => Self::Unknown,
            _ => Self::Unknown,
        }
    }
}

/// Byte 2: Type Modifier
///
/// Modifies the Byte 1 coarse type with additional classification.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-2-type-modifier>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TypeModifier {
    /// `A` - Alias entry (PA)
    /// Special records representing a phase where the satellite is leased temporarily
    /// or jointly owned. Time period overlaps another phase.
    Alias,

    /// `H` - Human spaceflight (PH, SH)
    /// Spaceship with humans aboard at launch.
    HumanSpaceflight,

    /// `P` - Pressurized cabin (PP, CP, SP)
    /// Spaceship with pressurized cabin, but without humans at launch.
    /// Test flights, cargo ship sections, space station modules.
    PressurizedCabin,

    /// `X` - Non-standard (PX, CX)
    /// Not in standard list of satellites/components.
    /// Examples: small calibration satellites, attached payloads, recovery capsules.
    NonStandard,

    /// `1` - Stage 1 (R1)
    Stage1,
    /// `2` - Stage 2 (R2)
    Stage2,
    /// `3` - Stage 3 (R3)
    Stage3,
    /// `4` - Stage 4 (R4)
    Stage4,
    /// `5` - Stage 5 (R5)
    Stage5,

    /// `C` - Cargo placeholder (CC)
    /// For mass accounting of station cargo.
    CargoPlaceholder,

    /// `D` - Deployer (CD)
    /// Separately integrated deployer (e.g., Nanoracks, Spaceflight).
    Deployer,

    /// No modifier (blank or dash)
    #[default]
    None,
}

impl TypeModifier {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'A' => Self::Alias,
            'H' => Self::HumanSpaceflight,
            'P' => Self::PressurizedCabin,
            'X' => Self::NonStandard,
            '1' => Self::Stage1,
            '2' => Self::Stage2,
            '3' => Self::Stage3,
            '4' => Self::Stage4,
            '5' => Self::Stage5,
            'C' => Self::CargoPlaceholder,
            'D' => Self::Deployer,
            ' ' | '-' => Self::None,
            _ => Self::None,
        }
    }
}

/// Byte 3: Attach Flag
///
/// Describes why this object is attached to its parent object.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-3-attach-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AttachFlag {
    /// `A` - Permanently attached component or payload
    PermanentlyAttached,

    /// `F` - Stuck attached by mistake
    /// Object was intended to separate but failed to do so.
    StuckByMistake,

    /// `S` - Expected to separate in future
    ExpectedToSeparate,

    /// `T` - Never flew free but transferred
    /// Hardware transferred from one object to another during spacewalk or by robotic arms.
    Transferred,

    /// `I` - Internal
    /// Hardware that remains internal to one or more host spacecraft.
    /// Examples: EVA spacesuits not used, cubesat dispensers on ISS, cargo mass accounting.
    Internal,

    /// Not attached or blank
    #[default]
    None,
}

impl AttachFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'A' => Self::PermanentlyAttached,
            'F' => Self::StuckByMistake,
            'S' => Self::ExpectedToSeparate,
            'T' => Self::Transferred,
            'I' => Self::Internal,
            ' ' | '-' => Self::None,
            _ => Self::None,
        }
    }
}

/// Byte 4: Subtype Flag
///
/// Categorizes objects more finely than the byte 1-2 specification.
/// Useful for gathering statistics about different kinds of debris.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-4-subtype-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SubtypeFlag {
    /// `A` - Payload adapter, support structures, interfaces (e.g., SYLDA)
    /// Allowed: C
    PayloadAdapter,

    /// `B` - Battery explosion debris
    /// Allowed: D
    BatteryExplosion,

    /// `C` - Passive calibration satellites, test objects or chaff
    /// Allowed: P, C
    CalibrationSatellite,

    /// `D` - Dummy satellite
    /// Allowed: P
    DummySatellite,

    /// `E` - Spacesuit on tethered spacewalk
    /// Allowed: P (PX)
    TetheredSpacesuit,

    /// `F` - Fairings and other covers
    /// Allowed: C
    Fairing,

    /// `G` - General, miscellaneous debris
    /// Allowed: C, D
    GeneralDebris,

    /// `H` - Human spaceflight related
    /// Allowed: P, C
    HumanSpaceflightRelated,

    /// `I` - Impact (accidental collision) debris
    /// Allowed: D
    ImpactDebris,

    /// `J` - Anomalous debris (insulation, soft material, ablated material)
    /// Allowed: C, D
    AnomalousDebris,

    /// `K` - Possible solid motor slag
    /// Allowed: D
    MotorSlag,

    /// `L` - Separated from vehicle after landing (rovers, etc.)
    /// Allowed: P, C
    SurfaceSeparated,

    /// `M` - Jettisoned motor or tank
    /// Allowed: C
    JettisonedMotor,

    /// `N` - Nuclear reactor core or coolant blob
    /// Allowed: C, D
    NuclearRelated,

    /// `O` - Unknown debris released at orbit insertion
    OrbitInsertionDebris,

    /// `P` - Propulsion related, residual-propellant breakup
    /// Allowed: D
    PropulsionDebris,

    /// `Q` - Aerodynamic breakup at low perigee
    /// Allowed: D
    AerodynamicBreakup,

    /// `R` - Reentry vehicle
    /// Allowed: P
    ReentryVehicle,

    /// `S` - Subsatellite or subpayload
    /// Allowed: P
    Subsatellite,

    /// `T` - Ejected section of payload
    /// Allowed: C
    EjectedSection,

    /// `U` - Untethered EVA
    /// Allowed: P (PX)
    UntetheredEva,

    /// `V` - Ejection mechanism (deploy canister, clamp band)
    /// Allowed: C
    EjectionMechanism,

    /// `W` - Weapons test, ASAT debris
    /// Allowed: D
    WeaponsTest,

    /// `X` - Debris of unknown nature
    /// Allowed: C, D
    UnknownDebris,

    /// `Y` - Despin (yo-yo) device
    /// Allowed: C
    DespinDevice,

    /// `Z` - Breakup debris from on-board destruct device
    /// Allowed: D
    DestructDebris,

    /// No subtype (blank or dash)
    #[default]
    None,
}

impl SubtypeFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'A' => Self::PayloadAdapter,
            'B' => Self::BatteryExplosion,
            'C' => Self::CalibrationSatellite,
            'D' => Self::DummySatellite,
            'E' => Self::TetheredSpacesuit,
            'F' => Self::Fairing,
            'G' => Self::GeneralDebris,
            'H' => Self::HumanSpaceflightRelated,
            'I' => Self::ImpactDebris,
            'J' => Self::AnomalousDebris,
            'K' => Self::MotorSlag,
            'L' => Self::SurfaceSeparated,
            'M' => Self::JettisonedMotor,
            'N' => Self::NuclearRelated,
            'O' => Self::OrbitInsertionDebris,
            'P' => Self::PropulsionDebris,
            'Q' => Self::AerodynamicBreakup,
            'R' => Self::ReentryVehicle,
            'S' => Self::Subsatellite,
            'T' => Self::EjectedSection,
            'U' => Self::UntetheredEva,
            'V' => Self::EjectionMechanism,
            'W' => Self::WeaponsTest,
            'X' => Self::UnknownDebris,
            'Y' => Self::DespinDevice,
            'Z' => Self::DestructDebris,
            ' ' | '-' => Self::None,
            _ => Self::None,
        }
    }
}

/// Byte 5: Orbit Flag
///
/// Notes special cases related to the object's trajectory.
/// Blank for normal SATCAT entries except deep space ones.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-5-orbit-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum OrbitFlag {
    /// `D` - Deep Space or escape
    DeepSpace,

    /// `E` - Destroyed in pad explosion
    PadExplosion,

    /// `F` - Failed to reach orbit
    FailedOrbit,

    /// `L` - Active on planet surface during this phase
    PlanetSurface,

    /// `M` - Missing from SATCAT by mistake (EXPRESS, IXV)
    MissingFromSatcat,

    /// `O` - Orbital-Energy but Non-Orbit
    OrbitalEnergyNonOrbit,

    /// `P` - Partial orbit
    /// Reached legit orbit but deorbited after less than 1 revolution.
    PartialOrbit,

    /// `R` - Reentry orbit
    /// Objects that were attached and separated in post-deorbit-burn suborbital trajectory.
    ReentryOrbit,

    /// `S` - Near-Orbit (marginally suborbital)
    NearOrbit,

    /// `T` - Transient orbit
    /// Separated just (perhaps seconds) before deorbit.
    TransientOrbit,

    /// `V` - Escape energy but not deep space
    EscapeEnergy,

    /// `X` - Extra catalog entry for extraterrestrially launched object
    ExtraterrestrialExtra,

    /// `Z` - Launch from extraterrestrial object, recataloged with new D series number
    ExtraterrestrialLaunch,

    /// Normal orbit (blank)
    #[default]
    Normal,
}

impl OrbitFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'D' => Self::DeepSpace,
            'E' => Self::PadExplosion,
            'F' => Self::FailedOrbit,
            'L' => Self::PlanetSurface,
            'M' => Self::MissingFromSatcat,
            'O' => Self::OrbitalEnergyNonOrbit,
            'P' => Self::PartialOrbit,
            'R' => Self::ReentryOrbit,
            'S' => Self::NearOrbit,
            'T' => Self::TransientOrbit,
            'V' => Self::EscapeEnergy,
            'X' => Self::ExtraterrestrialExtra,
            'Z' => Self::ExtraterrestrialLaunch,
            ' ' | '-' => Self::Normal,
            _ => Self::Normal,
        }
    }
}

/// Byte 6: Human Spaceflight / Special Group Flag
///
/// Supports easily identifying all objects in special categories,
/// such as all satellite deployments from ISS.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-6-human-spaceflightspecial-group-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum HumanSpaceflightFlag {
    /// `I` - Station program, general
    StationGeneral,

    /// `C` - Station major non-module component
    StationComponent,

    /// `D` - Station deployable subsatellite
    StationSubsatellite,

    /// `E` - Station EVA related equipment
    StationEva,

    /// `G` - Pseudo-entry used for station generic cargo mass accounting
    StationCargo,

    /// `M` - Station module
    StationModule,

    /// `S` - Space Shuttle program or Shuttle payload or component
    SpaceShuttle,

    /// `T` - Piece of visiting vehicle
    VisitingVehiclePiece,

    /// `U` - Visiting vehicle or rocket stage deployed satellite
    VisitingVehicleDeployed,

    /// `V` - Station visiting vehicle
    StationVisitingVehicle,

    /// Not applicable (blank)
    #[default]
    None,
}

impl HumanSpaceflightFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'I' => Self::StationGeneral,
            'C' => Self::StationComponent,
            'D' => Self::StationSubsatellite,
            'E' => Self::StationEva,
            'G' => Self::StationCargo,
            'M' => Self::StationModule,
            'S' => Self::SpaceShuttle,
            'T' => Self::VisitingVehiclePiece,
            'U' => Self::VisitingVehicleDeployed,
            'V' => Self::StationVisitingVehicle,
            ' ' | '-' => Self::None,
            _ => Self::None,
        }
    }
}

/// Byte 7: UN Registration Flag
///
/// Notes issues with UN registration for non-standard payloads.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-7-un-registration-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum UnRegistrationFlag {
    /// `U` - Is, or should be, UN registered even though not a standard payload
    ShouldBeRegistered,

    /// `X` - Was UN registered but should not have been
    ShouldNotBeRegistered,

    /// Normal (blank) - standard registration rules apply
    #[default]
    Normal,
}

impl UnRegistrationFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'U' => Self::ShouldBeRegistered,
            'X' => Self::ShouldNotBeRegistered,
            ' ' | '-' => Self::Normal,
            _ => Self::Normal,
        }
    }
}

/// Byte 8: Failure Flag / Constellation Status Flag
///
/// Records which rocket stage was to blame in an orbital launch failure,
/// or indicates constellation status (primarily for Starlink).
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-8-failure-flagconstellation-status-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum FailureFlag {
    /// `*` - This object was the one that failed during launch
    FailedObject,

    /// `A` - Satellite ascending: orbit raising to operational orbit
    Ascending,

    /// `D` - Satellite in plane drift orbit
    DriftOrbit,

    /// `F` - Satellite failed early in mission, before reaching operational orbit ("screened")
    EarlyFailure,

    /// `G` - Satellite retired to a graveyard orbit
    GraveyardOrbit,

    /// `L` - Satellite removed far from operational constellation
    RemovedFar,

    /// `M` - Satellite failed in operational orbit, undergoing uncontrolled decay
    UncontrolledDecay,

    /// `O` - Satellite is active in operational orbit
    Operational,

    /// `R` - Satellite active orbit lowering to reentry
    OrbitLowering,

    /// `S` - Satellite was used for special tests outside main constellation
    SpecialTests,

    /// `T` - Satellite removed slightly from operational constellation
    RemovedSlightly,

    /// `U` - Satellite apparently malfunctioning, held in intermediate orbit
    Malfunctioning,

    /// Numeric value (1-9) - Groups objects in a debris cloud for multi-event launches
    DebrisCloud(u8),

    /// Normal (blank)
    #[default]
    Normal,
}

impl FailureFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            '*' => Self::FailedObject,
            'A' => Self::Ascending,
            'D' => Self::DriftOrbit,
            'F' => Self::EarlyFailure,
            'G' => Self::GraveyardOrbit,
            'L' => Self::RemovedFar,
            'M' => Self::UncontrolledDecay,
            'O' => Self::Operational,
            'R' => Self::OrbitLowering,
            'S' => Self::SpecialTests,
            'T' => Self::RemovedSlightly,
            'U' => Self::Malfunctioning,
            '1'..='9' => Self::DebrisCloud(c as u8 - b'0'),
            ' ' | '-' => Self::Normal,
            _ => Self::Normal,
        }
    }
}

/// Byte 9: ID Flag
///
/// Notes problems with the identification of the object.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-9-id-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum IdFlag {
    /// `?` - Association with catalog number is a guess (ID may change)
    GuessedId,

    /// `+` - Starlink: out of service but could still maneuver (at time of FCC filing)
    StarlinkOutOfServiceManeuverable,

    /// `*` - Starlink: out of service and cannot maneuver
    StarlinkOutOfServiceNonManeuverable,

    /// `m` - Multiple objects placeholder for known debris event
    MultipleObjects,

    /// `C` - US government orbital data for this object is secret
    ClassifiedData,

    /// `c` - Older US government data was secret, current data is public
    FormerlyClassified,

    /// `U` - Cargo item on ISS assigned to likely launch but actual launch unknown
    UncertainCargoLaunch,

    /// `D` - Cargo item on ISS with uncertain return date
    UncertainCargoReturn,

    /// `X` - Launch unknown, so launch date and other parameters may be unknown
    UnknownLaunch,

    /// `s` - Disagreement between TLE and SupTLE data
    TleDisagreement,

    /// Normal (blank)
    #[default]
    Normal,
}

impl IdFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            '?' => Self::GuessedId,
            '+' => Self::StarlinkOutOfServiceManeuverable,
            '*' => Self::StarlinkOutOfServiceNonManeuverable,
            'm' => Self::MultipleObjects,
            'C' => Self::ClassifiedData,
            'c' => Self::FormerlyClassified,
            'U' => Self::UncertainCargoLaunch,
            'D' => Self::UncertainCargoReturn,
            'X' => Self::UnknownLaunch,
            's' => Self::TleDisagreement,
            ' ' | '-' => Self::Normal,
            _ => Self::Normal,
        }
    }
}

/// Byte 10: Annotation Flag
///
/// Controls display options in certain software associated with the database.
/// Should generally be ignored by other software.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-10-annotation-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AnnotationFlag {
    /// `r` - Red color for plots
    Red,
    /// `g` - Green color for plots
    Green,
    /// `b` - Blue color for plots
    Blue,
    /// `c` - Cyan color for plots
    Cyan,
    /// `m` - Magenta color for plots
    Magenta,
    /// `y` - Yellow color for plots
    Yellow,
    /// `k` - Black color for plots
    Black,

    /// No annotation (blank)
    #[default]
    None,
}

impl AnnotationFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            'r' => Self::Red,
            'g' => Self::Green,
            'b' => Self::Blue,
            'c' => Self::Cyan,
            'm' => Self::Magenta,
            'y' => Self::Yellow,
            'k' => Self::Black,
            ' ' | '-' => Self::None,
            _ => Self::None,
        }
    }
}

/// Byte 11: Group Control Flag
///
/// Used to include or exclude objects from debris cloud analysis.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html#byte-11-group-control-flag>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum GroupControlFlag {
    /// `+` - Object included in debris cloud analysis
    Included,

    /// `-` - Debris object not counted as part of debris cloud
    Excluded,

    /// Normal (blank)
    #[default]
    Normal,
}

impl GroupControlFlag {
    /// Parse from a single character.
    pub fn from_char(c: char) -> Self {
        match c {
            '+' => Self::Included,
            '-' => Self::Excluded,
            ' ' => Self::Normal,
            _ => Self::Normal,
        }
    }
}

/// The complete 12-byte SatType classification string.
///
/// Each byte provides a different aspect of object classification.
/// Byte 12 is currently unused.
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/type.html>
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct SatType {
    /// Byte 1: Coarse type (payload, rocket, debris)
    pub coarse_type: CoarseType,
    /// Byte 2: Type modifier (stage number, human spaceflight, etc.)
    pub type_modifier: TypeModifier,
    /// Byte 3: Attach flag (why object is attached to parent)
    pub attach_flag: AttachFlag,
    /// Byte 4: Subtype (finer categorization for debris, etc.)
    pub subtype: SubtypeFlag,
    /// Byte 5: Orbit flag (special trajectory cases)
    pub orbit_flag: OrbitFlag,
    /// Byte 6: Human spaceflight / special group flag
    pub human_spaceflight: HumanSpaceflightFlag,
    /// Byte 7: UN registration flag
    pub un_registration: UnRegistrationFlag,
    /// Byte 8: Failure flag / constellation status
    pub failure_flag: FailureFlag,
    /// Byte 9: ID flag (identification problems)
    pub id_flag: IdFlag,
    /// Byte 10: Annotation flag (display options)
    pub annotation: AnnotationFlag,
    /// Byte 11: Group control flag (debris cloud analysis)
    pub group_control: GroupControlFlag,
    // Byte 12: Not yet used
}

impl FromStr for SatType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // The Type field is typically 12 characters, but may be shorter
        // Pad with spaces if needed
        let chars: Vec<char> = s.chars().collect();

        let get_char = |i: usize| -> char { chars.get(i).copied().unwrap_or(' ') };

        Ok(SatType {
            coarse_type: CoarseType::from_char(get_char(0)),
            type_modifier: TypeModifier::from_char(get_char(1)),
            attach_flag: AttachFlag::from_char(get_char(2)),
            subtype: SubtypeFlag::from_char(get_char(3)),
            orbit_flag: OrbitFlag::from_char(get_char(4)),
            human_spaceflight: HumanSpaceflightFlag::from_char(get_char(5)),
            un_registration: UnRegistrationFlag::from_char(get_char(6)),
            failure_flag: FailureFlag::from_char(get_char(7)),
            id_flag: IdFlag::from_char(get_char(8)),
            annotation: AnnotationFlag::from_char(get_char(9)),
            group_control: GroupControlFlag::from_char(get_char(10)),
            // Byte 12 (index 11) is unused
        })
    }
}

/// Custom deserializer for SatType.
fn deserialize_sat_type<'de, D>(deserializer: D) -> Result<SatType, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    SatType::from_str(&s).map_err(serde::de::Error::custom)
}

// ============================================================================
// VAGUE DATE FORMAT
// ============================================================================

/// A date in GCAT's "Vague Date" format.
///
/// Vague dates represent a time range within which an event is believed to lie.
/// The precision of the date varies based on what fields are present.
///
/// Examples:
/// - `"1957 Oct  4"` - Day precision
/// - `"1957 Oct  4 1933"` - Minute precision
/// - `"1957 Oct  4 1933:00"` - Second precision
/// - `"1957"` - Year precision
/// - `"1957 Oct"` - Month precision
///
/// Reference: <https://planet4589.org/space/gcat/web/intro/vague.html>
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VagueDate {
    /// The raw string representation of the date.
    pub raw: String,
    /// Year component (always present for valid dates).
    pub year: Option<i32>,
    /// Month component (1-12), if known.
    pub month: Option<u8>,
    /// Day component (1-31), if known.
    pub day: Option<u8>,
    /// Hour component (0-23), if known.
    pub hour: Option<u8>,
    /// Minute component (0-59), if known.
    pub minute: Option<u8>,
    /// Second component (0-60), if known. 60 is valid for leap seconds.
    pub second: Option<u8>,
    /// Whether a question mark indicates uncertainty.
    pub uncertain: bool,
    /// Whether this is a scheduled (future) date.
    pub scheduled: bool,
}

impl VagueDate {
    /// Create an unknown/empty vague date.
    pub fn unknown() -> Self {
        VagueDate {
            raw: "?".to_string(),
            year: None,
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
            uncertain: true,
            scheduled: false,
        }
    }

    /// Parse a month abbreviation to a month number (1-12).
    fn parse_month(s: &str) -> Option<u8> {
        match s.to_lowercase().as_str() {
            "jan" => Some(1),
            "feb" => Some(2),
            "mar" => Some(3),
            "apr" => Some(4),
            "may" => Some(5),
            "jun" => Some(6),
            "jul" => Some(7),
            "aug" => Some(8),
            "sep" => Some(9),
            "oct" => Some(10),
            "nov" => Some(11),
            "dec" => Some(12),
            _ => None,
        }
    }
}

impl FromStr for VagueDate {
    type Err = String;

    /// Parse a vague date string.
    ///
    /// Format examples:
    /// - `"1957 Oct  4 1933:00"` (full with seconds)
    /// - `"1957 Oct  4 1933"` (minute precision)
    /// - `"1957 Oct  4"` (day precision)
    /// - `"1957 Oct"` (month precision)
    /// - `"1957"` (year precision)
    /// - `"-"` or `"?"` (unknown)
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();

        // Handle special cases
        if s.is_empty() || s == "-" || s == "?" {
            return Ok(VagueDate::unknown());
        }

        let uncertain = s.ends_with('?');
        let scheduled = s.ends_with('s');
        let s = s.trim_end_matches(|c| c == '?' || c == 's');

        // Parse year (first token, 4 digits)
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(VagueDate::unknown());
        }

        let year = parts[0]
            .parse::<i32>()
            .map_err(|_| format!("Invalid year: {}", parts[0]))?;

        let mut result = VagueDate {
            raw: s.to_string(),
            year: Some(year),
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
            uncertain,
            scheduled,
        };

        // Parse month if present
        if parts.len() > 1 {
            result.month = VagueDate::parse_month(parts[1]);
        }

        // Parse day if present
        if parts.len() > 2 {
            result.day = parts[2].parse::<u8>().ok();
        }

        // Parse time if present (format: HHMM or HHMM:SS)
        if parts.len() > 3 {
            let time_str = parts[3];
            if time_str.contains(':') {
                // HHMM:SS format
                let time_parts: Vec<&str> = time_str.split(':').collect();
                if time_parts[0].len() >= 4 {
                    result.hour = time_parts[0][0..2].parse::<u8>().ok();
                    result.minute = time_parts[0][2..4].parse::<u8>().ok();
                }
                if time_parts.len() > 1 {
                    result.second = time_parts[1].parse::<u8>().ok();
                }
            } else if time_str.len() >= 4 {
                // HHMM format
                result.hour = time_str[0..2].parse::<u8>().ok();
                result.minute = time_str[2..4].parse::<u8>().ok();
            }
        }

        Ok(result)
    }
}

impl std::fmt::Display for VagueDate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl PartialOrd for VagueDate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VagueDate {
    /// Compare two vague dates chronologically.
    ///
    /// Fields are compared from most to least significant: year → month → day →
    /// hour → minute → second.
    ///
    /// **`None` at any level sorts before a known value.** This means a date known
    /// only to the year (e.g. `"1957"`) sorts before one known to the month
    /// (`"1957 Oct"`) within the same year, reflecting that the year-only date
    /// could represent any time in that year, including before October.
    ///
    /// **When all known fields are equal, an uncertain date (`?`) sorts before a
    /// certain one.** Per the spec, `?` widens the range one unit in each direction,
    /// so the uncertain date's range starts one unit earlier.
    ///
    /// Unknown dates (no year at all) sort before everything else.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;

        macro_rules! cmp_opt_field {
            ($a:expr, $b:expr) => {
                match ($a, $b) {
                    (None, None) => {}
                    // None sorts before Some (coarser precision = earlier in range)
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some(a), Some(b)) => match a.cmp(&b) {
                        Ordering::Equal => {}
                        ord => return ord,
                    },
                }
            };
        }

        cmp_opt_field!(self.year, other.year);
        cmp_opt_field!(self.month, other.month);
        cmp_opt_field!(self.day, other.day);
        cmp_opt_field!(self.hour, other.hour);
        cmp_opt_field!(self.minute, other.minute);
        cmp_opt_field!(self.second, other.second);

        // All known fields are equal. Uncertain dates sort before certain ones
        // because their range extends one unit earlier. In Rust, false < true,
        // so flipping the operands gives us: uncertain (true) < certain (false).
        other.uncertain.cmp(&self.uncertain)
    }
}

impl VagueDate {
    /// Returns the **lower bound** of the date range as a `(year, month, day, hour, minute,
    /// second)` tuple.
    ///
    /// For an uncertain date the bound is shifted one unit *backward* at the
    /// finest known precision. For example:
    /// - `"1957 Oct  4?"` → lower `(1957, 10, 3, 0, 0, 0)` (one day before)
    /// - `"1957 Oct?"` → lower `(1957, 9, 1, 0, 0, 0)` (one month before)
    /// - `"1957?"` → lower `(1956, 1, 1, 0, 0, 0)` (one year before)
    ///
    /// For `None` fields, the start of the period is assumed (month 1, day 1,
    /// time 00:00:00).
    ///
    /// **Note:** Calendar overflow is intentional and not normalized. Month 0
    /// means "before January" and can still be compared meaningfully as an
    /// integer tuple.
    ///
    /// Returns `(i32::MIN, 0, 0, 0, 0, 0)` for completely unknown dates.
    pub fn lower_bound(&self) -> (i32, i32, i32, i32, i32, i32) {
        let Some(year) = self.year else {
            return (i32::MIN, 0, 0, 0, 0, 0);
        };
        let unc = if self.uncertain { 1i32 } else { 0i32 };

        let Some(m) = self.month else {
            // Year precision: shift back one year if uncertain
            return (year - unc, 1, 1, 0, 0, 0);
        };
        let Some(d) = self.day else {
            // Month precision: shift back one month if uncertain
            return (year, m as i32 - unc, 1, 0, 0, 0);
        };
        let Some(h) = self.hour else {
            // Day precision: shift back one day if uncertain
            return (year, m as i32, d as i32 - unc, 0, 0, 0);
        };
        let Some(min) = self.minute else {
            // Hour precision: shift back one hour if uncertain
            return (year, m as i32, d as i32, h as i32 - unc, 0, 0);
        };
        let Some(sec) = self.second else {
            // Minute precision: shift back one minute if uncertain
            return (year, m as i32, d as i32, h as i32, min as i32 - unc, 0);
        };
        // Second precision: shift back one second if uncertain
        (
            year,
            m as i32,
            d as i32,
            h as i32,
            min as i32,
            sec as i32 - unc,
        )
    }

    /// Returns the **upper bound** (exclusive) of the date range as a
    /// `(year, month, day, hour, minute, second)` tuple.
    ///
    /// For an uncertain date the bound is shifted one unit *forward* at the
    /// finest known precision, in addition to the one unit already added by
    /// normal range widening. For example:
    /// - `"1957 Oct  4"` → upper `(1957, 10, 5, 0, 0, 0)` (next day)
    /// - `"1957 Oct  4?"` → upper `(1957, 10, 6, 0, 0, 0)` (two days after)
    /// - `"1957 Oct"` → upper `(1957, 11, 1, 0, 0, 0)` (next month)
    /// - `"1957"` → upper `(1958, 1, 1, 0, 0, 0)` (next year)
    ///
    /// **Note:** Calendar overflow is intentional and not normalized. Month 13
    /// means "after December" and can still be compared meaningfully as an
    /// integer tuple.
    ///
    /// Returns `(i32::MAX, 13, 32, 24, 60, 61)` for completely unknown dates.
    pub fn upper_bound(&self) -> (i32, i32, i32, i32, i32, i32) {
        let Some(year) = self.year else {
            return (i32::MAX, 13, 32, 24, 60, 61);
        };
        // For uncertain, the range is 3× wide: shift *both* lower and upper by 1.
        // Upper bound of the period already adds 1 unit, so uncertain adds 1 more.
        let unc = if self.uncertain { 1i32 } else { 0i32 };

        let Some(m) = self.month else {
            return (year + 1 + unc, 1, 1, 0, 0, 0);
        };
        let Some(d) = self.day else {
            return (year, m as i32 + 1 + unc, 1, 0, 0, 0);
        };
        let Some(h) = self.hour else {
            return (year, m as i32, d as i32 + 1 + unc, 0, 0, 0);
        };
        let Some(min) = self.minute else {
            return (year, m as i32, d as i32, h as i32 + 1 + unc, 0, 0);
        };
        let Some(sec) = self.second else {
            return (year, m as i32, d as i32, h as i32, min as i32 + 1 + unc, 0);
        };
        (
            year,
            m as i32,
            d as i32,
            h as i32,
            min as i32,
            sec as i32 + 1 + unc,
        )
    }

    /// Returns `true` if the ranges of `self` and `other` overlap.
    ///
    /// Two vague dates overlap if one's lower bound is before the other's upper
    /// bound and vice versa.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.lower_bound() < other.upper_bound() && other.lower_bound() < self.upper_bound()
    }
}

/// Custom deserializer for VagueDate.
fn deserialize_vague_date<'de, D>(deserializer: D) -> Result<VagueDate, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    VagueDate::from_str(&s).map_err(serde::de::Error::custom)
}

// ============================================================================
// MAIN SATCAT ENTRY STRUCTURE
// ============================================================================

/// A single entry from the GCAT satcat catalog.
///
/// This structure represents the first 9 columns of the satcat.tsv file:
/// JCAT, Satcat, Launch_Tag, Piece, Type, Name, PLName, LDate, Parent
///
/// Reference: <https://planet4589.org/space/gcat/>
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatcatEntry {
    /// JCAT identifier (Jonathan's Catalog ID).
    /// Uniquely identifies this object across all GCAT catalogs.
    /// Format: prefix letter (A/C/D/F/L/R/S/T) + 5-9 digit sequence number.
    ///
    /// Example: `S00001` (first object in standard catalog)
    #[serde(rename = "JCAT", deserialize_with = "deserialize_jcat_id")]
    pub jcat: JcatId,

    /// US Space Force SATCAT catalog number.
    /// For standard catalog entries (JCAT prefix 'S'), this equals the JCAT sequence number.
    /// May be missing for auxiliary catalog entries.
    ///
    /// Example: `00001`
    #[serde(rename = "Satcat")]
    pub satcat: String,

    /// Launch designation tag.
    /// Usually the COSPAR designation (year-number) or Harvard designation (year + greek letter).
    ///
    /// Examples:
    /// - `"1957 ALP"` (Harvard: 1957 Alpha)
    /// - `"2020-056"` (COSPAR: 56th launch of 2020)
    /// - `"2020-F03"` (JSR: 3rd failed orbital attempt of 2020)
    #[serde(rename = "Launch_Tag")]
    pub launch_tag: String,

    /// Piece designation within the launch.
    /// Combines launch designation with piece letter (A-Z, then AA-ZZ, etc.).
    ///
    /// Examples:
    /// - `"1957 ALP 1"` (Harvard format, piece 1)
    /// - `"2020-056A"` (COSPAR format, piece A)
    #[serde(rename = "Piece")]
    pub piece: String,

    /// 12-byte SatType classification string.
    /// Each byte has a specific meaning (coarse type, modifier, attach flag, etc.).
    ///
    /// Example: `"P           "` (standard payload)
    /// Example: `"R2          "` (stage 2 rocket)
    #[serde(rename = "Type", deserialize_with = "deserialize_sat_type")]
    pub sat_type: SatType,

    /// Current or most recent name of the object.
    ///
    /// Example: `"8K71PS No. M1-10 Stage 2"`
    #[serde(rename = "Name")]
    pub name: String,

    /// Payload name or original project name.
    ///
    /// Example: `"8K71A M1-10 (M1-1PS)"`
    #[serde(rename = "PLName")]
    pub pl_name: String,

    /// Launch date in Vague Date format.
    /// Precision varies from seconds to years.
    ///
    /// Example: `"1957 Oct  4"` (day precision)
    #[serde(rename = "LDate", deserialize_with = "deserialize_vague_date")]
    pub launch_date: VagueDate,

    /// Parent object JCAT identifier.
    /// The object from which this one separated or to which it is attached.
    /// `-` indicates no parent (e.g., for launch vehicle stages).
    ///
    /// Example: `"S00001"` or `"-"`
    #[serde(rename = "Parent", deserialize_with = "deserialize_optional_jcat_id")]
    pub parent: Option<JcatId>,

    /// Spacecraft bus (chassis) type.
    /// The hardware platform or bus design used for the satellite.
    /// Many satellites share common bus designs from manufacturers.
    ///
    /// Examples:
    /// - `"Starlink V2M"` - SpaceX Starlink version 2 mini
    /// - `"A2100"` - Lockheed Martin A2100 bus
    /// - `"SSL-1300"` - Space Systems Loral 1300 bus
    /// - `"Vostok"` - Soviet Vostok spacecraft bus
    ///
    /// May be `-` or empty for debris, rocket stages, or unknown bus types.
    #[serde(rename = "Bus", deserialize_with = "deserialize_optional_string")]
    pub bus: Option<String>,
}

impl SatcatEntry {
    /// This function gets the inter-satellite link count for a specific starlink satellite
    ///
    /// Why? There is no real way to identify which satellites are V1.0 and which are V1.5 from our current data sources
    pub fn get_isl_count(&self) -> u32 {
        // TODO: make this robust to other constellations as well
        match self.bus.as_ref().unwrap().trim() {
            // All V2 satellites, no matter the flavour, have 3 ISLs
            "Starlink V2M" | "Starlink V2MD" | "Starlink V2MO" => 3,
            // V1.0 satellites had no ISLs and were launched until September of 2021
            "Starlink" => {
                // Just a comment: Alternatively we could also look at the groups that each satellite belongs to
                //                 (according to https://planet4589.org/space/con/star/stats.html)
                if self.launch_date > VagueDate::from_str("2021 Sep").unwrap() {
                    3
                } else {
                    0
                }
            }
            _ => 0,
        }
    }
}

// ============================================================================
// PARSING FUNCTIONS
// ============================================================================

/// Parse satcat data from any reader into a vector of SatcatEntry.
///
/// The data should be tab-separated with the header line starting with '#JCAT'.
/// Comment lines (other than the header) also start with '#'.
///
/// # Arguments
/// * `reader` - Any type implementing `std::io::Read`
///
/// # Returns
/// A Result containing a vector of parsed entries or an error.
pub fn parse_satcat_reader<R: std::io::Read>(
    reader: R,
) -> Result<Vec<SatcatEntry>, Box<dyn std::error::Error>> {
    use std::io::{BufRead, BufReader};

    let mut buf_reader = BufReader::new(reader);

    // Read the first line (header) - it starts with '#' but we need to parse it
    let mut header_line = String::new();
    buf_reader.read_line(&mut header_line)?;

    // Strip the leading '#' from the header if present
    let header_line = header_line.trim_start_matches('#');

    // Skip any comment lines (lines starting with '#' that aren't data)
    // The second line is typically "# Updated ..." which we want to skip
    let mut peek_line = String::new();
    loop {
        peek_line.clear();
        let bytes_read = buf_reader.read_line(&mut peek_line)?;
        if bytes_read == 0 {
            // EOF
            return Ok(Vec::new());
        }
        if !peek_line.starts_with('#') {
            // This is a data line, we need to process it
            break;
        }
        // Otherwise it's a comment, continue to next line
    }

    // Create a new reader that includes the header and remaining data
    // We need to chain: header + first data line + rest of file
    let combined =
        std::io::Cursor::new(format!("{}\n{}", header_line.trim(), peek_line)).chain(buf_reader);

    let mut csv_reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .has_headers(true)
        .flexible(true)
        .from_reader(combined);

    let mut entries = Vec::new();

    for result in csv_reader.deserialize() {
        match result {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                // Log error but continue parsing
                log::warn!("Failed to parse satcat entry: {}", e);
            }
        }
    }

    Ok(entries)
}

/// Parse a satcat.tsv file into a vector of SatcatEntry.
///
/// The file should be tab-separated with the first two lines being comments/headers.
///
/// # Arguments
/// * `path` - Path to the satcat.tsv file
///
/// # Returns
/// A Result containing a vector of parsed entries or an error.
pub fn parse_satcat_file(
    path: &std::path::Path,
) -> Result<Vec<SatcatEntry>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    parse_satcat_reader(file)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jcat_parsing() {
        // Simple JCAT ID
        let jcat: JcatId = "S00001".parse().unwrap();
        assert_eq!(jcat.catalog, JcatCatalog::Standard);
        assert_eq!(jcat.sequence, 1);
        assert_eq!(jcat.port_location, None);
        assert!(!jcat.has_asterisk);

        let jcat: JcatId = "A00022".parse().unwrap();
        assert_eq!(jcat.catalog, JcatCatalog::Auxiliary);
        assert_eq!(jcat.sequence, 22);
        assert_eq!(jcat.port_location, None);
        assert!(!jcat.has_asterisk);

        // Extended JCAT ID with port location
        let jcat: JcatId = "S11727  A06".parse().unwrap();
        assert_eq!(jcat.catalog, JcatCatalog::Standard);
        assert_eq!(jcat.sequence, 11727);
        assert_eq!(jcat.port_location, Some("A06".to_string()));
        assert!(!jcat.has_asterisk);

        // JCAT ID with asterisk marker
        let jcat: JcatId = "S11668*".parse().unwrap();
        assert_eq!(jcat.catalog, JcatCatalog::Standard);
        assert_eq!(jcat.sequence, 11668);
        assert_eq!(jcat.port_location, None);
        assert!(jcat.has_asterisk);

        // Extended JCAT ID with complex port location
        let jcat: JcatId = "S11079  SD2.3".parse().unwrap();
        assert_eq!(jcat.catalog, JcatCatalog::Standard);
        assert_eq!(jcat.sequence, 11079);
        assert_eq!(jcat.port_location, Some("SD2.3".to_string()));

        // Port location with question mark
        let jcat: JcatId = "S11079  EQ3?".parse().unwrap();
        assert_eq!(jcat.sequence, 11079);
        assert_eq!(jcat.port_location, Some("EQ3?".to_string()));

        // Port location with alphanumeric
        let jcat: JcatId = "S10765  6C".parse().unwrap();
        assert_eq!(jcat.sequence, 10765);
        assert_eq!(jcat.port_location, Some("6C".to_string()));
    }

    #[test]
    fn test_sat_type_parsing() {
        // Standard payload
        let sat_type: SatType = "P           ".parse().unwrap();
        assert_eq!(sat_type.coarse_type, CoarseType::Payload);
        assert_eq!(sat_type.type_modifier, TypeModifier::None);

        // Rocket stage 2
        let sat_type: SatType = "R2          ".parse().unwrap();
        assert_eq!(sat_type.coarse_type, CoarseType::RocketStage);
        assert_eq!(sat_type.type_modifier, TypeModifier::Stage2);

        // Payload with alias
        let sat_type: SatType = "PA          ".parse().unwrap();
        assert_eq!(sat_type.coarse_type, CoarseType::Payload);
        assert_eq!(sat_type.type_modifier, TypeModifier::Alias);

        // Component with fairing subtype
        let sat_type: SatType = "C  F        ".parse().unwrap();
        assert_eq!(sat_type.coarse_type, CoarseType::Component);
        assert_eq!(sat_type.subtype, SubtypeFlag::Fairing);
    }

    #[test]
    fn test_vague_date_parsing() {
        let date: VagueDate = "1957 Oct  4".parse().unwrap();
        assert_eq!(date.year, Some(1957));
        assert_eq!(date.month, Some(10));
        assert_eq!(date.day, Some(4));
        assert!(!date.uncertain);

        let date: VagueDate = "1957 Oct  4 1933".parse().unwrap();
        assert_eq!(date.hour, Some(19));
        assert_eq!(date.minute, Some(33));

        let date: VagueDate = "1957 Oct  4 1933:00".parse().unwrap();
        assert_eq!(date.second, Some(0));

        let date: VagueDate = "1957?".parse().unwrap();
        assert_eq!(date.year, Some(1957));
        assert!(date.uncertain);
    }

    #[test]
    fn test_vague_date_ordering() {
        // Basic chronological ordering
        let a: VagueDate = "1957 Oct  4".parse().unwrap();
        let b: VagueDate = "1958 Jan  1".parse().unwrap();
        assert!(a < b);

        // Same year, different month
        let a: VagueDate = "1957 Sep".parse().unwrap();
        let b: VagueDate = "1957 Oct".parse().unwrap();
        assert!(a < b);

        // Year-only sorts before month-specific within same year
        // ("1957" could be any time in 1957, including before October)
        let year_only: VagueDate = "1957".parse().unwrap();
        let with_month: VagueDate = "1957 Oct".parse().unwrap();
        assert!(year_only < with_month);

        // Day-only (month known) sorts before time-specific
        let day_only: VagueDate = "1957 Oct  4".parse().unwrap();
        let with_time: VagueDate = "1957 Oct  4 1933".parse().unwrap();
        assert!(day_only < with_time);

        // Uncertain sorts before certain when nominal values are equal
        let certain: VagueDate = "1957 Oct  4".parse().unwrap();
        let uncertain: VagueDate = "1957 Oct  4?".parse().unwrap();
        assert!(uncertain < certain);

        // Unknown date sorts before everything
        let unknown = VagueDate::unknown();
        assert!(unknown < "1957".parse::<VagueDate>().unwrap());

        // A specific day is after a month-only date within the same month:
        // "2021 Jan" has no day → None < Some(20), so "2021 Jan" < "2021 Jan 20"
        let month_only: VagueDate = "2021 Jan".parse().unwrap();
        let with_day: VagueDate = "2021 Jan 20".parse().unwrap();
        assert!(with_day > month_only);
    }

    #[test]
    fn test_vague_date_bounds() {
        // Certain day-precision date
        let d: VagueDate = "1957 Oct  4".parse().unwrap();
        assert_eq!(d.lower_bound(), (1957, 10, 4, 0, 0, 0));
        assert_eq!(d.upper_bound(), (1957, 10, 5, 0, 0, 0));

        // Uncertain day-precision: range is 3 days wide
        let d: VagueDate = "1957 Oct  4?".parse().unwrap();
        assert_eq!(d.lower_bound(), (1957, 10, 3, 0, 0, 0)); // one day before
        assert_eq!(d.upper_bound(), (1957, 10, 6, 0, 0, 0)); // two days after

        // Year-only
        let d: VagueDate = "1957".parse().unwrap();
        assert_eq!(d.lower_bound(), (1957, 1, 1, 0, 0, 0));
        assert_eq!(d.upper_bound(), (1958, 1, 1, 0, 0, 0));

        // Uncertain year: range is 3 years wide
        let d: VagueDate = "1957?".parse().unwrap();
        assert_eq!(d.lower_bound(), (1956, 1, 1, 0, 0, 0));
        assert_eq!(d.upper_bound(), (1959, 1, 1, 0, 0, 0));

        // Second-precision
        let d: VagueDate = "1957 Oct  4 1933:00".parse().unwrap();
        assert_eq!(d.lower_bound(), (1957, 10, 4, 19, 33, 0));
        assert_eq!(d.upper_bound(), (1957, 10, 4, 19, 33, 1));
    }

    #[test]
    fn test_vague_date_overlaps() {
        // A day within a month overlaps with that month
        let a: VagueDate = "1957 Oct  4".parse().unwrap();
        let b: VagueDate = "1957 Oct".parse().unwrap();
        assert!(a.overlaps(&b));

        // Dates in different years do not overlap
        let a: VagueDate = "1957".parse().unwrap();
        let b: VagueDate = "1958".parse().unwrap();
        assert!(!a.overlaps(&b));

        // "Oct 4?" covers [Oct 3, Oct 6) exclusively. "Oct 5" covers [Oct 5, Oct 6).
        // They share Oct 5, so they overlap.
        let a: VagueDate = "1957 Oct  4?".parse().unwrap(); // [Oct 3, Oct 6)
        let b: VagueDate = "1957 Oct  5".parse().unwrap(); // [Oct 5, Oct 6)
        assert!(a.overlaps(&b));

        // "Oct 4?" upper bound is Oct 6 (exclusive). "Oct 6" lower bound is Oct 6.
        // Adjacent ranges share only the boundary point, which is not included.
        let a: VagueDate = "1957 Oct  4?".parse().unwrap(); // [Oct 3, Oct 6)
        let b: VagueDate = "1957 Oct  6".parse().unwrap(); // [Oct 6, Oct 7)
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn test_coarse_type_from_char() {
        assert_eq!(CoarseType::from_char('P'), CoarseType::Payload);
        assert_eq!(CoarseType::from_char('R'), CoarseType::RocketStage);
        assert_eq!(CoarseType::from_char('C'), CoarseType::Component);
        assert_eq!(CoarseType::from_char('D'), CoarseType::FragmentationDebris);
        assert_eq!(CoarseType::from_char(' '), CoarseType::Unknown);
    }
}
