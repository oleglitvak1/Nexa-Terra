//! The colony: modules placed on the grid, built from a content file.

mod content;
mod save;

pub use content::{BatteryDef, ColonyDef, ContentError, MachineDef, ModuleDef, ModuleKind};
pub use save::SaveError;

use crate::grid::Grid;
use crate::machines::Machine;
use crate::parts::{Inventory, PartKind};
use crate::SimEvent;
use nexa_types::{Command, ModuleId};
use save::RawColony;
use serde::{Deserialize, Serialize};

/// A module can't lose more than its whole gap to ambient in an hour.
const MAX_HEAT_LOSS_PERCENT: u8 = 100;

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

impl Colony {
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn modules(&self) -> &[Module] {
        &self.modules
    }

    /// Saves from before `ModuleKind` load every module as Utility. Their habs
    /// were the `hab_*` modules, which all had a machine.
    pub(crate) fn mark_legacy_habs(&mut self) {
        for m in &mut self.modules {
            if m.def.id.as_str().starts_with("hab_") && m.def.machine.is_some() {
                m.def.kind = ModuleKind::Hab;
            }
        }
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
    pub fn failed(&self) -> bool {
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
    fn rejects_habs_without_a_machine() {
        let mut def = tiny();
        def.modules[1].kind = ModuleKind::Hab;
        assert_eq!(
            Colony::from_def(def),
            Err(ContentError::HabWithoutMachine(id("hab")))
        );
    }

    #[test]
    fn landing_site_habs_are_marked() {
        let c = ash_plain();
        let habs: Vec<_> = c
            .modules()
            .iter()
            .filter(|m| m.def().kind == ModuleKind::Hab)
            .map(|m| m.id().as_str())
            .collect();
        assert_eq!(habs, ["hab_01", "hab_02"]);
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
