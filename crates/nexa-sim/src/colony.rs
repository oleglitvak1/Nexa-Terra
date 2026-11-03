//! The colony: modules placed on the grid, built from a content file.

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

/// A module's live state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    def: ModuleDef,
    power_net: Option<usize>,
    heat_net: Option<usize>,
    active: bool,
    /// Whether the module got the power it asked for in the last hour.
    powered: bool,
    temp_centi_c: i32,
    charge_kwh: u32,
    machine: Option<Machine>,
}

impl Module {
    pub fn id(&self) -> &ModuleId {
        &self.def.id
    }

    pub fn def(&self) -> &ModuleDef {
        &self.def
    }

    pub fn power_net(&self) -> Option<usize> {
        self.power_net
    }

    pub fn heat_net(&self) -> Option<usize> {
        self.heat_net
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn powered(&self) -> bool {
        self.powered
    }

    pub fn temp_centi_c(&self) -> i32 {
        self.temp_centi_c
    }

    pub fn charge_kwh(&self) -> u32 {
        self.charge_kwh
    }

    pub fn machine(&self) -> Option<&Machine> {
        self.machine.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Colony {
    grid: Grid,
    ambient_centi_c: i32,
    heat_loss_percent: u8,
    modules: Vec<Module>,
    power_net_count: usize,
    /// Per heat network: the one module all its heat goes to, if diverted.
    heat_divert: Vec<Option<usize>>,
    inventory: Inventory,
}

impl Colony {
    /// Parse and check a colony content file.
    pub fn from_ron(text: &str) -> Result<Self, ContentError> {
        Self::from_def(ColonyDef::from_ron(text)?)
    }

    pub fn from_def(def: ColonyDef) -> Result<Self, ContentError> {
        if def.heat_loss_percent > 100 {
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
            let uses_power = m.output_kw > 0 || m.draw_kw > 0 || m.battery.is_some();
            if uses_power && power_net.is_none() {
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

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn ambient_centi_c(&self) -> i32 {
        self.ambient_centi_c
    }

    pub fn modules(&self) -> &[Module] {
        &self.modules
    }

    pub fn module(&self, id: &ModuleId) -> Option<&Module> {
        self.modules.iter().find(|m| m.id() == id)
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// Stored charge across all batteries, as a percentage of total capacity.
    pub fn power_percent(&self) -> u8 {
        let (charge, capacity) = self
            .modules
            .iter()
            .filter_map(|m| m.def.battery.map(|b| (m.charge_kwh, b.capacity_kwh)))
            .fold((0u64, 0u64), |(c, cap), (mc, mcap)| {
                (c + mc as u64, cap + mcap as u64)
            });
        // No batteries reads as 0%: there's nothing to draw on.
        (charge * 100).checked_div(capacity).unwrap_or(0) as u8
    }
}

/// The landing site the vertical slice starts on, compiled in so loading needs no file I/O.
pub const LANDING_ASH_PLAIN: &str = include_str!("../../../content/landing_ash_plain.ron");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::MachineState;
    use crate::parts::PartKind;

    fn ash_plain() -> Colony {
        Colony::from_ron(LANDING_ASH_PLAIN).unwrap()
    }

    fn id(s: &str) -> ModuleId {
        ModuleId::new(s)
    }

    /// A small valid colony to break in each test.
    fn tiny() -> ColonyDef {
        ColonyDef::from_ron(
            r#"(
                ambient_centi_c: -4000,
                heat_loss_percent: 10,
                layout: ["G=H", "..~"],
                modules: [
                    (id: "gen", glyph: 'G', output_kw: 10),
                    (id: "hab", glyph: 'H', draw_kw: 2),
                ],
            )"#,
        )
        .unwrap()
    }

    #[test]
    fn shipped_landing_site_loads() {
        let c = ash_plain();
        assert_eq!((c.grid().width(), c.grid().height()), (32, 32));
        assert!(c.modules().len() >= 6);
    }

    #[test]
    fn landing_site_wires_every_module_to_the_grid() {
        let c = ash_plain();
        for m in c.modules() {
            assert!(m.power_net().is_some(), "{} has no power", m.id());
        }
        // Habs, heaters and the smelter share one heat line.
        let line = c.module(&id("hab_01")).unwrap().heat_net();
        assert!(line.is_some());
        for m in ["hab_02", "heater_01", "heater_02", "smelter_01"] {
            assert_eq!(c.module(&id(m)).unwrap().heat_net(), line, "{m}");
        }
        assert_eq!(c.module(&id("gen_01")).unwrap().heat_net(), None);
    }

    #[test]
    fn landing_site_starts_nominal_with_a_damaged_hab() {
        let c = ash_plain();
        for m in c.modules() {
            if let Some(machine) = m.machine() {
                assert_eq!(machine.state(), MachineState::Nominal, "{}", m.id());
            }
        }
        let hab = c.module(&id("hab_01")).unwrap().machine().unwrap();
        assert!(hab.wear_bp() > 0);
    }

    #[test]
    fn landing_site_has_spare_parts_and_charge() {
        let c = ash_plain();
        assert!(c.inventory().count(PartKind::VitonSeal) > 0);
        assert!(c.power_percent() > 0);
    }

    #[test]
    fn tiny_colony_loads() {
        let c = Colony::from_def(tiny()).unwrap();
        assert_eq!(c.modules().len(), 2);
        assert_eq!(c.module(&id("hab")).unwrap().heat_net(), Some(0));
        assert_eq!(c.power_percent(), 0); // no batteries
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut def = tiny();
        def.modules[1].id = id("gen");
        assert_eq!(
            Colony::from_def(def),
            Err(ContentError::DuplicateId(id("gen")))
        );
    }

    #[test]
    fn rejects_glyphs_that_clash() {
        let mut def = tiny();
        def.modules[1].glyph = '=';
        assert!(matches!(
            Colony::from_def(def),
            Err(ContentError::GlyphTaken { .. })
        ));
    }

    #[test]
    fn rejects_modules_missing_from_the_map() {
        let mut def = tiny();
        def.layout = vec!["G=.".into(), "..~".into()];
        assert_eq!(
            Colony::from_def(def),
            Err(ContentError::NotOnMap(id("hab")))
        );
    }

    #[test]
    fn rejects_powered_modules_off_the_grid() {
        let mut def = tiny();
        def.layout = vec!["G=.H".into(), "...~".into()];
        assert_eq!(Colony::from_def(def), Err(ContentError::OffGrid(id("hab"))));
    }

    #[test]
    fn rejects_modules_bridging_two_networks() {
        let mut def = tiny();
        def.layout = vec!["G=H=".into(), "..~.".into()];
        // The second '=' isn't connected to the first, so H touches two power networks.
        assert!(matches!(
            Colony::from_def(def),
            Err(ContentError::SplitAcrossNetworks { kind: "power", .. })
        ));
    }

    #[test]
    fn rejects_overfull_batteries() {
        let mut def = tiny();
        def.modules[0].battery = Some(BatteryDef {
            capacity_kwh: 10,
            charge_kwh: 11,
        });
        assert_eq!(
            Colony::from_def(def),
            Err(ContentError::BatteryOverfull(id("gen")))
        );
    }

    #[test]
    fn rejects_inverted_tolerance_in_content() {
        let text = r#"(
            ambient_centi_c: 0, heat_loss_percent: 10, layout: ["A"],
            modules: [(id: "a", glyph: 'A',
                machine: Some((tolerance: (min_centi_c: 10, max_centi_c: -10), wear_bp_per_hour: 1)))],
        )"#;
        assert!(matches!(
            Colony::from_ron(text),
            Err(ContentError::Parse(_))
        ));
    }
}
