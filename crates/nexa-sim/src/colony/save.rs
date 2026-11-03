//! Loading a saved [`Colony`]: the unchecked wire form and what it must satisfy.

use super::{Colony, Module, MAX_HEAT_LOSS_PERCENT};
use crate::grid::Grid;
use crate::parts::Inventory;
use nexa_types::ModuleId;
use serde::Deserialize;

/// Unchecked wire form of [`Colony`]; serde checks it via `TryFrom`.
#[derive(Deserialize)]
pub(super) struct RawColony {
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
        if raw.heat_loss_percent > MAX_HEAT_LOSS_PERCENT {
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
