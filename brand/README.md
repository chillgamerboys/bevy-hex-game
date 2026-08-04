# Hex brand assets

The SVG files in this directory are the editable source of truth for the Hex
wordmark and compact mark. Both are built from the same pointy-top hexagon used
throughout the game, using the runtime UI palette:

- gold: `#edc975`
- charcoal: `#1a1c24`

Bevy consumes raster exports from `assets/ui/`. On macOS, regenerate the checked-in
PNGs without changing their authored dimensions with:

```sh
sips -s format png brand/hex-logo.svg --out assets/ui/hex-logo.png
sips -s format png brand/hex-mark.svg --out assets/ui/app-icon.png
```

Keep the SVGs when adding other raster sizes. The wordmark has a transparent
background; the compact mark has a transparent square canvas around its outlined
hexagonal badge.

## Element glyphs

`brand/elements/` contains the editable source of truth for the 18 elemental
glyphs. Each master uses a `0 0 256 256` view box, a transparent canvas, rounded
16-unit strokes, and normalized optical margins. The white geometry is intentional:
the runtime PNG is an alpha-backed tint mask, so Bevy can apply each element's
authored color without a custom shader. Preview the masters against a dark canvas
when editing them.

The glyph geometry follows the approved hand-drawn elemental-grid sketch. Preserve
the recognizable silhouettes and internal negative space when refining a mark;
normalize related marks as a set instead of substituting unrelated icon-library art.

On macOS, regenerate every checked-in 256×256 runtime PNG with:

```sh
for svg in brand/elements/*.svg; do
  name="${svg##*/}"
  sips -s format png "$svg" --out "assets/ui/elements/${name%.svg}.png"
done
```

Validate XML, the exact SVG view box, and PNG format/dimensions/alpha with:

```sh
xmllint --noout brand/elements/*.svg
grep -L 'viewBox="0 0 256 256"' brand/elements/*.svg
sips -g pixelWidth -g pixelHeight -g format -g hasAlpha assets/ui/elements/*.png
file assets/ui/elements/*.png
```

`grep -L` must print nothing. Every `sips` record must report width and height 256,
PNG format, and alpha; `file` must report 8-bit/color RGBA. `sips` is the established
local export path, not a claim of cross-platform or CI-deterministic rasterization.

To audit checked-in exports against a clean local regeneration:

```sh
element_audit_dir="$(mktemp -d /tmp/hex-element-exports.XXXXXX)"
for svg in brand/elements/*.svg; do
  name="${svg##*/}"
  sips -s format png "$svg" --out "$element_audit_dir/${name%.svg}.png" >/dev/null
  cmp "assets/ui/elements/${name%.svg}.png" "$element_audit_dir/${name%.svg}.png"
done
```

The loop must finish without `cmp` output or a nonzero exit status. The temporary
directory can be discarded after the audit.
