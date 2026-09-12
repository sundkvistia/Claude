//! The pan/zoom camera mapping document (world) coordinates to screen pixels.

pub const MIN_ZOOM: f64 = 0.1;
pub const MAX_ZOOM: f64 = 20.0;

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
}

impl Default for View {
    fn default() -> Self {
        View { pan_x: 0.0, pan_y: 0.0, zoom: 1.0 }
    }
}

impl View {
    pub fn screen_to_world(&self, sx: f64, sy: f64) -> (f64, f64) {
        ((sx - self.pan_x) / self.zoom, (sy - self.pan_y) / self.zoom)
    }

    pub fn pan_by(&mut self, dx: f64, dy: f64) {
        self.pan_x += dx;
        self.pan_y += dy;
    }

    /// Multiply the zoom by `factor`, keeping the world point currently under
    /// screen point `(cx, cy)` fixed in place.
    pub fn zoom_at(&mut self, factor: f64, cx: f64, cy: f64) {
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let (wx, wy) = self.screen_to_world(cx, cy);
        self.zoom = new_zoom;
        self.pan_x = cx - wx * self.zoom;
        self.pan_y = cy - wy * self.zoom;
    }
}
