# Grand forest artwork

`trees.ron` contains exact compact occupancy for twelve accepted Forest–Massif
expedition trees and three broader Grand grove trees. It is compiler content;
rendered, colliding and destructible geometry share these occupied runs.

The reference is `assets/config/v4/forest-massif/expedition/`, retained in the
successful `hex-expedition` checkout at `b314a9d`. Its integrated artwork commit is
`e8ecd0ab5bd29e348a84614b7243a655dc1a5e54`. The twelve stock templates preserve every
voxel and all three foliage style bands. The new grove templates reuse that same
`tools/forest_trees.py` sculptor with explicit heights, radii and seeds in
`provenance.json`: 80/97/114 levels and 12/14/16 hex radii. Their respective occupied
cell counts are 7,526, 12,076 and 18,742; each stays below the existing 65,536-cell
artwork bound. The dominant World Tree remains its separate exact authored object.

Reproduce or verify from the repository root:

```sh
python3 assets/config/v4/grand-v4/forest/generate.py
python3 assets/config/v4/grand-v4/forest/generate.py --write
```

Source hashes must match before either operation. Stock shape and rotation tests
also compare the expanded runtime templates directly against the accepted artwork.
Changing a source intentionally requires an explicit provenance update and fresh
visual review; the helper does not silently adopt new art.

Dressing uses the current compiler surface, preserves the original terrain,
extends existing woody roots down to their exact contacts, and rejects steep
foundations, solid overlaps and occupied gameplay clearance. Mature shapes are
placed before the understory. Seeded groves, irregular edges and named glades
replace the former fixed grid. Limits are 780 ordinary/frozen trees and 60 small
forest-floor details, within the complete composition's 900-object limit.

Foliage shade names carry the same gameplay policy as ordinary foliage. They are
three opaque colors, not new gameplay substances or render-only air.

Fresh full-footprint, forest approach, reverse, ground-level and World Tree views
remain required on the combined terrain/dressing candidate. Source validation or
canopy coverage measurements alone do not grant presentation approval.
