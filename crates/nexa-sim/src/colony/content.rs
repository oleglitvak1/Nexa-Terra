//! Colony content files: their data shapes and the checks that build a [`Colony`].

use super::{Colony, Module, MAX_HEAT_LOSS_PERCENT};
use crate::grid::{Grid, LayoutError, Tile};
use crate::machines::{Machine, ToleranceBand, MAX_WEAR_BP};
use crate::parts::Inventory;
use nexa_types::ModuleId;
use serde::{Deserialize, Serialize};

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
}

impl ModuleDef {
    /// Generates, stores or draws power, so it must touch a power conduit.
    pub fn uses_power(&self) -> bool {
        self.output_kw > 0 || self.draw_kw > 0 || self.battery.is_some()
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryDef {
    pub capacity_kwh: u32,
    pub charge_kwh: u32,
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
}

impl std::fmt::Display for ContentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentError::Parse(e) => write!(f, "colony content doesn't parse: {e}"),
            ContentError::Layout(e) => write!(f, "colony layout: {e}"),
            ContentError::HeatLossOutOfRange(p) => {
                write!(f, "heat_loss_percent {p} is above 100")
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
        }
    }
}

impl std::error::Error for ContentError {}

impl ColonyDef {
    pub fn from_ron(text: &str) -> Result<Self, ContentError> {
        ron::from_str(text).map_err(|e| ContentError::Parse(e.to_string()))
    }
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
        })
    }
}
