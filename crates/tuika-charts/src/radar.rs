//! Shared radar geometry for the cell and pixel renderers.

/// A radar web laid out in terminal-cell coordinates.
pub(crate) struct RadarLayout {
    cols: u16,
    rows: u16,
    center: (f64, f64),
    radii: (f64, f64),
}

impl RadarLayout {
    pub(crate) fn new(cols: u16, rows: u16, labels: &[String]) -> Option<Self> {
        if cols < 20 || rows < 9 {
            return None;
        }
        let label_margin = labels
            .iter()
            .map(|label| usize::from(tuika::width::str_cols(label)))
            .max()
            .unwrap_or(0)
            .saturating_add(2)
            .min(usize::from(cols / 3)) as u16;
        let radius_y = f64::from(rows.saturating_sub(4)) / 2.0;
        // Most terminal cells are about twice as tall as they are wide. This
        // cap keeps the web visually round without hard-coding pixel metrics.
        let radius_x = (f64::from(cols.saturating_sub(label_margin.saturating_mul(2))) / 2.0)
            .min(radius_y * 2.75);
        if radius_x < 2.0 || radius_y < 2.0 {
            return None;
        }
        Some(Self {
            cols,
            rows,
            center: (
                f64::from(cols.saturating_sub(1)) / 2.0,
                f64::from(rows.saturating_sub(1)) / 2.0,
            ),
            radii: (radius_x, radius_y),
        })
    }

    pub(crate) fn center(&self) -> (f64, f64) {
        self.center
    }

    pub(crate) fn point(&self, index: usize, count: usize, scale: f64) -> (f64, f64) {
        let angle = self.angle(index, count);
        (
            self.center.0 + self.radii.0 * scale * angle.cos(),
            self.center.1 + self.radii.1 * scale * angle.sin(),
        )
    }

    pub(crate) fn label_position(
        &self,
        index: usize,
        count: usize,
        label: &str,
    ) -> Option<(u16, u16)> {
        let angle = self.angle(index, count);
        let (anchor_x, anchor_y) = self.point(index, count, 1.08);
        let width = i32::from(tuika::width::str_cols(label));
        let x = if angle.cos() < -0.2 {
            anchor_x.round() as i32 - width - 1
        } else if angle.cos() > 0.2 {
            anchor_x.round() as i32 + 1
        } else {
            anchor_x.round() as i32 - width / 2
        };
        let y = if angle.sin() < -0.2 {
            anchor_y.floor() as i32
        } else if angle.sin() > 0.2 {
            anchor_y.ceil() as i32
        } else {
            anchor_y.round() as i32
        };
        (x >= 0 && y >= 0 && x + width <= i32::from(self.cols) && y < i32::from(self.rows))
            .then_some((x as u16, y as u16))
    }

    fn angle(&self, index: usize, count: usize) -> f64 {
        -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * index as f64 / count as f64
    }
}
