//! Bounded dynamic native-prism projections for the Worm body.

use bevy_math::Vec3;
use serde::Serialize;

use crate::BodyHexPrism;

/// Largest compound body currently admitted by the arena (the seven-cell Golem).
pub const MAX_BODY_HEX_PRISMS: usize = 7;

/// Immutable observed component geometry, expressed relative to an actor's feet.
/// Its private constructor keeps physical queries finite and bounded.
#[derive(Debug, Clone, Copy)]
pub struct BodyPrismSnapshot {
    parts: [BodyHexPrism; MAX_BODY_HEX_PRISMS],
    count: u8,
}

impl BodyPrismSnapshot {
    pub(crate) fn try_from_parts(parts: &[BodyHexPrism]) -> Option<Self> {
        if parts.is_empty()
            || parts.len() > MAX_BODY_HEX_PRISMS
            || parts.iter().any(|part| {
                !part.offset.is_finite() || !part.height.is_finite() || part.height <= 0.0
            })
        {
            return None;
        }
        let mut result = Self {
            parts: [BodyHexPrism {
                offset: Vec3::ZERO,
                height: 0.0,
            }; MAX_BODY_HEX_PRISMS],
            count: u8::try_from(parts.len()).ok()?,
        };
        result.parts.get_mut(..parts.len())?.copy_from_slice(parts);
        let (low, high) = result.bounds();
        (low.is_finite()
            && high.is_finite()
            && (high - low).is_finite()
            && (high - low).cmpgt(Vec3::ZERO).all())
        .then_some(result)
    }

    /// Exact components in stable head-first order; no allocation or live lookup.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = BodyHexPrism> + '_ {
        self.parts.iter().take(usize::from(self.count)).copied()
    }

    pub(crate) fn packed(self) -> ([BodyHexPrism; MAX_BODY_HEX_PRISMS], usize) {
        (self.parts, usize::from(self.count))
    }

    /// Actual union bounds relative to the actor anchor, including horizontal offset.
    pub(crate) fn bounds(self) -> (Vec3, Vec3) {
        let half = hex_core::config::HEX_SMALL_DIAMETER * 0.5;
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for part in self.iter() {
            low = low.min(part.offset - Vec3::new(half, 0.0, 1.0));
            high = high.max(part.offset + Vec3::new(half, part.height, 1.0));
        }
        (low, high)
    }

    pub(crate) fn center_offset(self) -> Vec3 {
        let (low, high) = self.bounds();
        low * 0.5 + high * 0.5
    }
}

/// Physical Worm phase; it contains no target, memory, or movement destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WormPhase {
    /// Hidden or moving on the admitted shallow soil band.
    Travel,
    /// The leading body components are physically rising.
    Emerging,
    /// The head is raised while looking or attacking.
    Exposed,
    /// Leading components are returning to the shallow travel band.
    Diving,
}

/// Authority-owned rendering/evidence state; never a human enemy-location marker.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct WormSnapshot {
    /// Current body phase.
    pub phase: WormPhase,
    /// Stable head component index, always zero for this body.
    pub head_index: u8,
    /// Lowest head face minus the current resolved local surface height.
    pub head_clearance: f32,
    /// Complete head clear and at least one voxel level above that surface.
    pub exposed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct WormBodyState {
    pub(crate) current: BodyPrismSnapshot,
    pub(crate) previous: BodyPrismSnapshot,
    pub(crate) bounds_min: Vec3,
    pub(crate) bounds_max: Vec3,
    pub(crate) snapshot: WormSnapshot,
}

impl WormBodyState {
    pub(crate) fn straight(count: u8, heading: f32) -> Option<Self> {
        if !matches!(count, 4 | 6) || !heading.is_finite() {
            return None;
        }
        let behind = bevy_math::Quat::from_rotation_y(heading) * Vec3::Z;
        let mut parts = [BodyHexPrism {
            offset: Vec3::ZERO,
            height: 0.4,
        }; MAX_BODY_HEX_PRISMS];
        for (index, part) in parts.iter_mut().take(usize::from(count)).enumerate() {
            let step = f32::from(u8::try_from(index).ok()?);
            part.offset = behind * hex_core::config::HEX_SMALL_DIAMETER * step;
        }
        Self::observed(BodyPrismSnapshot::try_from_parts(
            parts.get(..usize::from(count))?,
        )?)
    }

    pub(crate) fn parts_at(count: u8, heading: f32, lift: f32) -> Option<BodyPrismSnapshot> {
        if !lift.is_finite() || lift < 0.0 {
            return None;
        }
        let body = Self::straight(count, heading)?;
        let mut parts: Vec<_> = body.current.iter().collect();
        let denominator = f32::from(count - 1);
        for (i, part) in parts.iter_mut().enumerate() {
            part.offset.y = lift * (1.0 - f32::from(u8::try_from(i).ok()?) / denominator);
        }
        BodyPrismSnapshot::try_from_parts(&parts)
    }

    pub(crate) fn update(&mut self, current: BodyPrismSnapshot) -> Option<()> {
        if current.iter().len() != self.current.iter().len()
            || current
                .iter()
                .zip(self.current.iter())
                .any(|(a, b)| a.height.to_bits() != b.height.to_bits())
        {
            return None;
        }
        let (low, high) = current.bounds();
        self.current = current;
        self.bounds_min = low;
        self.bounds_max = high;
        Some(())
    }

    pub(crate) fn observed(current: BodyPrismSnapshot) -> Option<Self> {
        if !matches!(current.iter().len(), 4 | 6)
            || current
                .iter()
                .any(|part| (part.height - 0.4).abs() > crate::collision::SKIN)
        {
            return None;
        }
        let (bounds_min, bounds_max) = current.bounds();
        Some(Self {
            current,
            previous: current,
            bounds_min,
            bounds_max,
            // A copied shape does not invent a live movement/attack phase.
            snapshot: WormSnapshot {
                phase: WormPhase::Emerging,
                head_index: 0,
                head_clearance: 0.0,
                exposed: false,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_constructor_rejects_unbounded_and_nonphysical_components() {
        let valid = BodyHexPrism {
            offset: Vec3::ZERO,
            height: 0.4,
        };
        assert!(BodyPrismSnapshot::try_from_parts(&[]).is_none());
        assert!(BodyPrismSnapshot::try_from_parts(&[valid; 8]).is_none());
        for bad in [
            BodyHexPrism {
                offset: Vec3::NAN,
                ..valid
            },
            BodyHexPrism {
                height: 0.0,
                ..valid
            },
            BodyHexPrism {
                height: f32::INFINITY,
                ..valid
            },
        ] {
            assert!(BodyPrismSnapshot::try_from_parts(&[bad]).is_none());
        }
        assert_eq!(
            BodyPrismSnapshot::try_from_parts(&[valid; 7])
                .expect("bounded")
                .iter()
                .len(),
            7
        );
    }

    #[test]
    fn current_and_previous_observed_geometry_are_identical_and_center_is_offset() {
        let body = WormBodyState::straight(4, 0.0).expect("four components");
        let mut parts: Vec<_> = body.current.iter().collect();
        for (index, part) in parts.iter_mut().enumerate() {
            let index = f32::from(u8::try_from(index).expect("bounded"));
            part.offset.y = 1.2 * (1.0 - index / 3.0);
        }
        let observed =
            WormBodyState::observed(BodyPrismSnapshot::try_from_parts(&parts).expect("snapshot"))
                .expect("Worm");
        assert!(observed
            .current
            .iter()
            .zip(observed.previous.iter())
            .all(|(a, b)| a.offset == b.offset && a.height.to_bits() == b.height.to_bits()));
        let center = observed.current.center_offset();
        assert!(center.z > 2.5 && (center.y - 0.8).abs() < 0.0001);
        assert!(!observed.snapshot.exposed);
    }
}
