//! Retained-history selection uses character positions, independent of viewport widgets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub row: usize,
    pub column: usize,
}
#[derive(Default)]
pub struct Selection {
    pub anchor: Option<Position>,
    pub head: Option<Position>,
    pub dragging: bool,
}
impl Selection {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn bounds(&self) -> Option<(Position, Position)> {
        let (a, b) = (self.anchor?, self.head?);
        Some((a.min(b), a.max(b)))
    }
    pub fn select_all(&mut self, rows: usize, last_columns: usize) {
        if rows == 0 {
            self.clear();
            return;
        }
        self.anchor = Some(Position::default());
        self.head = Some(Position {
            row: rows - 1,
            column: last_columns,
        });
    }
    /// Drop only the evicted part of a selection; retained text keeps its position.
    pub fn evict(&mut self, count: usize) {
        if let Some((_, end)) = self.bounds() {
            if end.row < count {
                self.clear();
                return;
            }
        }
        for point in [&mut self.anchor, &mut self.head].into_iter().flatten() {
            if point.row < count {
                point.column = 0;
            }
            point.row = point.row.saturating_sub(count);
        }
    }
    pub fn insert_row(&mut self, row: usize) {
        for point in [&mut self.anchor, &mut self.head].into_iter().flatten() {
            if point.row >= row {
                point.row += 1;
            }
        }
    }
    pub fn columns(&self, row: usize, length: usize) -> Option<std::ops::Range<usize>> {
        let (start, end) = self.bounds()?;
        if row < start.row || row > end.row {
            return None;
        }
        Some(
            (if row == start.row {
                start.column.min(length)
            } else {
                0
            })..(if row == end.row {
                end.column.min(length)
            } else {
                length
            }),
        )
    }
    pub fn copy(&self, rows: usize, mut text: impl FnMut(usize) -> String) -> String {
        let Some((start, end)) = self.bounds() else {
            return String::new();
        };
        let mut output = String::new();
        for row in start.row..=end.row.min(rows.saturating_sub(1)) {
            if row >= rows {
                break;
            }
            let value = text(row);
            if row != start.row {
                output.push('\n');
            }
            let range = self.columns(row, value.chars().count()).unwrap();
            output.extend(value.chars().skip(range.start).take(range.len()));
        }
        output
    }
}
