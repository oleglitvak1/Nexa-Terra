//! The colony: modules placed on the grid, built from a content file.

use crate::grid::{Grid, LayoutError, Tile};
use crate::machines::{Machine, ToleranceBand, MAX_WEAR_BP};
use crate::parts::{Inventory, PartKind};
use crate::SimEvent;
use nexa_types::{Command, ModuleId};
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

/// Fields are private and a save is checked on load (see [`SaveError`]), so the
/// tick can rely on what [`Colony::from_def`] guarantees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawColony")]
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

/// Unchecked wire form of [`Colony`]; serde checks it via `TryFrom`.
#[derive(Deserialize)]
struct RawColony {
    grid: Grid,
    ambient_centi_c: i32,
    heat_loss_percent: u8,
    modules: Vec<Module>,
    power_net_count: usize,
    heat_divert: Vec<Option<usize>>,
    inventory: Inventory,
}

/// Why a saved colony was rejected: it breaks something content loading guarantees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    HeatLossOutOfRange(u8),
    /// The tiles don't fill the grid, or name a module that doesn't exist.
    BadGrid,
    /// The module's networks aren't the ones its tiles touch.
    NetworksChanged(ModuleId),
    BatteryOverfull(ModuleId),
    /// The machine doesn't match its def or its module's temperature.
    MachineMismatch(ModuleId),
    /// A heat diversion points at a module that isn't on that line.
    BadDiversion,
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::HeatLossOutOfRange(p) => write!(f, "heat_loss_percent {p} is above 100"),
            SaveError::BadGrid => write!(f, "grid tiles don't match its size or modules"),
            SaveError::NetworksChanged(id) => {
                write!(f, "module {id}'s networks don't match the grid")
            }
            SaveError::BatteryOverfull(id) => write!(f, "module {id}'s battery is above capacity"),
            SaveError::MachineMismatch(id) => {
                write!(
                    f,
                    "module {id}'s machine doesn't match its def or temperature"
                )
            }
            SaveError::BadDiversion => write!(f, "a heat diversion targets a module off its line"),
        }
    }
}

impl std::error::Error for SaveError {}

impl TryFrom<RawColony> for Colony {
    type Error = SaveError;

    fn try_from(raw: RawColony) -> Result<Self, Self::Error> {
        if raw.heat_loss_percent > 100 {
            return Err(SaveError::HeatLossOutOfRange(raw.heat_loss_percent));
        }
        let n = raw.modules.len();
        if !raw.grid.is_consistent(n) {
            return Err(SaveError::BadGrid);
        }
        // Rebuild the networks from the tiles rather than trusting saved indices.
        let nets = raw.grid.networks(n);
        if raw.power_net_count != nets.power_count || raw.heat_divert.len() != nets.heat_count {
            return Err(SaveError::BadGrid);
        }
        for (i, m) in raw.modules.iter().enumerate() {
            let power: Vec<usize> = m.power_net.into_iter().collect();
            let heat: Vec<usize> = m.heat_net.into_iter().collect();
            if power != nets.module_power[i] || heat != nets.module_heat[i] {
                return Err(SaveError::NetworksChanged(m.def.id.clone()));
            }
            if m.charge_kwh > m.def.battery.map_or(0, |b| b.capacity_kwh) {
                return Err(SaveError::BatteryOverfull(m.def.id.clone()));
            }
            let machine_ok = match (&m.machine, &m.def.machine) {
                (Some(machine), Some(def)) => {
                    machine.tolerance() == def.tolerance && machine.temp_centi_c() == m.temp_centi_c
                }
                (None, None) => true,
                _ => false,
            };
            if !machine_ok {
                return Err(SaveError::MachineMismatch(m.def.id.clone()));
            }
        }
        for (net, divert) in raw.heat_divert.iter().enumerate() {
            if let Some(i) = *divert {
                if raw.modules.get(i).and_then(|m| m.heat_net) != Some(net) {
                    return Err(SaveError::BadDiversion);
                }
            }
        }
        Ok(Self {
            grid: raw.grid,
            ambient_centi_c: raw.ambient_centi_c,
            heat_loss_percent: raw.heat_loss_percent,
            modules: raw.modules,
            power_net_count: raw.power_net_count,
            heat_divert: raw.heat_divert,
            inventory: raw.inventory,
        })
    }
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

/// Why a command was refused. A refused command changes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    UnknownModule(ModuleId),
    NotOnHeatLine(ModuleId),
    NotFailed(ModuleId),
    MissingPart(PartKind),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::UnknownModule(id) => write!(f, "no module named {id}"),
            CommandError::NotOnHeatLine(id) => write!(f, "{id} is not on a heat line"),
            CommandError::NotFailed(id) => write!(f, "{id} has not failed"),
            CommandError::MissingPart(part) => write!(f, "no {part:?} in stock"),
        }
    }
}

impl std::error::Error for CommandError {}

impl Module {
    fn failed(&self) -> bool {
        self.machine.as_ref().is_some_and(Machine::is_failed)
    }

    /// Switched on and not failed: may generate, store or draw power this hour.
    fn on(&self) -> bool {
        self.active && !self.failed()
    }

    /// Doing its job this hour: switched on, not failed, and powered if it needs power.
    pub fn running(&self) -> bool {
        self.on() && (self.def.draw_kw == 0 || self.powered)
    }
}

impl Colony {
    /// One hour: route power, then heat, then update machine states and wear.
    pub fn tick(&mut self) -> Vec<SimEvent> {
        self.route_power();
        self.route_heat();
        self.update_machines()
    }

    pub fn apply(&mut self, cmd: &Command) -> Result<Vec<SimEvent>, CommandError> {
        match cmd {
            Command::Activate(id) => {
                let i = self.index(id)?;
                self.modules[i].active = true;
            }
            Command::Deactivate(id) => {
                let i = self.index(id)?;
                self.modules[i].active = false;
            }
            Command::DivertHeat { to } => {
                let i = self.index(to)?;
                let net = self.heat_line(i)?;
                self.heat_divert[net] = Some(i);
            }
            Command::ShareHeat { line_of } => {
                let i = self.index(line_of)?;
                let net = self.heat_line(i)?;
                self.heat_divert[net] = None;
            }
            Command::Repair(id) => {
                let i = self.index(id)?;
                let module = &mut self.modules[i];
                let Some(machine) = module.machine.as_mut() else {
                    return Err(CommandError::NotFailed(id.clone()));
                };
                let part = machine
                    .part_needed()
                    .ok_or_else(|| CommandError::NotFailed(id.clone()))?;
                if !self.inventory.take(part) {
                    return Err(CommandError::MissingPart(part));
                }
                machine.repair();
                return Ok(vec![SimEvent::MachineStateChanged {
                    module: id.clone(),
                    state: machine.state(),
                }]);
            }
        }
        Ok(Vec::new())
    }

    fn index(&self, id: &ModuleId) -> Result<usize, CommandError> {
        self.modules
            .iter()
            .position(|m| m.id() == id)
            .ok_or_else(|| CommandError::UnknownModule(id.clone()))
    }

    fn heat_line(&self, i: usize) -> Result<usize, CommandError> {
        self.modules[i]
            .heat_net
            .ok_or_else(|| CommandError::NotOnHeatLine(self.modules[i].def.id.clone()))
    }

    /// Generators and batteries supply each network's consumers in module order;
    /// a consumer that doesn't fit what's left goes dark this hour. A switched-off
    /// battery neither supplies nor charges.
    fn route_power(&mut self) {
        for m in &mut self.modules {
            m.powered = false;
        }
        for net in 0..self.power_net_count {
            let members: Vec<usize> = (0..self.modules.len())
                .filter(|&i| self.modules[i].power_net == Some(net) && self.modules[i].on())
                .collect();
            let generated: u32 = members.iter().map(|&i| self.modules[i].def.output_kw).sum();
            let stored: u32 = members.iter().map(|&i| self.modules[i].charge_kwh).sum();
            let mut available = generated + stored;
            let mut used = 0;
            for &i in &members {
                let m = &mut self.modules[i];
                let draw = m.def.draw_kw;
                if draw > 0 && draw <= available {
                    m.powered = true;
                    available -= draw;
                    used += draw;
                }
            }
            if generated >= used {
                let mut surplus = generated - used;
                for &i in &members {
                    let m = &mut self.modules[i];
                    if let Some(b) = m.def.battery {
                        let add = surplus.min(b.capacity_kwh - m.charge_kwh);
                        m.charge_kwh += add;
                        surplus -= add;
                    }
                }
            } else {
                let mut deficit = used - generated;
                for &i in &members {
                    let m = &mut self.modules[i];
                    let take = deficit.min(m.charge_kwh);
                    m.charge_kwh -= take;
                    deficit -= take;
                }
            }
        }
    }

    /// Running modules pour heat into their line's pool, shared among the line's
    /// working machines (or all of it to a diversion target); then everything
    /// cools toward ambient. A module off any line keeps its own heat.
    fn route_heat(&mut self) {
        let n = self.modules.len();
        let mut gain = vec![0i64; n];
        for (net, divert) in self.heat_divert.iter().enumerate() {
            let members: Vec<usize> = (0..n)
                .filter(|&i| self.modules[i].heat_net == Some(net))
                .collect();
            let pool: i64 = members
                .iter()
                .map(|&i| &self.modules[i])
                .filter(|m| m.running())
                .map(|m| i64::from(m.def.heat_out_centi_c))
                .sum();
            let recipients = match divert {
                Some(target) => vec![*target],
                None => {
                    // A failed machine gets no share, so it cools like the
                    // wreck it is; diverting to it on purpose still works.
                    let working: Vec<usize> = members
                        .iter()
                        .copied()
                        .filter(|&i| self.modules[i].machine.is_some() && !self.modules[i].failed())
                        .collect();
                    if working.is_empty() {
                        members
                            .into_iter()
                            .filter(|&i| !self.modules[i].failed())
                            .collect()
                    } else {
                        working
                    }
                }
            };
            share(pool, &recipients, &mut gain);
        }
        for (i, m) in self.modules.iter().enumerate() {
            if m.heat_net.is_none() && m.running() {
                gain[i] += i64::from(m.def.heat_out_centi_c);
            }
        }
        let ambient = i64::from(self.ambient_centi_c);
        let loss = i64::from(self.heat_loss_percent);
        for (m, g) in self.modules.iter_mut().zip(gain) {
            let t = i64::from(m.temp_centi_c) + g;
            let t = t - (t - ambient) * loss / 100;
            m.temp_centi_c = t.clamp(i32::MIN.into(), i32::MAX.into()) as i32;
        }
    }

    fn update_machines(&mut self) -> Vec<SimEvent> {
        let mut events = Vec::new();
        for m in &mut self.modules {
            let (Some(machine), Some(def)) = (m.machine.as_mut(), m.def.machine) else {
                continue;
            };
            let before = machine.state();
            machine.set_temperature(m.temp_centi_c);
            machine.wear_for_an_hour(def.wear_bp_per_hour);
            if machine.state() != before {
                events.push(SimEvent::MachineStateChanged {
                    module: m.def.id.clone(),
                    state: machine.state(),
                });
            }
        }
        events
    }
}

/// Split `pool` evenly; the first recipients take the remainder so no heat is lost.
fn share(pool: i64, recipients: &[usize], gain: &mut [i64]) {
    let count = recipients.len() as i64;
    if count == 0 {
        return;
    }
    let (each, rem) = (pool.div_euclid(count), pool.rem_euclid(count));
    for (k, &i) in recipients.iter().enumerate() {
        gain[i] += each + i64::from((k as i64) < rem);
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

    // --- Saves: a loaded colony must hold what content loading guarantees.

    /// Save `c`, rewrite one field of the RON text, and load it back.
    fn reload_with(c: &Colony, from: &str, to: &str) -> Result<Colony, ron::error::SpannedError> {
        let saved = ron::to_string(c).unwrap();
        assert!(saved.contains(from), "{from} not in save");
        ron::from_str(&saved.replacen(from, to, 1))
    }

    #[test]
    fn an_untouched_save_loads() {
        let mut c = ash_plain();
        c.apply(&Command::DivertHeat { to: id("hab_01") }).unwrap();
        run(&mut c, 30);
        assert_eq!(reload_with(&c, "(", "(").unwrap(), c);
    }

    #[test]
    fn saves_with_an_overfull_battery_are_rejected() {
        let c = ash_plain();
        // The live charge, which the save writes just before `machine`.
        assert!(reload_with(&c, "charge_kwh:300,machine", "charge_kwh:450,machine").is_err());
    }

    #[test]
    fn saves_with_a_module_moved_off_its_network_are_rejected() {
        let c = ash_plain();
        assert!(reload_with(&c, "power_net:Some(0)", "power_net:None").is_err());
    }

    #[test]
    fn saves_with_a_machine_at_another_temperature_are_rejected() {
        let c = ash_plain();
        let saved = ron::to_string(&c).unwrap();
        // The smelter's machine copy of its temperature, not the module's.
        let machine_temp = "tolerance:(min_centi_c:-500,max_centi_c:8000),temp_centi_c:1400";
        assert!(saved.contains(machine_temp));
        let edited = saved.replacen(
            machine_temp,
            "tolerance:(min_centi_c:-500,max_centi_c:8000),temp_centi_c:1500",
            1,
        );
        assert!(ron::from_str::<Colony>(&edited).is_err());
    }

    fn run(c: &mut Colony, hours: u64) -> Vec<SimEvent> {
        (0..hours).flat_map(|_| c.tick()).collect()
    }

    fn state(c: &Colony, m: &str) -> MachineState {
        c.module(&id(m)).unwrap().machine().unwrap().state()
    }

    fn changed(m: &str, state: MachineState) -> SimEvent {
        SimEvent::MachineStateChanged {
            module: id(m),
            state,
        }
    }

    // --- Power: a 10 kW generator, an empty battery and two 6 kW loads.

    fn power_rig() -> Colony {
        Colony::from_ron(
            r#"(
                ambient_centi_c: 0, heat_loss_percent: 0,
                layout: ["GBAC", "===="],
                modules: [
                    (id: "gen", glyph: 'G', output_kw: 10),
                    (id: "bat", glyph: 'B', battery: Some((capacity_kwh: 20, charge_kwh: 0))),
                    (id: "a", glyph: 'A', draw_kw: 6),
                    (id: "c", glyph: 'C', draw_kw: 6),
                ],
            )"#,
        )
        .unwrap()
    }

    fn charge(c: &Colony) -> u32 {
        c.module(&id("bat")).unwrap().charge_kwh()
    }

    fn powered(c: &Colony, m: &str) -> bool {
        c.module(&id(m)).unwrap().powered()
    }

    #[test]
    fn short_power_goes_to_modules_in_order_and_surplus_charges_the_battery() {
        let mut c = power_rig();
        c.tick();
        assert!(powered(&c, "a"));
        assert!(!powered(&c, "c"));
        assert_eq!(charge(&c), 4);
    }

    #[test]
    fn battery_covers_the_deficit_until_it_runs_dry() {
        let mut c = power_rig();
        c.tick(); // 10 available: only a runs, battery charges to 4
        c.tick(); // 14 available: both run, 2 drawn from the battery
        assert!(powered(&c, "c"));
        assert_eq!(charge(&c), 2);
        c.tick(); // 12 available: both run, battery empty
        assert_eq!(charge(&c), 0);
        c.tick(); // 10 available: c goes dark again
        assert!(!powered(&c, "c"));
    }

    #[test]
    fn charging_stops_at_capacity() {
        let mut c = power_rig();
        c.apply(&Command::Deactivate(id("a"))).unwrap();
        c.apply(&Command::Deactivate(id("c"))).unwrap();
        run(&mut c, 5);
        assert_eq!(charge(&c), 20);
        assert_eq!(c.power_percent(), 100);
    }

    #[test]
    fn a_switched_off_battery_neither_supplies_nor_charges() {
        let mut c = power_rig();
        c.tick(); // battery charges to 4
        c.apply(&Command::Deactivate(id("bat"))).unwrap();
        c.tick(); // 10 available without the battery: c stays dark
        assert!(!powered(&c, "c"));
        assert_eq!(charge(&c), 4);
    }

    #[test]
    fn a_switched_off_generator_produces_nothing() {
        let mut c = power_rig();
        c.apply(&Command::Deactivate(id("gen"))).unwrap();
        c.tick();
        assert!(!powered(&c, "a"));
    }

    // --- Heat: two machines and a 601-per-hour heater on one line.

    fn heat_rig(loss: u8) -> Colony {
        Colony::from_ron(&format!(
            r#"(
                ambient_centi_c: 0, heat_loss_percent: {loss},
                layout: ["XYR", "~~~"],
                modules: [
                    (id: "x", glyph: 'X', machine: Some((
                        tolerance: (min_centi_c: -100000, max_centi_c: 100000), wear_bp_per_hour: 0))),
                    (id: "y", glyph: 'Y', machine: Some((
                        tolerance: (min_centi_c: -100000, max_centi_c: 100000), wear_bp_per_hour: 0))),
                    (id: "heater", glyph: 'R', heat_out_centi_c: 601),
                ],
            )"#
        ))
        .unwrap()
    }

    fn temp(c: &Colony, m: &str) -> i32 {
        c.module(&id(m)).unwrap().temp_centi_c()
    }

    #[test]
    fn line_heat_is_shared_among_its_machines_without_loss() {
        let mut c = heat_rig(0);
        c.tick();
        assert_eq!((temp(&c, "x"), temp(&c, "y")), (301, 300));
        assert_eq!(temp(&c, "heater"), 0, "the heater itself isn't a recipient");
    }

    #[test]
    fn diverting_sends_all_line_heat_to_one_module_until_shared_again() {
        let mut c = heat_rig(0);
        c.apply(&Command::DivertHeat { to: id("y") }).unwrap();
        c.tick();
        assert_eq!((temp(&c, "x"), temp(&c, "y")), (0, 601));

        c.apply(&Command::ShareHeat { line_of: id("x") }).unwrap();
        c.tick();
        assert_eq!((temp(&c, "x"), temp(&c, "y")), (301, 901));
    }

    #[test]
    fn a_switched_off_heater_adds_no_heat() {
        let mut c = heat_rig(0);
        c.apply(&Command::Deactivate(id("heater"))).unwrap();
        c.tick();
        assert_eq!(temp(&c, "x"), 0);
    }

    #[test]
    fn modules_cool_toward_ambient() {
        let mut c = Colony::from_ron(
            r#"(ambient_centi_c: 0, heat_loss_percent: 10, layout: ["X"],
                modules: [(id: "x", glyph: 'X', start_temp_centi_c: Some(1000))])"#,
        )
        .unwrap();
        c.tick();
        assert_eq!(temp(&c, "x"), 900);
        c.tick();
        assert_eq!(temp(&c, "x"), 810);
    }

    // --- Wear, failure and repair.

    /// A machine that drops to -10 C at once and wears 50% an hour while frozen.
    fn freezer() -> Colony {
        Colony::from_ron(
            r#"(
                ambient_centi_c: -1000, heat_loss_percent: 100, layout: ["X"],
                modules: [(id: "x", glyph: 'X', start_temp_centi_c: Some(0), machine: Some((
                    tolerance: (min_centi_c: -500, max_centi_c: 500), wear_bp_per_hour: 5000)))],
                parts: { VitonSeal: 1 },
            )"#,
        )
        .unwrap()
    }

    #[test]
    fn sustained_freeze_takes_the_machine_offline() {
        let mut c = freezer();
        assert_eq!(c.tick(), vec![changed("x", MachineState::Freeze)]);
        assert_eq!(c.tick(), vec![changed("x", MachineState::Offline)]);
        assert!(run(&mut c, 3).is_empty(), "failure is sticky");
    }

    #[test]
    fn repair_uses_the_matching_part() {
        let mut c = freezer();
        run(&mut c, 2);
        // Still below the band after the repair, so it's freezing again.
        assert_eq!(
            c.apply(&Command::Repair(id("x"))),
            Ok(vec![changed("x", MachineState::Freeze)])
        );
        assert_eq!(c.inventory().count(PartKind::VitonSeal), 0);
        assert_eq!(
            c.apply(&Command::Repair(id("x"))),
            Err(CommandError::NotFailed(id("x")))
        );

        run(&mut c, 2);
        assert_eq!(
            c.apply(&Command::Repair(id("x"))),
            Err(CommandError::MissingPart(PartKind::VitonSeal))
        );
        assert_eq!(state(&c, "x"), MachineState::Offline);
    }

    #[test]
    fn burned_out_machines_draw_no_power_and_make_no_heat() {
        let mut c = Colony::from_ron(
            r#"(
                ambient_centi_c: 0, heat_loss_percent: 0, layout: ["GS", "=="],
                modules: [
                    (id: "gen", glyph: 'G', output_kw: 10),
                    (id: "s", glyph: 'S', draw_kw: 5, heat_out_centi_c: 1000, machine: Some((
                        tolerance: (min_centi_c: -500, max_centi_c: 500), wear_bp_per_hour: 10000))),
                ],
            )"#,
        )
        .unwrap();
        assert_eq!(c.tick(), vec![changed("s", MachineState::Burnout)]);
        c.tick();
        assert!(!powered(&c, "s"));
        assert_eq!(temp(&c, "s"), 1000);
    }

    #[test]
    fn failed_machines_get_no_share_of_line_heat() {
        // x burns out on its first hour; after that the heater's 601 all goes to y.
        let mut c = Colony::from_ron(
            r#"(
                ambient_centi_c: 0, heat_loss_percent: 0,
                layout: ["XYR", "~~~"],
                modules: [
                    (id: "x", glyph: 'X', machine: Some((
                        tolerance: (min_centi_c: -100000, max_centi_c: 100), wear_bp_per_hour: 10000))),
                    (id: "y", glyph: 'Y', machine: Some((
                        tolerance: (min_centi_c: -100000, max_centi_c: 100000), wear_bp_per_hour: 0))),
                    (id: "heater", glyph: 'R', heat_out_centi_c: 601),
                ],
            )"#,
        )
        .unwrap();
        assert_eq!(c.tick(), vec![changed("x", MachineState::Burnout)]);
        assert_eq!((temp(&c, "x"), temp(&c, "y")), (301, 300));
        c.tick();
        assert_eq!((temp(&c, "x"), temp(&c, "y")), (301, 901));
    }

    #[test]
    fn a_failed_hab_cools_toward_ambient() {
        let mut c = ash_plain();
        c.apply(&Command::Deactivate(id("heater_01"))).unwrap();
        // The damaged seal cracks at hour 53, just above -4 C.
        let events = run(&mut c, 53);
        assert_eq!(
            events.last(),
            Some(&changed("hab_01", MachineState::Offline))
        );
        let before = temp(&c, "hab_01");
        run(&mut c, 26);
        assert!(
            temp(&c, "hab_01") < before - 1000,
            "{before} -> {}",
            temp(&c, "hab_01")
        );
    }

    #[test]
    fn commands_reject_unknown_modules_and_modules_off_heat_lines() {
        let mut c = power_rig();
        assert_eq!(
            c.apply(&Command::Activate(id("nope"))),
            Err(CommandError::UnknownModule(id("nope")))
        );
        assert_eq!(
            c.apply(&Command::DivertHeat { to: id("a") }),
            Err(CommandError::NotOnHeatLine(id("a")))
        );
    }

    // --- Scenarios on the shipped landing site.

    #[test]
    fn landfall_holds_steady_for_a_sol() {
        let mut c = ash_plain();
        let start = c.power_percent();
        let events = run(&mut c, 26);
        assert!(events.is_empty(), "{events:?}");
        for m in ["hab_01", "hab_02", "smelter_01"] {
            assert_eq!(state(&c, m), MachineState::Nominal, "{m}");
        }
        assert!(c.power_percent() < start);
    }

    #[test]
    fn without_orders_the_battery_runs_flat_in_about_three_sols() {
        let mut c = ash_plain();
        run(&mut c, 70);
        assert!(c.power_percent() > 0);
        run(&mut c, 10);
        assert!(c.power_percent() <= 5);
    }

    // architecture.md's scenario test: heater off for 3 Sols freezes the seal.
    #[test]
    fn heater_off_for_three_sols_freezes_the_seal() {
        let mut c = ash_plain();
        c.apply(&Command::Deactivate(id("heater_01"))).unwrap();
        let events = run(&mut c, 3 * 26);
        // The damaged hab's seal cracks within the three Sols. The line's heat
        // then goes to the machines still working, so the sound hab recovers.
        assert!(events.contains(&changed("hab_01", MachineState::Offline)));
        assert!(events.contains(&changed("hab_02", MachineState::Freeze)));
        assert_eq!(state(&c, "hab_01"), MachineState::Offline);
        assert_eq!(state(&c, "hab_02"), MachineState::Nominal);
    }

    #[test]
    fn diverting_heat_warms_one_hab_at_the_others_expense() {
        let mut c = ash_plain();
        c.apply(&Command::DivertHeat { to: id("hab_01") }).unwrap();
        run(&mut c, 6);
        assert!(temp(&c, "hab_01") > 1400);
        assert!(temp(&c, "hab_02") < 1400);
    }
}
