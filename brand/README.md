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
