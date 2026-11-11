//! Picture-in-picture: the main window shrinks into a small, borderless,
//! always-on-top player and later returns to its previous size and position.
//! This module holds the pure geometry; the window commands are sent by `App`.

use egui::{Pos2, Rect, Vec2, pos2, vec2};

/// Size of the PiP window the first time (16:9).
pub const DEFAULT_SIZE: Vec2 = vec2(400.0, 225.0);
/// The PiP window cannot be resized below this.
pub const MIN_SIZE: Vec2 = vec2(240.0, 135.0);
/// Minimum size of the normal window (the sidebar needs room).
pub const NORMAL_MIN_SIZE: Vec2 = vec2(640.0, 400.0);
/// Fallback when the window size is not known yet (e.g. PiP right at start-up).
pub const FALLBACK_NORMAL_SIZE: Vec2 = vec2(1000.0, 640.0);

/// The normal window's geometry, kept while in PiP.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Saved {
    pub inner_size: Vec2,
    pub outer_pos: Option<Pos2>,
}

pub fn clamp_size(size: Vec2) -> Vec2 {
    vec2(size.x.max(MIN_SIZE.x), size.y.max(MIN_SIZE.y))
}

/// Where to put the PiP window: it keeps the bottom-right corner of the
/// normal window, which is the classic PiP spot and always on a visible screen.
pub fn pip_position(normal_outer: Option<Rect>, pip_size: Vec2) -> Option<Pos2> {
    normal_outer.map(|r| pos2(r.max.x - pip_size.x, r.max.y - pip_size.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_keeps_the_bottom_right_corner() {
        let outer = Rect::from_min_size(pos2(100.0, 50.0), vec2(1000.0, 700.0));
        let pos = pip_position(Some(outer), vec2(400.0, 225.0)).unwrap();
        assert_eq!(pos, pos2(700.0, 525.0));
        assert_eq!(pip_position(None, DEFAULT_SIZE), None);
    }

    #[test]
    fn size_is_clamped_to_the_minimum() {
        assert_eq!(clamp_size(vec2(100.0, 50.0)), MIN_SIZE);
        assert_eq!(clamp_size(vec2(800.0, 450.0)), vec2(800.0, 450.0));
        assert_eq!(clamp_size(vec2(100.0, 900.0)), vec2(MIN_SIZE.x, 900.0));
    }

    #[test]
    fn default_size_is_16_by_9() {
        assert!((DEFAULT_SIZE.x / DEFAULT_SIZE.y - 16.0 / 9.0).abs() < 0.01);
    }
}
