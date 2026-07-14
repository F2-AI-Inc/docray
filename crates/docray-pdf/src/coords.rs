use docray_model::{round3, BBox};

/// Maps raw PDF coordinates into the rotated, top-left, y-down space the schema
/// promises ("coordinates ... after page rotation is applied", per the design
/// spec).
///
/// Empirically (pdfium-render 0.8.37): for a page carrying a `/Rotate` entry,
/// pdfium reports object/char geometry (`bounds`, `loose_bounds`, `origin_*`) in
/// the *unrotated* PDF coordinate space — y-up, bottom-left origin, within the
/// unrotated media box — exactly as if there were no rotation. It reports
/// `page.width()/height()`, however, as the *rotated* visible dimensions (a
/// 612x792 media box with `/Rotate 90` reports 792x612). So the raw geometry
/// must be both y-flipped and rotated to land in the visible page's top-left
/// space; a plain y-flip against the visible height (the pre-fix behaviour)
/// produced wrong/negative coordinates on rotated pages.
#[derive(Clone, Copy)]
pub struct PageSpace {
    /// Clockwise page rotation in degrees: 0, 90, 180, or 270.
    rotation: i32,
    /// Unrotated media-box width (points).
    unrotated_w: f64,
    /// Unrotated media-box height (points).
    unrotated_h: f64,
}

impl PageSpace {
    /// Build from the page rotation and the *rotated* (visible) dimensions that
    /// pdfium reports, recovering the unrotated media-box dimensions that the raw
    /// coordinates actually live in. 90/270 swap width and height between the
    /// unrotated and visible frames.
    pub fn new(rotation: i32, rotated_w: f64, rotated_h: f64) -> PageSpace {
        let (unrotated_w, unrotated_h) = match rotation {
            90 | 270 => (rotated_h, rotated_w),
            _ => (rotated_w, rotated_h),
        };
        PageSpace {
            rotation,
            unrotated_w,
            unrotated_h,
        }
    }

    /// Map a single raw PDF point (y-up, unrotated media box) into rotated
    /// top-left (y-down) space. Derivation: convert to unrotated top-left
    /// (u=x, v=Hu-y), apply the clockwise page rotation, then substitute back.
    /// Result is unrounded — callers round the final derived values so that
    /// bbox extremes round exactly once.
    pub fn point(&self, x: f64, y: f64) -> (f64, f64) {
        let (wu, hu) = (self.unrotated_w, self.unrotated_h);
        match self.rotation {
            90 => (y, x),
            180 => (wu - x, y),
            270 => (hu - y, wu - x),
            // 0 (and any unexpected value): plain vertical flip, which keeps the
            // unrotated-page output byte-identical to the pre-rotation transform.
            _ => (x, hu - y),
        }
    }

    /// Map a raw PDF rect (`left/bottom/right/top`, y-up) into a rotated top-left
    /// axis-aligned `BBox`. Each 90°-multiple rotation maps axis-aligned rects to
    /// axis-aligned rects, so transforming the four corners and taking the
    /// extremes is exact.
    pub fn bbox(&self, left: f64, bottom: f64, right: f64, top: f64) -> BBox {
        let corners = [
            self.point(left, bottom),
            self.point(right, bottom),
            self.point(right, top),
            self.point(left, top),
        ];
        let mut x0 = f64::INFINITY;
        let mut y0 = f64::INFINITY;
        let mut x1 = f64::NEG_INFINITY;
        let mut y1 = f64::NEG_INFINITY;
        for (x, y) in corners {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        BBox {
            x0: round3(x0),
            y0: round3(y0),
            x1: round3(x1),
            y1: round3(y1),
        }
    }
}
