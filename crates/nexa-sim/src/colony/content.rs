//! Colony content files: their data shapes and the checks that build a [`Colony`].

use super::{Colony, Module, MAX_HEAT_LOSS_PERCENT};
use crate::grid::{Grid, LayoutError, Tile};
use crate::machines::{Machine, ToleranceBand, MAX_WEAR_BP};
use crate::parts::Inventory;
use nexa_types::{Guard, ModuleId};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// A colony content file, e.g. `content/landing_ash_plain.ron`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColonyDef {
    /// Outside temperature every module cools toward.
    pub ambient_centi_c: i32,
    /// Share of a module's gap to ambient lost each hour, 0..=100.
    pub heat_loss_percent: u8,
    /// Map rows; see [`Tile::LEGEND`], plus each module's glyph.
    pub layout: Vec<String>,
    pub modules: Vec<ModuleDef>,
    #[serde(default)]
    pub parts: Inventory,
}

/// One module's fixed data. A module may generate, store or draw power, push
/// heat into its heat line, and have a tolerance band (a [`Machine`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDef {
    pub id: ModuleId,
    /// How plain-words text names it, verbatim and written bare, article
    /// included where one belongs (`name: "the smelter"`, `name: "heater 2"`). The
    /// ID stands in without one.
    #[serde(
        default,
        deserialize_with = "crate::present",
        serialize_with = "crate::bare",
        skip_serializing_if = "Option::is_none"
    )]
    pub name: Option<String>,
    #[serde(default)]
    pub kind: ModuleKind,
    /// Marks this module's tiles in the layout.
    pub glyph: char,
    #[serde(default)]
    pub output_kw: u32,
    #[serde(default)]
    pub draw_kw: u32,
    #[serde(default)]
    pub battery: Option<BatteryDef>,
    /// Heat added to the module's heat line each hour it runs.
    #[serde(default)]
    pub heat_out_centi_c: i32,
    #[serde(default)]
    pub machine: Option<MachineDef>,
    #[serde(default = "default_true")]
    pub starts_active: bool,
    /// Defaults to the ambient temperature.
    #[serde(default)]
    pub start_temp_centi_c: Option<i32>,
    /// Makes ore while running in its band; at most one module per colony.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ore: Option<OreDef>,
    /// Life support's saturation meter; at most one module per colony.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrubber: Option<ScrubberDef>,
    /// The battery guard this module ships with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<Guard>,
}

impl ModuleDef {
    /// The name plain-words text uses: `name`, or the ID.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(self.id.as_str())
    }

    /// Generates, stores or draws power, so it must touch a power conduit.
    pub fn uses_power(&self) -> bool {
        self.output_kw > 0 || self.draw_kw > 0 || self.battery.is_some()
    }
}

/// What a module is for, where other rules care (crew bunk in habs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ModuleKind {
    #[default]
    Utility,
    /// Living quarters. Needs a machine, since crew stress follows its state.
    Hab,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryDef {
    pub capacity_kwh: u32,
    pub charge_kwh: u32,
}

/// Ore output: whole kilograms an hour, packed into crates of `crate_kg`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OreDef {
    pub kg_per_hour: u32,
    pub crate_kg: u32,
    /// What a crate holds, e.g. "raw silicon".
    pub cargo: String,
}

/// The scrubber's saturation meter, in whole units. It is full at `capacity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrubberDef {
    pub capacity: u32,
    /// Added each hour the scrubber runs.
    pub per_running_hour: u32,
    /// Added each hour it is switched off or unpowered.
    pub per_dark_hour: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineDef {
    pub tolerance: ToleranceBand,
    /// Wear added per hour outside the band, in basis points.
    pub wear_bp_per_hour: u16,
    #[serde(default)]
    pub start_wear_bp: u16,
}

/// Why a colony content file was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentError {
    Parse(String),
    Layout(LayoutError),
    HeatLossOutOfRange(u8),
    DuplicateId(ModuleId),
    /// The glyph is already a legend tile or another module's glyph.
    GlyphTaken {
        module: ModuleId,
        glyph: char,
    },
    NotOnMap(ModuleId),
    /// Generates, stores or draws power but touches no power conduit.
    OffGrid(ModuleId),
    /// Touches more than one network of a kind ("power" or "heat").
    SplitAcrossNetworks {
        module: ModuleId,
        kind: &'static str,
    },
    BatteryOverfull(ModuleId),
    StartWearTooHigh(ModuleId),
    /// A hab with no tolerance band, so its bunks could never go cold.
    HabWithoutMachine(ModuleId),
    /// Ore needs a nonzero rate, crate mass and cargo name, and a machine,
    /// since it is made only while the machine is in its band.
    BadOre(ModuleId),
    /// A second ore producer; the slice has one smelter feeding one hold.
    TwoOreProducers(ModuleId),
    /// A scrubber needs a capacity, a running rate, a dark rate at least as
    /// high, and a power draw, since unpowered hours count as dark.
    BadScrubber(ModuleId),
    TwoScrubbers(ModuleId),
    /// A guard needs ordered thresholds and a module that draws power.
    BadGuard(ModuleId),
    BlankName(ModuleId),
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentError::Parse(e) => write!(f, "colony content doesn't parse: {e}"),
            ContentError::Layout(e) => write!(f, "colony layout: {e}"),
            ContentError::HeatLossOutOfRange(p) => {
                write!(f, "heat_loss_percent {p} is above {MAX_HEAT_LOSS_PERCENT}")
            }
            ContentError::DuplicateId(id) => write!(f, "module id {id} is used twice"),
            ContentError::GlyphTaken { module, glyph } => {
                write!(f, "module {module}'s glyph {glyph:?} is already taken")
            }
            ContentError::NotOnMap(id) => write!(f, "module {id} has no tiles on the map"),
            ContentError::OffGrid(id) => {
                write!(f, "module {id} uses power but touches no power conduit")
            }
            ContentError::SplitAcrossNetworks { module, kind } => {
                write!(f, "module {module} touches more than one {kind} network")
            }
            ContentError::BatteryOverfull(id) => {
                write!(f, "module {id}'s battery starts above its capacity")
            }
            ContentError::StartWearTooHigh(id) => {
                write!(f, "module {id} starts at or above failure wear")
            }
            ContentError::HabWithoutMachine(id) => {
                write!(f, "hab {id} has no machine")
            }
            ContentError::BadOre(id) => write!(
                f,
                "module {id}'s ore needs a rate, crate mass, cargo and a machine"
            ),
            ContentError::TwoOreProducers(id) => {
                write!(f, "module {id} is a second ore producer")
            }
            ContentError::BadScrubber(id) => write!(
                f,
                "module {id}'s scrubber needs a capacity, rates (dark at least running) and a power draw"
            ),
            ContentError::TwoScrubbers(id) => write!(f, "module {id} is a second scrubber"),
            ContentError::BlankName(id) => write!(f, "module {id}'s name is blank"),
            ContentError::BadGuard(id) => write!(
                f,
                "module {id}'s guard needs pause below resume, resume at most 100%, and a power draw"
            ),
        }
    }
}

impl std::error::Error for ContentError {}

impl ColonyDef {
    pub fn from_ron(text: &str) -> Result<Self, ContentError> {
        ron::from_str(text).map_err(|e| ContentError::Parse(e.to_string()))
    }

    /// The shipped landing site, parsed once; checked by tests, so failing here is a build bug.
    pub fn shipped() -> &'static Self {
        static SHIPPED: OnceLock<ColonyDef> = OnceLock::new();
        SHIPPED.get_or_init(|| {
            Self::from_ron(super::LANDING_ASH_PLAIN).expect("colony content is valid")
        })
    }
}

/// The name and the ore, scrubber and guard blocks: each valid, and one
/// producer and one scrubber at most. Shared by content loading and save loading.
pub(super) fn check_blocks<'a>(
    modules: impl IntoIterator<Item = &'a ModuleDef>,
) -> Result<(), ContentError> {
    let mut ore_seen = false;
    let mut scrubber_seen = false;
    for m in modules {
        if m.name.as_ref().is_some_and(|n| n.trim().is_empty()) {
            return Err(ContentError::BlankName(m.id.clone()));
        }
        if let Some(ore) = &m.ore {
            let ok = ore.kg_per_hour > 0
                && ore.crate_kg > 0
                && !ore.cargo.is_empty()
                && m.machine.is_some();
            if !ok {
                return Err(ContentError::BadOre(m.id.clone()));
            }
            if std::mem::replace(&mut ore_seen, true) {
                return Err(ContentError::TwoOreProducers(m.id.clone()));
            }
        }
        if let Some(sc) = m.scrubber {
            let ok = sc.capacity > 0
                && sc.per_running_hour > 0
                && sc.per_dark_hour >= sc.per_running_hour
                && m.draw_kw > 0;
            if !ok {
                return Err(ContentError::BadScrubber(m.id.clone()));
            }
            if std::mem::replace(&mut scrubber_seen, true) {
                return Err(ContentError::TwoScrubbers(m.id.clone()));
            }
        }
        if let Some(guard) = m.guard {
            if !guard.is_valid() || m.draw_kw == 0 {
                return Err(ContentError::BadGuard(m.id.clone()));
            }
        }
    }
    Ok(())
}

impl Colony {
    /// Parse and check a colony content file.
    pub fn from_ron(text: &str) -> Result<Self, ContentError> {
        Self::from_def(ColonyDef::from_ron(text)?)
    }

    pub fn from_def(def: ColonyDef) -> Result<Self, ContentError> {
        if def.heat_loss_percent > MAX_HEAT_LOSS_PERCENT {
            return Err(ContentError::HeatLossOutOfRange(def.heat_loss_percent));
        }
        let mut glyphs: Vec<char> = Vec::with_capacity(def.modules.len());
        for (i, m) in def.modules.iter().enumerate() {
            if def.modules[..i].iter().any(|other| other.id == m.id) {
                return Err(ContentError::DuplicateId(m.id.clone()));
            }
            let in_legend = Tile::LEGEND.iter().any(|(g, _)| *g == m.glyph);
            if in_legend || glyphs.contains(&m.glyph) {
                return Err(ContentError::GlyphTaken {
                    module: m.id.clone(),
                    glyph: m.glyph,
                });
            }
            glyphs.push(m.glyph);
        }

        check_blocks(&def.modules)?;

        let grid = Grid::parse(&def.layout, &glyphs).map_err(ContentError::Layout)?;
        let tile_counts = grid.module_tile_counts(def.modules.len());
        let nets = grid.networks(def.modules.len());

        let mut modules = Vec::with_capacity(def.modules.len());
        for (i, m) in def.modules.into_iter().enumerate() {
            if tile_counts[i] == 0 {
                return Err(ContentError::NotOnMap(m.id));
            }
            let single = |list: &[usize], kind| match list {
                [] => Ok(None),
                [n] => Ok(Some(*n)),
                _ => Err(ContentError::SplitAcrossNetworks {
                    module: m.id.clone(),
                    kind,
                }),
            };
            let power_net = single(&nets.module_power[i], "power")?;
            let heat_net = single(&nets.module_heat[i], "heat")?;
            if m.uses_power() && power_net.is_none() {
                return Err(ContentError::OffGrid(m.id));
            }
            let charge_kwh = match m.battery {
                Some(b) if b.charge_kwh > b.capacity_kwh => {
                    return Err(ContentError::BatteryOverfull(m.id))
                }
                Some(b) => b.charge_kwh,
                None => 0,
            };
            if m.kind == ModuleKind::Hab && m.machine.is_none() {
                return Err(ContentError::HabWithoutMachine(m.id));
            }
            let temp_centi_c = m.start_temp_centi_c.unwrap_or(def.ambient_centi_c);
            let machine = match m.machine {
                Some(md) if md.start_wear_bp >= MAX_WEAR_BP => {
                    return Err(ContentError::StartWearTooHigh(m.id))
                }
                Some(md) => {
                    Some(Machine::new(md.tolerance, temp_centi_c).with_wear(md.start_wear_bp))
                }
                None => None,
            };
            modules.push(Module {
                power_net,
                heat_net,
                active: m.starts_active,
                powered: false,
                temp_centi_c,
                charge_kwh,
                machine,
                def: m,
            });
        }

        Ok(Self {
            grid,
            ambient_centi_c: def.ambient_centi_c,
            heat_loss_percent: def.heat_loss_percent,
            modules,
            power_net_count: nets.power_count,
            heat_divert: vec![None; nets.heat_count],
            inventory: def.parts,
            ore_kg: 0,
        })
    }
}
