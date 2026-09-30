# L3 — CLI, SVG diagnostics, and gallery

Implement strict `grid`, `generate`, `gallery`, and `validate` subcommands. Emit canonical
RON, metrics, labelled composite/diagnostic SVGs, and an atomic twelve-seed HTML gallery.
Use patterns and text as well as a review-only palette. An aborted run must not leave an
incomplete destination: gallery publication stages beside an absent requested destination,
rejects an existing destination unchanged, and exposes the complete pack with one rename.
Do not make pixel inspection a logical oracle.
