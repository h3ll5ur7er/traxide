use traxide_contracts::geometry::{Point, Rect};

/// A uniform grid over the board; each cell lists the items that touch it.
#[derive(Debug, Clone)]
pub(crate) struct Buckets {
    origin: Point,
    size: i32,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<usize>>,
}

impl Buckets {
    pub(crate) fn new(area: Rect, size: i32) -> Self {
        let span = |from: i32, to: i32| usize::try_from((to - from) / size + 1).unwrap_or(1);
        let (columns, rows) = (span(area.min.x, area.max.x), span(area.min.y, area.max.y));
        Self {
            origin: area.min,
            size,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
        }
    }

    pub(crate) fn insert(&mut self, bounds: Rect, item: usize) {
        for cell in self.cells_in(bounds) {
            self.cells[cell].push(item);
        }
    }

    /// Lists `item`, within `bounds`, only in the cells `touches` says it
    /// reaches: a long diagonal track's bounds cover many cells it never comes
    /// near, where every query would otherwise find it and check it in full.
    /// The outermost cells stand for everything beyond the area too.
    pub(crate) fn insert_touching(
        &mut self,
        bounds: Rect,
        item: usize,
        touches: impl Fn(Rect) -> bool,
    ) {
        for cell in self.cells_in(bounds) {
            if touches(self.reach_of(cell)) {
                self.cells[cell].push(item);
            }
        }
    }

    /// What `cell` stands for: its square, stretched to the far side of the
    /// plane at the edges of the area.
    fn reach_of(&self, cell: usize) -> Rect {
        const BEYOND: i32 = i32::MAX / 4;
        let (column, row) = (cell % self.columns, cell / self.columns);
        let start = |index: usize, from: i32| {
            from + i32::try_from(index).unwrap_or(i32::MAX / 2) * self.size
        };
        let (x, y) = (start(column, self.origin.x), start(row, self.origin.y));
        Rect {
            min: Point::new(
                if column == 0 { -BEYOND } else { x },
                if row == 0 { -BEYOND } else { y },
            ),
            max: Point::new(
                if column + 1 == self.columns {
                    BEYOND
                } else {
                    x + self.size
                },
                if row + 1 == self.rows {
                    BEYOND
                } else {
                    y + self.size
                },
            ),
        }
    }

    /// Takes out `item`, inserted with `bounds`.
    pub(crate) fn remove(&mut self, bounds: Rect, item: usize) {
        for cell in self.cells_in(bounds) {
            let items = &mut self.cells[cell];
            if let Some(at) = items.iter().position(|&i| i == item) {
                items.swap_remove(at);
            }
        }
    }

    /// Items that may touch `area`, possibly more than once and possibly not at all.
    pub(crate) fn candidates(&self, area: Rect) -> impl Iterator<Item = usize> + '_ {
        self.cells_in(area)
            .flat_map(|cell| self.cells[cell].iter().copied())
    }

    pub(crate) fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// The cell `at` falls in, or the nearest cell at the edge of the area.
    pub(crate) fn cell_at(&self, at: Point) -> usize {
        self.cells_in(Rect { min: at, max: at })
            .next()
            .unwrap_or_default()
    }

    pub(crate) fn centre_of(&self, cell: usize) -> Point {
        let (column, row) = (cell % self.columns, cell / self.columns);
        let at = |index: usize, from: i32| {
            from + i32::try_from(index).unwrap_or(i32::MAX / 2) * self.size + self.size / 2
        };
        Point::new(at(column, self.origin.x), at(row, self.origin.y))
    }

    pub(crate) fn is_empty(&self, cell: usize) -> bool {
        self.cells[cell].is_empty()
    }

    fn cells_in(&self, area: Rect) -> impl Iterator<Item = usize> + use<> {
        // Items beyond the board still land in the outermost cells, so nothing is lost.
        let clamp = |value: i32, from: i32, count: usize| {
            usize::try_from((value - from).div_euclid(self.size))
                .unwrap_or(0)
                .min(count - 1)
        };
        let (x0, x1) = (
            clamp(area.min.x, self.origin.x, self.columns),
            clamp(area.max.x, self.origin.x, self.columns),
        );
        let (y0, y1) = (
            clamp(area.min.y, self.origin.y, self.rows),
            clamp(area.max.y, self.origin.y, self.rows),
        );
        let columns = self.columns;
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| y * columns + x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x1: i32, y1: i32, x2: i32, y2: i32) -> Rect {
        Rect {
            min: Point::new(x1, y1),
            max: Point::new(x2, y2),
        }
    }

    #[test]
    fn finds_items_in_the_cells_they_span() {
        let mut buckets = Buckets::new(rect(0, 0, 100, 100), 10);
        buckets.insert(rect(5, 5, 25, 5), 7);
        assert!(buckets.candidates(rect(21, 0, 22, 9)).any(|i| i == 7));
        assert!(!buckets.candidates(rect(50, 50, 60, 60)).any(|i| i == 7));
    }

    #[test]
    fn items_beyond_the_area_are_kept_at_its_edge() {
        let mut buckets = Buckets::new(rect(0, 0, 100, 100), 10);
        buckets.insert(rect(-50, -50, -40, -40), 3);
        assert!(buckets.candidates(rect(-45, -45, -44, -44)).any(|i| i == 3));
    }

    #[test]
    fn an_item_removed_is_found_in_none_of_its_cells_and_the_rest_stay() {
        let mut buckets = Buckets::new(rect(0, 0, 100, 100), 10);
        buckets.insert(rect(5, 5, 25, 5), 7);
        buckets.insert(rect(5, 5, 6, 6), 8);
        buckets.remove(rect(5, 5, 25, 5), 7);
        assert!(!buckets.candidates(rect(0, 0, 100, 100)).any(|i| i == 7));
        assert!(buckets.candidates(rect(5, 5, 6, 6)).any(|i| i == 8));
        assert!(buckets.is_empty(buckets.cell_at(Point::new(21, 5))));
    }

    #[test]
    fn an_item_is_listed_only_in_the_cells_it_touches() {
        let mut buckets = Buckets::new(rect(0, 0, 100, 100), 10);
        let on_the_diagonal =
            |cell: Rect| cell.min.x - cell.min.y < 10 && cell.min.y - cell.min.x < 10;
        buckets.insert_touching(rect(0, 0, 99, 99), 4, on_the_diagonal);
        assert!(buckets.candidates(rect(55, 55, 56, 56)).any(|i| i == 4));
        assert!(!buckets.candidates(rect(5, 85, 6, 86)).any(|i| i == 4));
    }

    #[test]
    fn the_outermost_cells_stand_for_everything_beyond_the_area() {
        let mut buckets = Buckets::new(rect(0, 0, 100, 100), 10);
        let far_out = |cell: Rect| cell.min.x <= -500 && cell.min.y <= -500;
        buckets.insert_touching(rect(-600, -600, -500, -500), 3, far_out);
        assert!(
            buckets
                .candidates(rect(-550, -550, -549, -549))
                .any(|i| i == 3)
        );
    }
}
