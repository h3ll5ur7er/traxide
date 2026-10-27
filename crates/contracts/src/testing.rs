//! Small designs for tests, built in a few readable lines.

use crate::{
    Design, DesignRules, Keepout, KeepoutScope, Layer, LayerId, LayerShape, Net, NetClass,
    NetClassId, NetId, Pad, PadId, Plane, Routes, ViaKind, ViaKindId,
};
use traxide_geometry::{Point, Polygon, Rect, Region, Shape};

/// Millimetres in nanometres.
pub fn mm(value: f64) -> i32 {
    (value * 1_000_000.0).round() as i32
}

pub fn at(x_mm: f64, y_mm: f64) -> Point {
    Point::new(mm(x_mm), mm(y_mm))
}

/// A rectangular board with tracks of 0.2 mm, clearances of 0.2 mm and through
/// vias of 0.6 mm on every layer, unless told otherwise.
#[derive(Debug, Clone)]
pub struct DesignBuilder {
    design: Design,
}

impl DesignBuilder {
    pub fn new(width_mm: f64, height_mm: f64) -> Self {
        let outline = Region::from(Polygon::rectangle(Rect {
            min: at(0.0, 0.0),
            max: at(width_mm, height_mm),
        }));
        let design = Design {
            layers: vec![],
            outline,
            cutout_footprints: Vec::new(),
            locked: Routes::default(),
            nets: vec![],
            pads: vec![],
            via_kinds: vec![],
            keepouts: vec![],
            planes: vec![],
            existing: Routes::default(),
            rules: DesignRules {
                classes: vec![NetClass {
                    name: "default".into(),
                    track_width: mm(0.2),
                    clearance: mm(0.2),
                    via_kinds: vec![ViaKindId(0)],
                }],
                default_class: NetClassId(0),
                edge_clearance: mm(0.2),
                hole_clearance: 0,
                hole_to_hole: 0,
                min_track_width: 0,
                custom: vec![],
            },
            resolution: 0,
            outline_tolerance: 0,
            warnings: Vec::new(),
        };
        Self { design }.layers(2)
    }

    /// Replaces the layers with `count` routable ones, and the via with a through via.
    pub fn layers(mut self, count: usize) -> Self {
        self.design.layers = (0..count)
            .map(|i| Layer {
                name: format!("L{}", i + 1),
                routable: true,
            })
            .collect();
        let copper = (0..count)
            .map(|i| LayerShape {
                layer: LayerId::from_index(i),
                shape: Shape::circle(Point::default(), mm(0.6)),
            })
            .collect();
        self.design.via_kinds = vec![ViaKind {
            name: "via".into(),
            copper,
            drill: Some(mm(0.3)),
        }];
        self
    }

    pub fn track_width(mut self, width_mm: f64) -> Self {
        self.design.rules.classes[0].track_width = mm(width_mm);
        self
    }

    pub fn clearance(mut self, clearance_mm: f64) -> Self {
        self.design.rules.classes[0].clearance = mm(clearance_mm);
        self.design.rules.edge_clearance = mm(clearance_mm);
        self
    }

    pub fn without_vias(mut self) -> Self {
        self.design.rules.classes[0].via_kinds.clear();
        self
    }

    /// A square SMD pad of `size_mm` on `layer`.
    pub fn smd(
        self,
        name: &str,
        net: Option<&str>,
        center: Point,
        size_mm: f64,
        layer: usize,
    ) -> Self {
        let half = mm(size_mm) / 2;
        let rect = Rect {
            min: Point::new(center.x - half, center.y - half),
            max: Point::new(center.x + half, center.y + half),
        };
        let copper = vec![LayerShape {
            layer: LayerId::from_index(layer),
            shape: Shape::Polygon(Polygon::rectangle(rect)),
        }];
        self.pad(name, net, center, copper)
    }

    /// A round through-hole pad on every layer.
    pub fn tht(self, name: &str, net: Option<&str>, center: Point, diameter_mm: f64) -> Self {
        let copper = (0..self.design.layers.len())
            .map(|i| LayerShape {
                layer: LayerId::from_index(i),
                shape: Shape::circle(center, mm(diameter_mm)),
            })
            .collect();
        self.pad(name, net, center, copper)
    }

    pub fn keepout(mut self, shape: Shape, layer: Option<usize>, scope: KeepoutScope) -> Self {
        self.design.keepouts.push(Keepout {
            layer: layer.map(LayerId::from_index),
            shape,
            scope,
            clearance: 0,
        });
        self
    }

    /// Makes `layer` a plane layer, which carries no tracks.
    pub fn plane_layer(mut self, layer: usize) -> Self {
        self.design.layers[layer].routable = false;
        self
    }

    /// A pour of `net` filling the rectangle from `min` to `max` on `layer`.
    pub fn plane(mut self, net: &str, layer: usize, min: Point, max: Point) -> Self {
        let net = self.net_named(net);
        self.design.planes.push(Plane {
            net,
            layer: LayerId::from_index(layer),
            outline: Polygon::rectangle(Rect { min, max }),
            holes: Vec::new(),
            clearance: 0,
            min_width: 0,
        });
        self
    }

    fn pad(
        mut self,
        name: &str,
        net: Option<&str>,
        position: Point,
        copper: Vec<LayerShape>,
    ) -> Self {
        let id = PadId::from_index(self.design.pads.len());
        let net = net.map(|n| self.net_named(n));
        if let Some(net) = net {
            self.design.nets[net.index()].pads.push(id);
        }
        self.design.pads.push(Pad {
            name: name.into(),
            net,
            position,
            copper,
        });
        self
    }

    fn net_named(&mut self, name: &str) -> NetId {
        let existing = self.design.nets.iter().position(|n| n.name == name);
        NetId::from_index(existing.unwrap_or_else(|| {
            self.design.nets.push(Net {
                name: name.into(),
                class: NetClassId(0),
                pads: vec![],
            });
            self.design.nets.len() - 1
        }))
    }

    pub fn build(self) -> Design {
        self.design
    }
}

impl Design {
    /// The id of the net called `name`; panics if there is none, as a test should.
    pub fn net_named(&self, name: &str) -> NetId {
        NetId::from_index(
            self.nets
                .iter()
                .position(|n| n.name == name)
                .expect("the net exists"),
        )
    }

    pub fn pad_named(&self, name: &str) -> PadId {
        PadId::from_index(
            self.pads
                .iter()
                .position(|p| p.name == name)
                .expect("the pad exists"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_join_the_nets_they_name() {
        let design = DesignBuilder::new(10.0, 10.0)
            .smd("R1-1", Some("A"), at(1.0, 1.0), 1.0, 0)
            .tht("R2-1", Some("A"), at(5.0, 5.0), 1.6)
            .smd("R1-2", None, at(3.0, 1.0), 1.0, 0)
            .build();
        let net = design.net(design.net_named("A"));
        assert_eq!(net.pads, [PadId(0), PadId(1)]);
        assert_eq!(design.pad(PadId(1)).copper.len(), 2);
        assert_eq!(design.pad(PadId(2)).net, None);
    }
}
