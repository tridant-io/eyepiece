# the tridant product mark family, as found (2026-11-24)

Sources: the live products section of https://tridant.io, rendered with Playwright (`scrape.json`, `site-products-section.png`, `site-manifest-strip*.png`). The marks' source is `src/components/ProductGlyph.astro`, `Products.astro` and `ProductVisual.astro` in `Documents\Git\tridant.io`, and the deployed DOM matches it. The real app icons come from the product repos (`app-icons/`). Measurements come from the SVGs (`measure.json`) and from PNG pixel scans.

## Bad news first: there are two systems, not one

1. **Site glyphs.** Seven marks on a bare 48×48 grid, each in one product tone, with no container. Five are vector. Two (owlette, moonshine) ship as painted rasters.
2. **App icons.** These disagree with each other:
   - starlight: macOS squircle.
   - Latticus: rounded square, rx 20%.
   - phase plot: rounded square, rx 22%.
   - owlette and moonshine: bare circles.

I follow the site-glyph rules for eyepiece's flat mark. For the app icon I follow starlight's, because it is the only native desktop app icon built in the current brand and it ships a full `.ico` and `.icns`.

## The site glyphs

| product | site tone light / dark | what it is | construction | ink box on the 48 grid |
|---|---|---|---|---|
| Transmission | `#ff3d2e` / `#ff4a38` | the brand gear, "ST 2110 · SMPTE" engraved round the hub | one solid evenodd path: 16 sin²-profile teeth (22.5° pitch), bore r≈4.8; text on r 12.6 at 3.6 units | 2.62–45.38 (r≈21.4) |
| Constellation | `#2563d9` / `#4b82e8` | the pixel reticle from its web UI, ×3 | 6 rects on a **3-unit grid**, `crispEdges`: cross 33×3 and 3×33, hub 9×9, three "stars" 6, 3, 3 | x 0–45, y 9–42 |
| Moonshine | `#5d7590` / `#90a8c6` (glow only) | Méliès moon with a **projector lens in its eye** | raster, 481 px painted disc | full disc |
| Starlight | `#a87b16` / `#d9a83a` (glow only) | a star at the centre of rings | plate disc r 21 `#0a0e1a`, rings 0.8 stroke in plate ink (r 6.2 / 10.4 / 14.6 / 18.8 when static), superformula star (m 4, n 0.3) | 3–45 (r 21) |
| Latticus | `#6f6855` / `#8f8a75` ("in development, muted") | a woven lattice line | epitrochoid x = 7cos t − 6cos 7t, stroke 1.7, round caps, gradient along the line in 90 segments (red, gold, slate, blue) | x 5.0–43.0, y 4–44 |
| owlette | `#b05a28` / `#d97f42` (glow only) | the owl's eye | raster orb, radial gradient in a black rim `#0a0604`, r 88/100 (≈21 on 48) | ≈3–45 |
| phase | `#1e6e54` / `#37a583` | sin and cos, half a period apart | two polar curves r = 15.2 + 3.4·sin(4θ + {0, π}), stroke 1.8, tone + site accent red | 5.5–42.5 |

- **Canvas and safe area.**
  - viewBox `0 0 48 48`, no container.
  - The live area is a circle of r≈21 around (24, 24), leaving a 3-unit (6%) margin. phase sits smaller (r≈18.5).
  - Shown at 36 px in the sticky products strip (`h-9`, 80 px slot) and at 176 px in the register (`h-44`), with a tone-coloured blur glow behind.
- **Weights.**
  - Line marks use one stroke each: 1.7 (Latticus) and 1.8 (phase), about 3.5–4% of the canvas, or 1.3 px at 36 px.
  - Solid marks are filled: the gear, the constellation rects, the plate disc.
  - Hairlines appear only inside a dark plate (starlight's 0.8 rings).
- **Shape count.** One idea and few shapes: gear 1 path, constellation 6 rects, phase 2 curves, starlight disc + rings + star. Every mark is generated: a formula (superformula, epitrochoid, polar sine, sin² teeth) or a grid (constellation).
- **Colour.**
  - One "program insignia" tone per product (`--color-p-*` in `app.css`, a light/dark pair). It drives `currentColor` and the glow.
  - Exceptions: phase adds the site accent red, Latticus runs a four-colour gradient, starlight draws in plate colours, and the rasters carry their own.
- **Names.**
  - In the register: Archivo semibold (600), tracking-tight (−0.025em), 34 px, olive ink `#3d4419` (dark `#e6ddc4`). Transmission and Latticus use drawn wordmarks (Latticus's is lowercase); owlette is in Geist semibold.
  - In the strip: IBM Plex Mono 9 px, tracking 0.14em, dim `#6f6855` (dark `#7e8aa8`).
  - Case follows the brand: lowercase owlette and phase (and tridant); capitalised Transmission, Constellation, Moonshine, Starlight, Latticus.
- **Mark and name.** Every mark is a nod to the name or the job, drawn as geometry rather than illustrated: a gear for transmission, stars for constellation, the moon for moonshine, a star's light in rings, a lattice, an owl's eye, two waves in phase. Some are puns, none is a letter, and only the rasters are pictures.
- **What makes them a family.**
  - centred, radial, round silhouettes (six of seven)
  - one tone each
  - built from a formula or a pixel grid
  - celestial and instrument vocabulary (star, moon, constellation, gear, dial)
  - set bare on the paper

## The app icons (outside the site)

- **starlight** (`starlight/resources`)
  - 1024 canvas with an 824 superellipse body (n≈5, 100 px margin: the macOS template). The mark disc is 658 px (64% of the canvas, 80% of the body).
  - Light: body `#f0eadb` paper, disc `#0a0e1a`, rings `#f0eadb`.
  - Dark: body `#0d1220`, disc `#06080f`, rings `#dbe6fa`.
  - The `.ico` has 16, 20, 24, 32, 40, 48, 64, 96, 128 and 256. The **16, 24 and 32 layers are hand-drawn pixel art in a full-bleed rounded square with a 1 px margin**, rings reduced from four to two at 16 (`app-icons/starlight-ico-16.png`).
- **Latticus** (cms `icon.svg`): 512 rounded square, rx 104 (20.3%), `#0b1020`, with the "l" from the wordmark in `#fafafa`.
- **phase plot** (favicon): 64 rounded square, rx 14 (21.9%), `#0B0B0E`, grey rings and three coloured dots. phase's patch uses an embroidered "P" patch photo.
- **owlette**: the orb, no container. `.ico` 16–256.
- **moonshine**: the painted moon disc. `.ico` 16–256.
- **transmission.video**: the gear in `#ff3d2e` on transparent.
- **tridant favicon**: dark disc r 48/100 `#0a0e1a`, trident star stroked `#cfe0ff`.

## What none of them does (the open ground)

- Nothing on the site is a letter. Only app-icon outliers use one: Latticus "l", patch "P".
- No media glyph: no play triangle, no frame or timeline.
- No square or rectilinear silhouette. constellation is a cross, not a box.
- No measuring scale or ticks.
- Nothing is violet or purple. The free hues are violet and magenta.
- Nobody uses the player's orange `#ff5a36`. It sits between transmission's red `#ff3d2e` and owlette's amber `#b05a28`, so it is effectively taken.

## Motifs already owned (collision table)

| motif | owner |
|---|---|
| eye / orb | owlette |
| concentric rings | starlight, phase plot favicon |
| pixel crosshair reticle | constellation |
| a literal eyepiece / lens over an eye | moonshine's face (the projector monocle) |
| lens "eyes" | the tridant star (three lens eyes that blink on the hero) |

An eyepiece concept that is a ring-in-ring, a crosshair or an eye has to prove at 16 px that it is not one of these.

## The brief I design to

- **Flat mark**
  - 48-unit grid, live circle r 21, one colour through `currentColor`.
  - Built from few shapes on the 3-unit (16 px) and 1.5-unit (32 px) grids.
  - Solid fills or 3-unit strokes, since family hairlines vanish below 32 px.
  - A nod to the job found in the geometry.
  - Its own tone, which must not be orange.
- **App icon:** starlight's template.
  - macOS 1024 canvas, 824 superellipse body, mark at about 64% of the canvas.
  - A dark body (the player's `#161616`) and a light paper body (`#f0eadb`).
  - 16, 24 and 32 drawn by hand, full bleed with a 1 px margin.
