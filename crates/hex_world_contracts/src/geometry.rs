use crate::ContractError;
use serde::{Deserialize, Serialize};

/// Side length of an axial storage chunk; independent of regions and render batches.
pub const CHUNK_SIZE: i64 = 16;

/// Checked axial world coordinate. The derived cube coordinate is `-q-r`.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct WorldHex {
    /// First axial coordinate.
    pub q: i64,
    /// Second axial coordinate.
    pub r: i64,
}

impl WorldHex {
    /// Construct a horizontal coordinate without narrowing either axis.
    pub const fn new(q: i64, r: i64) -> Self {
        Self { q, r }
    }

    /// Translate without wrapping either stored axis.
    pub fn checked_add(self, offset: Self) -> Result<Self, ContractError> {
        let q = self
            .q
            .checked_add(offset.q)
            .ok_or_else(|| ContractError::new("coordinate", "q translation overflow"))?;
        let r = self
            .r
            .checked_add(offset.r)
            .ok_or_else(|| ContractError::new("coordinate", "r translation overflow"))?;
        Ok(Self { q, r })
    }

    /// Translate by an axial offset; alias for [`Self::checked_add`].
    pub fn translate(self, offset: Self) -> Result<Self, ContractError> {
        self.checked_add(offset)
    }

    /// Exact cube distance, using widened intermediates and rejecting a result over `u64`.
    pub fn checked_distance(self, other: Self) -> Result<u64, ContractError> {
        let dq = i128::from(self.q) - i128::from(other.q);
        let dr = i128::from(self.r) - i128::from(other.r);
        let distance = dq.abs().max(dr.abs()).max((dq + dr).abs());
        u64::try_from(distance).map_err(|error| ContractError::new("distance", error.to_string()))
    }

    /// Rotate counterclockwise in axial coordinates by `turns` multiples of 60 degrees.
    ///
    /// Intermediate arithmetic is widened so a valid final rotated coordinate is
    /// not rejected because a non-final axis would temporarily exceed `i64`.
    pub fn rotate_60(self, turns: u8) -> Result<Self, ContractError> {
        let mut q = i128::from(self.q);
        let mut r = i128::from(self.r);
        for _ in 0..turns % 6 {
            (q, r) = (-r, q + r);
        }
        Ok(Self {
            q: i64::try_from(q)
                .map_err(|error| ContractError::new("rotation.q", error.to_string()))?,
            r: i64::try_from(r)
                .map_err(|error| ContractError::new("rotation.r", error.to_string()))?,
        })
    }

    /// Storage chunk using Euclidean division, including negative coordinates.
    pub fn chunk(self) -> ChunkId {
        ChunkId::from_world_hex(self)
    }

    /// Local axial coordinates in the range `0..16`.
    pub fn local(self) -> (i64, i64) {
        (self.q.rem_euclid(CHUNK_SIZE), self.r.rem_euclid(CHUNK_SIZE))
    }

    /// Six adjacent columns in stable axial direction order; reject world-edge overflow.
    pub fn neighbors(self) -> Result<[Self; 6], ContractError> {
        Ok([
            self.checked_add(Self::new(1, 0))?,
            self.checked_add(Self::new(0, 1))?,
            self.checked_add(Self::new(-1, 1))?,
            self.checked_add(Self::new(-1, 0))?,
            self.checked_add(Self::new(0, -1))?,
            self.checked_add(Self::new(1, -1))?,
        ])
    }
}

/// Global storage chunk coordinate; not a region identity.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct ChunkId {
    /// Chunk coordinate on the first axial axis.
    pub q: i64,
    /// Chunk coordinate on the second axial axis.
    pub r: i64,
}

impl ChunkId {
    /// Resolve the unique chunk containing a world column.
    pub const fn from_world_hex(column: WorldHex) -> Self {
        Self {
            q: column.q.div_euclid(CHUNK_SIZE),
            r: column.r.div_euclid(CHUNK_SIZE),
        }
    }

    /// Lowest axial world coordinate in this chunk, rejecting invalid serialized IDs.
    pub fn origin(self) -> Result<WorldHex, ContractError> {
        Ok(WorldHex {
            q: self
                .q
                .checked_mul(CHUNK_SIZE)
                .ok_or_else(|| ContractError::new("chunk.q", "origin overflow"))?,
            r: self
                .r
                .checked_mul(CHUNK_SIZE)
                .ok_or_else(|| ContractError::new("chunk.r", "origin overflow"))?,
        })
    }
}

/// One voxel in an exact horizontal column; stacked surfaces retain their level.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct VoxelPosition {
    /// Horizontal column.
    pub column: WorldHex,
    /// Material voxel level, never the third horizontal cube coordinate.
    pub level: i32,
}
