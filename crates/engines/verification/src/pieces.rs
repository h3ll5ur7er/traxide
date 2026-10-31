use crate::fill::Fill;
use crate::unions::Unions;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use traxide_contracts::geometry::{Point, Rect, Segment, Shape};
use traxide_contracts::{ConstraintSet, CopperItem, Design, LayerId, NetId, PadId, Routes};

/// One shape of copper on one layer.
#[derive(Debug, Clone)]
pub(crate) struct Piece {
    pub(crate) layer: LayerId,
    pub(crate) net: Option<NetId>,
    pub(crate) shape: Shape,
    pub(crate) bounds: Rect,
    pub(crate) item: CopperItem,
}

impl Piece {
    fn new(layer: LayerId, net: Option<NetId>, shape: Shape, item: CopperItem) -> Self {
        Self {
            layer,
            net,
            bounds: shape.bounds(),
            shape,
            item,
        }
    }

    pub(crate) fn is_pad(&self) -> bool {
        matches!(self.item, CopperItem::Pad(_))
    }
}

pub(crate) fn collect(design: &Design, routes: &Routes) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for (index, pad) in design.pads.iter().enumerate() {
        for copper in &pad.copper {
            pieces.push(Piece::new(
                copper.layer,
                pad.net,
                copper.shape.clone(),
                CopperItem::Pad(PadId::from_index(index)),
            ));
        }
    }
    for (index, track) in routes.tracks.iter().enumerate() {
        for pair in track.path.windows(2) {
            let shape = Shape::track(Segment::new(pair[0], pair[1]), track.width);
            pieces.push(Piece::new(
                track.layer,
                Some(track.net),
                shape,
                CopperItem::Track(index),
            ));
        }
    }
    for (index, via) in routes.vias.iter().enumerate() {
        let at = via.position;
        for copper in &design.via_kind(via.kind).copper {
            let shape = copper.shape.map(|p| Point::new(p.x + at.x, p.y + at.y));
            pieces.push(Piece::new(
                copper.layer,
                Some(via.net),
                shape,
                CopperItem::Via(index),
            ));
        }
    }
    pieces
}

/// The layers of one through-hole pad or via are connected by its barrel, and
/// the segments of one track by their shared points.
pub(crate) fn join_layers_of_the_same_item(pieces: &[Piece], joined: &mut Unions) {
    let mut by_item: Vec<usize> = (0..pieces.len()).collect();
    by_item.sort_by_key(|&i| pieces[i].item);
    for pair in by_item.windows(2) {
        if pieces[pair[0]].item == pieces[pair[1]].item {
            joined.join(pair[0], pair[1]);
        }
    }
}

/// A plane joins the copper of its net that lies on one piece of its fill,
/// poured around other nets' copper as the board's program pours it; no
/// plane fills where one of another net of at least its priority overlaps
/// it, as only the board's program tells which of two of one priority does. Only on a layer that takes no tracks: elsewhere
/// routes of other nets may cut a pour apart, and only refilling it tells where.
pub(crate) fn join_through_planes(
    design: &Design,
    rules: &ConstraintSet,
    pieces: &[Piece],
    joined: &mut Unions,
) {
    for plane in design
        .planes
        .iter()
        .filter(|p| !design.layers[p.layer.index()].routable)
    {
        let fill = Fill::pour(design, rules, plane, pieces);
        let mut first_on: HashMap<u32, usize> = HashMap::new();
        for (index, piece) in pieces.iter().enumerate() {
            if piece.net != Some(plane.net) || piece.layer != plane.layer {
                continue;
            }
            for part in fill.pieces_touching(&piece.shape, plane) {
                match first_on.entry(part) {
                    Entry::Occupied(first) => joined.join(*first.get(), index),
                    Entry::Vacant(none) => {
                        none.insert(index);
                    }
                }
            }
        }
    }
}
