use crate::buckets::Buckets;
use traxide_contracts::ConstraintSet;
use traxide_contracts::geometry::{Rect, Region, Segment, Shape};

/// The board's outline, indexed so that a question about a piece of copper
/// looks only at the edges near it. Boards with rounded corners have outlines
/// of hundreds of edges, and testing them all made every check of every
/// search slow.
#[derive(Debug, Clone)]
pub(crate) struct Outline {
    region: Region,
    bounds: Rect,
    edges: Vec<Segment>,
    /// Per edge, how far copper keeps from it: the edge clearance, or a
    /// cutout's own.
    clearances: Vec<i32>,
    widest_clearance: i32,
    buckets: Buckets,
    /// For each bucket no edge passes through, which side of the outline it is
    /// on; `None` where edges pass.
    inside: Vec<Option<bool>>,
}

impl Outline {
    pub(crate) fn new(region: &Region, rules: &ConstraintSet, bucket_size: i32) -> Self {
        let bounds = region.bounds();
        let mut buckets = Buckets::new(bounds, bucket_size);
        let edges: Vec<Segment> = region.edges().collect();
        // The region's edges are its polygons', then its holes', in order.
        let outer = region.polygons().iter().map(|p| p.edges().count()).sum();
        let mut clearances = vec![rules.edge_clearance(); outer];
        for (hole, polygon) in region.holes().iter().enumerate() {
            let edges = polygon.edges().count();
            clearances.extend(std::iter::repeat_n(rules.cutout_clearance(hole), edges));
        }
        debug_assert_eq!(clearances.len(), edges.len());
        let widest_clearance = clearances.iter().copied().max().unwrap_or(0);
        for (index, edge) in edges.iter().enumerate() {
            let edge_bounds =
                Rect::around([edge.a, edge.b]).unwrap_or_else(|| unreachable!("two points"));
            buckets.insert(edge_bounds, index);
        }
        let inside = (0..buckets.cell_count())
            .map(|cell| {
                buckets
                    .is_empty(cell)
                    .then(|| region.contains(buckets.centre_of(cell)))
            })
            .collect();
        Self {
            region: region.clone(),
            bounds,
            edges,
            clearances,
            widest_clearance,
            buckets,
            inside,
        }
    }

    /// Whether `shape` lies inside the outline, keeping each edge's clearance.
    pub(crate) fn holds(&self, shape: &Shape) -> bool {
        let reach = shape.bounds().inflated(self.widest_clearance);
        let near_an_edge = self.buckets.candidates(reach).any(|index| {
            Shape::Capsule {
                segment: self.edges[index],
                radius: 0,
            }
            .is_closer_than(shape, self.clearances[index])
        });
        if near_an_edge {
            return false;
        }
        // Clear of every edge, the shape lies wholly on one side: one point
        // decides which.
        let point = match shape {
            Shape::Capsule { segment, .. } => segment.a,
            Shape::Polygon(polygon) => polygon.points()[0],
        };
        if !self.bounds.contains(point) {
            return false;
        }
        self.inside[self.buckets.cell_at(point)].unwrap_or_else(|| self.region.contains(point))
    }
}
