# eyepiece mark

not chosen yet. the owner asked for a mark that sits with the product icons on tridant.io and stands on its own
(2026-11-23). three rounds have been made. the drawings are in `concepts/`, the sheets beside this file.

- `sheet-render.png`, `eyepiece-render.png`, `3d/eyepiece.py`: round three, F rendered in blender (below).
- `sheet-lens.png`, `sheet-lens-large.png`: round two, the lens (below). six directions, each as an app icon dark and
  light from 256 to 16 px, and its flat mark between starlight and owlette; and the six app icons large.
- `sheet-family.png`: each of round one's finalists placed in the site's own products strip, light and dark, and at
  112 px beside the seven product marks.
- `sheet-sizes.png`: each of round one's finalists at 256, 64, 32, 24 and 16 px, app icon and flat mark, dark and
  light, with the 16 and 32 px drawings enlarged so their pixels can be judged.
- `sheet-all-six.png`: the first six concepts.
- `family.md`: what the seven product marks have in common, measured from their drawings.

## the family, in five lines

1. a 48 x 48 canvas with no container; the mark sits in a circle of about 21 across the middle. 36 px in the strip.
2. one idea a mark, built from a formula or a grid, as a solid fill or one stroke.
3. one tone a product. six of the seven silhouettes are round.
4. each mark nods to its name in geometry, never as a letter.
5. the desktop app icon follows starlight's: a rounded body on paper or night, with 16, 24 and 32 px layers drawn
   by hand.

## round three: F, rendered (2026-11-24)

the owner took F, asked for it a little more straight on, and for a render of it in blender: a product shot, exact,
well lit, readable very small and very large.

- `3d/eyepiece.py` builds and renders it: `blender --background --factory-startup --python 3d/eyepiece.py -- --out
  x.png --size 2048 --samples 512 --tilt 40` (blender 5.0; 19 s for 2048 px on this pc's gpu). every part is a lathe
  profile in the script. `eyepiece-render.png` is its picture at 40 degrees, on nothing. `sheet-render.png` sets it
  in the app icon's body at every size, with two other angles, an all-black finish and the two engines side by side.
- the engine: cycles. blender has two, and in the same scene eevee, the real-time one, shows the dark glass gray and
  the orange washed out (the sheet's last row). no other engine is installed, and none was added.
- the angle: 40 degrees off the axis. the drawn F was at 53; 33 and 47 are on the sheet.
- the finish: a bright turned bezel, a black ribbed grip, a chrome barrel. all black, as most eyepieces are, it is
  lost on the app icon's dark body but for its edges (on the sheet).
- the glass shows two things and no more, as the drawing did: a light along its upper left edge and the one orange
  low on the right, which came out as #eb5231 where the player's is #ff5a36. each is a light placed where that part
  of the glass looks, and lights nothing else.
- what is not exact: only the barrel's 31.75 mm is a standard's, the rest is a plossl's proportions by eye; the glass
  is its upper surface alone, since a whole lens showed every light three times over; the grip's ribs are shading,
  not shape; nothing is engraved.
- small: from 48 px down the render made small is a ring and a blur, as the sheet shows. it stands for 64 px and up;
  16 to 48 still want layers drawn for their size.

## round two: the lens (2026-11-24)

the owner on round one: none of them, "too bland and generic". pixel field is the one liked most: minimal, simple,
and its colors are right. the brief that came with it: draw an actual eyepiece, simplified, perhaps looked towards,
with sheen and a reflection on its glass, as when looking through a telescope.

six directions (`concepts/l1` to `l6`), over three choices: looked at down the barrel or turned a little away; the
sheen as flat shapes or as glass; dark glass or the picture behind it. the colors are pixel field's: the dark body,
the light field, orange once. in all of them the light on the glass is a sliver across the upper left, and where
the glass is dark its ghost from the back surface, low on the right, is the one orange.

| | what it is | for | against |
|---|---|---|---|
| **A flat glass** | the eyecup's ring and dark glass, the sheen as two flat shapes | the plainest; four shapes | a lens like any camera's; the sliver reads as a moon; a ring in a ring, as starlight is |
| **B soft glass** | A as glass: a lit ring, the retaining ring with its two slots, glass that fades | looks like optics | the nearest to owlette: a warm glow in a dark round; nothing of it survives below 32 px |
| **C pixels through flat glass** | pixel field seen through the eyepiece: its grid dim behind the glass, the pixel looked at lit | keeps the mark the owner liked and says what the program is for | busier than A; its flat mark can read as a globe |
| **D pixels through soft glass** | C as glass, dark towards the edge, the lit pixel glowing | the most "looking into it" | as C; the glow is orange twice over |
| **E the eyepiece, flat** | the thing itself, turned away: eyecup, grip, chrome sleeve, in flat tones | the only one that is an eyepiece and nothing else; no round neighbor to be taken for | not round, where six of the family are; a blur at 16 px |
| **F the eyepiece, lit** | E with its body and sleeve lit | reads as an object at once | an illustration more than a mark; the hardest to draw small |

every one of them needs its 16, 24 and 32 px layers drawn by hand: the small ones on the sheet are the large drawing
made small, and show what is lost.

## round one: the finalists

| | idea | for | against |
|---|---|---|---|
| **graticule** | an eyepiece's measuring scale read as a timeline: every tick a frame, the tall one is now | the cleverest: lens, frame-exact stepping and measuring in one figure | the busiest at 16 px; near constellation's crosshair; light in the strip |
| **pixel field** | an eyepiece's round field over a pixel grid; the middle pixel is the one under inspection | crisp at every size, the only grid in the family; the app icon is a neutral field with the accent used once, as the player's own rule has it | can read as a plain grid ball; the heaviest in the strip |
| **reticle e** | a lens with one hairline across it is a lowercase e | instantly read; owns the initial | generic; the family never uses letters |

## open with the owner

- the render of F: the angle, the finish, and what it still lacks.
- its tone on the site. the player's orange (#ff5a36) reads as a second transmission there, and graphite as
  "disabled"; violet (#7d3fb8 light, #b384ea dark) is the one hue the family does not use. orange stays as the
  single accent inside the app icon.

## still to do for the one chosen

a look for marks too close to it; 16, 20, 24, 32, 40 and 48 px layers drawn for their size; the windows .ico and
the macos .icns; the name set beside it (archivo semibold, lowercase); a look at it in a real taskbar at 100, 125
and 150%.
