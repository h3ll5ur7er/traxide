//! A plane's fill as the board's program pours it: its outline within the
//! board, around the copper of other nets, without the necks narrower than
//! its minimum width, in pieces that copper of its net may lie on apart.

use crate::pieces::Piece;
use std::collections::VecDeque;
use traxide_contracts::geometry::{Point, Polygon, Rect, Segment, Shape};
use traxide_contracts::{ConstraintSet, Design, Plane};

/// The raster the fill is worked out on: fine against clearances of tenths of
/// a millimetre, coarse enough for a whole board.
const CELL: i32 = 100_000;

pub(crate) struct Fill {
    /// The centre of the first cell.
    origin: Point,
    columns: usize,
    rows: usize,
    /// Per cell, from 1, the piece of fill whose middle it is part of; 0 where
    /// none is. The middle is the fill shrunk by half the minimum width, so a
    /// neck too narrow for the fill parts it.
    piece: Vec<u32>,
}

impl Fill {
    pub(crate) fn pour(
        design: &Design,
        rules: &ConstraintSet,
        plane: &Plane,
        pieces: &[Piece],
    ) -> Self {
        let bounds = plane.outline.bounds();
        let cells = |length: i32| usize::try_from(length / CELL + 1).unwrap_or(0);
        let mut fill = Self {
            origin: Point::new(bounds.min.x + CELL / 2, bounds.min.y + CELL / 2),
            columns: cells(bounds.max.x - bounds.min.x),
            rows: cells(bounds.max.y - bounds.min.y),
            piece: Vec::new(),
        };
        let mut open = vec![false; fill.columns * fill.rows];
        fill.scan(&plane.outline, |cell| open[cell] = true);
        for hole in &plane.holes {
            fill.scan(hole, |cell| open[cell] = false);
        }
        let mut on_the_board = vec![false; open.len()];
        for polygon in design.outline.polygons() {
            fill.scan(polygon, |cell| on_the_board[cell] = true);
        }
        for hole in design.outline.holes() {
            fill.scan(hole, |cell| on_the_board[cell] = false);
        }
        for other in design.planes.iter() {
            if plane.gives_way_to(other) {
                fill.scan(&other.outline, |cell| open[cell] = false);
            }
        }
        for (cell, inside) in on_the_board.into_iter().enumerate() {
            open[cell] &= inside;
        }
        let half = plane.min_width / 2;
        let edge = |segment| Shape::Capsule { segment, radius: 0 };
        for segment in plane
            .outline
            .edges()
            .chain(plane.holes.iter().flat_map(Polygon::edges))
        {
            fill.close_within(&edge(segment), half, &mut open);
        }
        for polygon in design.outline.polygons() {
            for segment in polygon.edges() {
                fill.close_within(&edge(segment), rules.edge_clearance() + half, &mut open);
            }
        }
        for (index, hole) in design.outline.holes().iter().enumerate() {
            for segment in hole.edges() {
                fill.close_within(
                    &edge(segment),
                    rules.cutout_clearance(index) + half,
                    &mut open,
                );
            }
        }
        for other in &design.planes {
            if plane.gives_way_to(other) {
                let clearance = (plane.clearance.max(other.clearance))
                    .max(rules.clearance(Some(plane.net), Some(other.net)));
                for segment in other.outline.edges() {
                    fill.close_within(&edge(segment), clearance + half, &mut open);
                }
            }
        }
        for other in pieces {
            if other.layer == plane.layer && other.net != Some(plane.net) {
                let clearance = plane
                    .clearance
                    .max(rules.clearance(Some(plane.net), other.net));
                fill.close_within(&other.shape, clearance + half, &mut open);
            }
        }
        fill.piece = fill.number_the_pieces(&open);
        fill
    }

    /// The pieces of fill that copper of the plane's net of `shape` touches:
    /// the fill reaches half its minimum width beyond its middle.
    pub(crate) fn pieces_touching(&self, shape: &Shape, plane: &Plane) -> Vec<u32> {
        let reach = plane.min_width / 2 + CELL;
        let mut touched = Vec::new();
        self.for_each_cell_in(shape.bounds().inflated(reach), |cell, centre| {
            let piece = self.piece[cell];
            if piece != 0 && point(centre).is_closer_than(shape, reach) {
                touched.push(piece);
            }
        });
        touched.sort_unstable();
        touched.dedup();
        touched
    }

    fn centre(&self, column: usize, row: usize) -> Point {
        let step = |n: usize| i32::try_from(n).unwrap_or(i32::MAX / CELL) * CELL;
        Point::new(self.origin.x + step(column), self.origin.y + step(row))
    }

    /// The cells whose centres lie inside `polygon`, row by row.
    fn scan(&self, polygon: &Polygon, mut cell: impl FnMut(usize)) {
        for row in 0..self.rows {
            let y = self.centre(0, row).y;
            let mut crossings: Vec<i64> = polygon
                .edges()
                .filter(|e| (e.a.y <= y) != (e.b.y <= y))
                .map(|e| {
                    let (ax, ay) = (i64::from(e.a.x), i64::from(e.a.y));
                    let (bx, by) = (i64::from(e.b.x), i64::from(e.b.y));
                    ax + (bx - ax) * (i64::from(y) - ay) / (by - ay)
                })
                .collect();
            crossings.sort_unstable();
            for [from, to] in crossings.as_chunks::<2>().0 {
                let first = (from - i64::from(self.origin.x)).div_euclid(i64::from(CELL)) + 1;
                let last = (to - i64::from(self.origin.x)).div_euclid(i64::from(CELL));
                for column in first.max(0)..=last.min(self.columns as i64 - 1) {
                    cell(row * self.columns + column as usize);
                }
            }
        }
    }

    /// Takes the cells whose centres lie within `distance` of `shape` out of the fill.
    fn close_within(&self, shape: &Shape, distance: i32, open: &mut [bool]) {
        self.for_each_cell_in(shape.bounds().inflated(distance), |cell, centre| {
            if open[cell] && point(centre).is_closer_than(shape, distance) {
                open[cell] = false;
            }
        });
    }

    fn for_each_cell_in(&self, area: Rect, mut f: impl FnMut(usize, Point)) {
        let index = |v: i32, origin: i32, count: usize| {
            usize::try_from((v - origin).div_euclid(CELL).max(0))
                .unwrap_or(0)
                .min(count)
        };
        let columns = index(area.min.x, self.origin.x, self.columns)
            ..index(area.max.x, self.origin.x, self.columns)
                .saturating_add(1)
                .min(self.columns);
        let rows = index(area.min.y, self.origin.y, self.rows)
            ..index(area.max.y, self.origin.y, self.rows)
                .saturating_add(1)
                .min(self.rows);
        for row in rows {
            for column in columns.clone() {
                f(row * self.columns + column, self.centre(column, row));
            }
        }
    }

    /// Numbers the pieces of the open cells, side to side.
    fn number_the_pieces(&self, open: &[bool]) -> Vec<u32> {
        let mut piece = vec![0_u32; open.len()];
        let mut next = 0;
        let mut queue = VecDeque::new();
        for start in 0..open.len() {
            if !open[start] || piece[start] != 0 {
                continue;
            }
            next += 1;
            piece[start] = next;
            queue.push_back(start);
            while let Some(cell) = queue.pop_front() {
                let (row, column) = (cell / self.columns, cell % self.columns);
                let sides = [
                    (column > 0).then(|| cell - 1),
                    (column + 1 < self.columns).then_some(cell + 1),
                    (row > 0).then(|| cell - self.columns),
                    (row + 1 < self.rows).then_some(cell + self.columns),
                ];
                for side in sides.into_iter().flatten() {
                    if open[side] && piece[side] == 0 {
                        piece[side] = next;
                        queue.push_back(side);
                    }
                }
            }
        }
        piece
    }
}

fn point(at: Point) -> Shape {
    Shape::Capsule {
        segment: Segment::new(at, at),
        radius: 0,
    }
}
