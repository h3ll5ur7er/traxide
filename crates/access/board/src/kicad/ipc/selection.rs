//! The nets of the items selected in KiCad's PCB editor.

use super::messages::board::types::{
    Arc, BoardGraphicShape, FootprintInstance, Net, Pad, Track, Via, Zone, zone,
};
use super::messages::{Any, Packed};
use std::collections::BTreeSet;

/// The names of the nets of `items`, each once: a copper item's own net, a
/// footprint's pads' nets; items of no net add none.
pub(super) fn nets_of(items: &[Any]) -> Vec<String> {
    let mut nets = BTreeSet::new();
    for item in items {
        nets.extend(nets_of_item(item).into_iter().map(|net| net.name));
    }
    nets.remove("");
    nets.into_iter().collect()
}

fn nets_of_item(item: &Any) -> Vec<Net> {
    if let Some(track) = item.to::<Track>() {
        track.net.into_iter().collect()
    } else if let Some(arc) = item.to::<Arc>() {
        arc.net.into_iter().collect()
    } else if let Some(via) = item.to::<Via>() {
        via.net.into_iter().collect()
    } else if let Some(pad) = item.to::<Pad>() {
        pad.net.into_iter().collect()
    } else if let Some(shape) = item.to::<BoardGraphicShape>() {
        shape.net.into_iter().collect()
    } else if let Some(zone) = item.to::<Zone>() {
        match zone.settings {
            Some(zone::Settings::CopperSettings(copper)) => copper.net.into_iter().collect(),
            _ => Vec::new(),
        }
    } else if let Some(footprint) = item.to::<FootprintInstance>() {
        footprint
            .definition
            .into_iter()
            .flat_map(|definition| definition.items)
            .filter_map(|item| item.to::<Pad>())
            .filter_map(|pad| pad.net)
            .collect()
    } else {
        Vec::new()
    }
}
