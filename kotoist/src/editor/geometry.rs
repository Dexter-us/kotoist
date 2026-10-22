//! Logical editor dimensions, kept separate from the native window lifetime.

#[derive(Clone)]
pub(crate) struct EditorGeometry {
    pub scale: f64,
    preferred: Option<(f64, f64)>,
}

impl EditorGeometry {
    pub(crate) fn new(size: (u32, u32)) -> Self {
        Self {
            scale: 1.0,
            preferred: Some((size.0 as f64, size.1 as f64)),
        }
    }

    pub(crate) fn physical_size(&self, scale: f64) -> (u32, u32) {
        let logical = self.preferred.unwrap_or((560.0, 420.0));
        Self::constrain(
            (
                (logical.0 * scale).round() as u32,
                (logical.1 * scale).round() as u32,
            ),
            scale,
        )
    }

    pub(crate) fn logical_size(&self) -> (u32, u32) {
        let logical = self.preferred.unwrap_or((560.0, 420.0));
        (logical.0.round() as u32, logical.1.round() as u32)
    }

    pub(crate) fn constrain(size: (u32, u32), scale: f64) -> (u32, u32) {
        let min_width = (480.0 * scale).round().min(900.0) as u32;
        let min_height = (360.0 * scale).round().min(700.0) as u32;
        (
            size.0.clamp(min_width, 4096),
            size.1.clamp(min_height, 3072),
        )
    }

    pub(crate) fn remember(&mut self, size: (u32, u32), scale: f64) {
        self.preferred = Some((size.0 as f64 / scale, size.1 as f64 / scale));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_starts_compact_and_tracks_logical_size_at_hidpi() {
        let geometry = EditorGeometry::new((560, 420));
        assert_eq!(geometry.physical_size(1.0), (560, 420));
        assert_eq!(geometry.physical_size(1.5), (840, 630));
        assert_eq!(geometry.logical_size(), (560, 420));
    }

    #[test]
    fn a_user_resize_round_trips_at_fractional_dpi() {
        let mut geometry = EditorGeometry::new((560, 420));
        geometry.remember((1000, 701), 1.5);
        assert_eq!(geometry.physical_size(1.5), (1000, 701));
        assert_eq!(geometry.logical_size(), (667, 467));
        assert_eq!(EditorGeometry::constrain((1, 1), 1.0), (480, 360));
    }
}
