//! The Mullein: her tank, ECM pods, cargo and where she is.
//!
//! Like the colony's modules, the ship keeps its content definition in the
//! save, so a balance change reaches only new games.

use nexa_types::StationId;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Shipped ship content, compiled in so loading needs no file I/O.
pub const MULLEIN: &str = include_str!("../../../content/mullein.ron");

/// Cargo integrity is a whole percent, 0..=MAX_INTEGRITY_PCT.
pub const MAX_INTEGRITY_PCT: u8 = 100;

/// One crate in the hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OreCrate {
    pub id: String,
    /// What's in it, e.g. "raw silicon".
    pub cargo: String,
    pub mass_kg: u32,
    pub integrity_pct: u8,
}

/// Where the ship flies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteDef {
    pub destination: StationId,
    /// Display name, e.g. "Scrap Barge 77".
    pub name: String,
    /// Nominal Sols each way through The Slip.
    pub transit_sols: u32,
}

/// The ship content file, `content/mullein.ron`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipDef {
    pub name: String,
    pub tank_l: u32,
    pub start_fuel_l: u32,
    /// Burned at launch, for each leg.
    pub leg_burn_l: u32,
    /// One Overclock Burn at a hazard terminal.
    pub overclock_burn_l: u32,
    /// Countermeasure pods aboard at landfall.
    pub ecm_pods: u8,
    /// The colony's credits at landfall.
    pub starting_credits: u32,
    pub route: RouteDef,
    /// The outbound load.
    pub manifest: Vec<OreCrate>,
}

/// Why ship content, or a saved ship, was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShipError {
    Parse(String),
    EmptyTank,
    /// Fuel above the tank's capacity.
    Overfull {
        fuel_l: u32,
        tank_l: u32,
    },
    /// A leg or an Overclock Burn costs nothing or more than a full tank.
    BurnOutOfRange {
        field: &'static str,
        liters: u32,
    },
    NoTransit,
    EmptyManifest,
    DuplicateCrate(String),
    WeightlessCrate(String),
    IntegrityTooHigh(String),
    /// A saved ship has more pods than she can carry.
    TooManyPods(u8),
    /// The hold's total mass doesn't fit a whole-kg `u32`.
    HoldTooHeavy,
}

impl std::fmt::Display for ShipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShipError::Parse(e) => write!(f, "ship content doesn't parse: {e}"),
            ShipError::EmptyTank => write!(f, "the tank holds nothing"),
            ShipError::Overfull { fuel_l, tank_l } => {
                write!(f, "{fuel_l} L of fuel in a {tank_l} L tank")
            }
            ShipError::BurnOutOfRange { field, liters } => {
                write!(f, "{field} is {liters} L, expected 1 L up to a full tank")
            }
            ShipError::NoTransit => write!(f, "the route takes no Sols"),
            ShipError::EmptyManifest => write!(f, "the manifest is empty"),
            ShipError::DuplicateCrate(id) => write!(f, "crate {id} is listed twice"),
            ShipError::WeightlessCrate(id) => write!(f, "crate {id} has no mass"),
            ShipError::IntegrityTooHigh(id) => {
                write!(f, "crate {id} integrity is above {MAX_INTEGRITY_PCT}%")
            }
            ShipError::TooManyPods(n) => write!(f, "{n} ECM pods is more than she carries"),
            ShipError::HoldTooHeavy => write!(f, "the hold's total mass is out of range"),
        }
    }
}

impl std::error::Error for ShipError {}

fn check_crates(crates: &[OreCrate]) -> Result<(), ShipError> {
    for (i, c) in crates.iter().enumerate() {
        if crates[..i].iter().any(|other| other.id == c.id) {
            return Err(ShipError::DuplicateCrate(c.id.clone()));
        }
        if c.mass_kg == 0 {
            return Err(ShipError::WeightlessCrate(c.id.clone()));
        }
        if c.integrity_pct > MAX_INTEGRITY_PCT {
            return Err(ShipError::IntegrityTooHigh(c.id.clone()));
        }
    }
    // So `Ship::cargo_mass_kg` can't overflow.
    crates
        .iter()
        .try_fold(0u32, |sum, c| sum.checked_add(c.mass_kg))
        .ok_or(ShipError::HoldTooHeavy)?;
    Ok(())
}

impl ShipDef {
    pub fn from_ron(text: &str) -> Result<Self, ShipError> {
        let def: Self = ron::from_str(text).map_err(|e| ShipError::Parse(e.to_string()))?;
        def.check()?;
        Ok(def)
    }

    /// Parsed once; checked by this module's tests, so failing here is a build bug.
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<ShipDef> = OnceLock::new();
        SHIPPED.get_or_init(|| Self::from_ron(MULLEIN).expect("ship content is valid"))
    }

    pub fn check(&self) -> Result<(), ShipError> {
        if self.tank_l == 0 {
            return Err(ShipError::EmptyTank);
        }
        if self.start_fuel_l > self.tank_l {
            return Err(ShipError::Overfull {
                fuel_l: self.start_fuel_l,
                tank_l: self.tank_l,
            });
        }
        for (field, liters) in [
            ("leg_burn_l", self.leg_burn_l),
            ("overclock_burn_l", self.overclock_burn_l),
        ] {
            if liters == 0 || liters > self.tank_l {
                return Err(ShipError::BurnOutOfRange { field, liters });
            }
        }
        if self.route.transit_sols == 0 {
            return Err(ShipError::NoTransit);
        }
        if self.manifest.is_empty() {
            return Err(ShipError::EmptyManifest);
        }
        check_crates(&self.manifest)
    }
}

/// Where the ship is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShipLocation {
    /// On the colony's landing pad.
    Pad,
    InSlip,
    /// Arrived and holding over a station (milestone 5 ends here).
    Holding(StationId),
}

/// The ship as she stands. Changed only by the flight rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ship {
    def: ShipDef,
    fuel_l: u32,
    pods: u8,
    cargo: Vec<OreCrate>,
    location: ShipLocation,
}

impl Ship {
    /// Fuelled, loaded and on the pad.
    pub fn new(def: ShipDef) -> Self {
        Self {
            fuel_l: def.start_fuel_l,
            pods: def.ecm_pods,
            cargo: def.manifest.clone(),
            location: ShipLocation::Pad,
            def,
        }
    }

    pub fn def(&self) -> &ShipDef {
        &self.def
    }

    pub fn fuel_l(&self) -> u32 {
        self.fuel_l
    }

    pub fn pods(&self) -> u8 {
        self.pods
    }

    pub fn cargo(&self) -> &[OreCrate] {
        &self.cargo
    }

    pub fn location(&self) -> &ShipLocation {
        &self.location
    }

    pub fn cargo_mass_kg(&self) -> u32 {
        self.cargo.iter().map(|c| c.mass_kg).sum()
    }

    /// Mass-weighted integrity of the whole hold; 0 with an empty hold.
    pub fn cargo_integrity_pct(&self) -> u8 {
        let mass = u64::from(self.cargo_mass_kg());
        if mass == 0 {
            return 0;
        }
        let weighted: u64 = self
            .cargo
            .iter()
            .map(|c| u64::from(c.mass_kg) * u64::from(c.integrity_pct))
            .sum();
        (weighted / mass) as u8
    }

    /// Burn up to `liters`; returns what was burned.
    pub(crate) fn burn(&mut self, liters: u32) -> u32 {
        let burned = liters.min(self.fuel_l);
        self.fuel_l -= burned;
        burned
    }

    /// Spend up to `n` pods; returns how many were spent.
    pub(crate) fn spend_pods(&mut self, n: u8) -> u8 {
        let spent = n.min(self.pods);
        self.pods -= spent;
        spent
    }

    /// Jettison up to `n` crates from the end of the hold; returns their mass.
    pub(crate) fn dump_crates(&mut self, n: u32) -> (u32, u32) {
        let keep = self.cargo.len().saturating_sub(n as usize);
        let dumped: Vec<OreCrate> = self.cargo.drain(keep..).collect();
        (dumped.len() as u32, dumped.iter().map(|c| c.mass_kg).sum())
    }

    /// Every crate loses `pct` points of integrity.
    pub(crate) fn damage_cargo(&mut self, pct: u8) {
        for c in &mut self.cargo {
            c.integrity_pct = c.integrity_pct.saturating_sub(pct);
        }
    }

    pub(crate) fn set_location(&mut self, location: ShipLocation) {
        self.location = location;
    }

    /// What a loaded save must satisfy.
    pub fn check(&self) -> Result<(), ShipError> {
        self.def.check()?;
        if self.fuel_l > self.def.tank_l {
            return Err(ShipError::Overfull {
                fuel_l: self.fuel_l,
                tank_l: self.def.tank_l,
            });
        }
        if self.pods > self.def.ecm_pods {
            return Err(ShipError::TooManyPods(self.pods));
        }
        check_crates(&self.cargo)
    }
}

impl Default for Ship {
    /// The shipped Mullein, fuelled and loaded on the pad.
    fn default() -> Self {
        Self::new(ShipDef::shipped().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> ShipDef {
        ShipDef::from_ron(MULLEIN).unwrap()
    }

    #[test]
    fn shipped_mullein_matches_the_fuel_ledger() {
        let def = shipped();
        assert_eq!(def.tank_l, 1000);
        assert_eq!(def.start_fuel_l, 1000);
        assert_eq!(def.leg_burn_l, 350);
        assert_eq!(def.overclock_burn_l, 150);
        assert_eq!(def.route.transit_sols, 5);
        assert_eq!(def.route.destination, StationId::new("barge_77"));
        let ship = Ship::new(def);
        assert_eq!(ship.location(), &ShipLocation::Pad);
        assert_eq!(ship.cargo_mass_kg(), 10_000);
        assert_eq!(ship.cargo_integrity_pct(), 100);
    }

    #[test]
    fn bad_ship_content_is_refused() {
        type Bend = fn(&mut ShipDef);
        let cases: [(Bend, ShipError); 8] = [
            (|d| d.tank_l = 0, ShipError::EmptyTank),
            (
                |d| d.start_fuel_l = 1001,
                ShipError::Overfull {
                    fuel_l: 1001,
                    tank_l: 1000,
                },
            ),
            (
                |d| d.leg_burn_l = 0,
                ShipError::BurnOutOfRange {
                    field: "leg_burn_l",
                    liters: 0,
                },
            ),
            (|d| d.route.transit_sols = 0, ShipError::NoTransit),
            (|d| d.manifest.clear(), ShipError::EmptyManifest),
            (
                |d| d.manifest[1].id = d.manifest[0].id.clone(),
                ShipError::DuplicateCrate("crate_01".into()),
            ),
            (
                |d| d.manifest[2].integrity_pct = 101,
                ShipError::IntegrityTooHigh("crate_03".into()),
            ),
            (
                |d| d.manifest[0].mass_kg = u32::MAX,
                ShipError::HoldTooHeavy,
            ),
        ];
        for (bend, err) in cases {
            let mut def = shipped();
            bend(&mut def);
            assert_eq!(def.check(), Err(err));
        }
    }

    #[test]
    fn misspelled_fields_are_parse_errors() {
        let typo = MULLEIN.replacen("tank_l:", "tank:", 1);
        assert!(matches!(ShipDef::from_ron(&typo), Err(ShipError::Parse(_))));
    }

    #[test]
    fn burns_pods_and_cargo_never_go_below_zero() {
        let mut ship = Ship::default();
        assert_eq!(ship.burn(1200), 1000);
        assert_eq!(ship.fuel_l(), 0);
        assert_eq!(ship.spend_pods(5), 1);
        assert_eq!(ship.pods(), 0);
        assert_eq!(ship.dump_crates(3), (3, 3000));
        assert_eq!(ship.cargo().len(), 7);
        ship.damage_cargo(30);
        assert_eq!(ship.cargo_integrity_pct(), 70);
        ship.damage_cargo(200);
        assert_eq!(ship.cargo_integrity_pct(), 0);
        assert_eq!(ship.dump_crates(20), (7, 7000));
        assert_eq!(ship.cargo_integrity_pct(), 0);
    }

    #[test]
    fn integrity_is_weighted_by_mass() {
        let mut def = shipped();
        def.manifest.truncate(2);
        def.manifest[0].mass_kg = 3000;
        def.manifest[1].integrity_pct = 40;
        // (3000 * 100 + 1000 * 40) / 4000 = 85
        assert_eq!(Ship::new(def).cargo_integrity_pct(), 85);
    }
}
