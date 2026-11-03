//! Wear parts and the colony's stock of them.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PartKind {
    /// Replaces a filament destroyed by overheat (Burnout).
    TungstenFilament,
    /// Replaces a seal cracked by freeze (Offline).
    VitonSeal,
    SilicateBaffleMesh,
    /// Off-world only; the life-support scrubbers need them by Sol 45.
    CarbonFilter,
}

/// Part counts. A BTreeMap so iteration and saves are deterministic.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Inventory(BTreeMap<PartKind, u32>);

impl Inventory {
    pub fn count(&self, part: PartKind) -> u32 {
        self.0.get(&part).copied().unwrap_or(0)
    }

    pub fn add(&mut self, part: PartKind, n: u32) {
        *self.0.entry(part).or_insert(0) += n;
    }

    /// Take one part; false (and no change) if none are in stock.
    pub fn take(&mut self, part: PartKind) -> bool {
        match self.0.get_mut(&part) {
            Some(n) if *n > 0 => {
                *n -= 1;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_consumes_one_and_fails_when_empty() {
        let mut inv = Inventory::default();
        inv.add(PartKind::VitonSeal, 1);
        assert!(inv.take(PartKind::VitonSeal));
        assert_eq!(inv.count(PartKind::VitonSeal), 0);
        assert!(!inv.take(PartKind::VitonSeal));
        assert!(!inv.take(PartKind::CarbonFilter));
    }

    #[test]
    fn reads_from_ron_map() {
        let inv: Inventory = ron::from_str("{ VitonSeal: 2, TungstenFilament: 1 }").unwrap();
        assert_eq!(inv.count(PartKind::VitonSeal), 2);
        assert_eq!(inv.count(PartKind::TungstenFilament), 1);
    }
}
