# divine

Sacred geometry drawn by compass and straightedge — white (or coloured) glowing
lines on black, each figure genuinely *constructed*: every circle is centred on
a point solved from the intersections of previous circles, never on a stored
coordinate. The one deliberate exception is the Sri Yantra's triangle
proportions, which are a classical offline solve (perfect concurrency is
provably impossible — Kulaichev, 1984); its marma points and lotus petals are
still solved live.

Rust + wgpu (Vulkan/Metal/DX12), no UI toolkit — the readout is drawn with the
same stroke pipeline as the figures.

Two optional shader layers sit either side of the geometry: a generated field
behind it, and a screen-space fold in front. Both default to off, and with them
off the render is exactly the drawing it always was.

## Run

```
cargo run --release
```

Opens clean with no UI. `H` shows the readout; press again for the key panel.

Headless verification stills (no window, reproducible):

```
cargo run --release -- --shot out.png --figure 5 --ft 1.0 --settled 1.0 \
    --palette 7 --snow 1 --marks 1 --seed 42 --sym 9 --hud 1
cargo run --release -- --verify     # build every figure, print step counts
```

## Controls

| | |
|---|---|
| `H` | readout → key panel → hidden |
| mouse | click a value = next, right-click = back, wheel = adjust |
| `Space` `←` `→` `↑` `↓` | pause · step construction · speed |
| `N` `P` `R` | next / previous figure, restart |
| `A` `S` `X` | autoplay · scaffold visibility · endless modes |
| `,` `.` `;` `'` `9` `0` `K` `L` `O` `I` | symmetry · rings · ratio · skip · twist |
| `G` `C` | new seed · reset classical |
| `V` `T` `Y` `U` | palette · hue ∓ · saturation |
| `W` `M` | snow · compass marks |
| `[` `]` `-` `=` `F` | shimmer · bloom · fullscreen |
| `B` `E` `J` | background · foreground · trails |
| `1` `2` `3` `4` `5` `6` `Q` | bg gain · fg amount · trail length · trail flow |

## Figures

Vesica Piscis, Seed of Life, Egg of Life, Flower of Life, Fruit of Life,
Metatron's Cube, Hexagram, Pentagram, Star Tetrahedron, 64 Tetrahedron Grid,
Golden Spiral (φ solved by compass swing), Vector Equilibrium, Sri Yantra,
Tree of Life — plus two endless modes: an ever-growing Flower lattice the
camera pulls back through, and an Apollonian gasket it descends into forever
(Descartes' theorem per circle).

## Layers

Off by default — the construction is the front door, and nothing below is
running until you ask for it.

**Background** (`B`) paints behind the figure: `PLASMA` (a domain-warped noise
marble), `TUNNEL`, `KALEIDO`, and two escape-time fractals, `KALI` and `JULIA`.
The field takes its hue from the live palette, so it grounds the figure instead
of arguing with it, and it holds the centre back so the drawing stays legible.

**Foreground** (`E`) remaps the composed frame: `WARP`, `KALEIDO`, `TUNNEL`,
`DROSTE`, `VORTEX`, each with a chromatic split on the fetch. It folds the
background and the geometry together as one image — a mirrored figure sits in a
mirrored field.

**Trails** (`J`) are temporal feedback: `SOFT`, `LONG`, and `FLOW`, which adds a
rotate/zoom to each echo so a still figure grows a receding spiral of itself.
Trail length is wall-clock, not frame-count — the constants are corrected
against the frame delta, so the effect is the same at 30 Hz and 144 Hz.

Every layer is also a token in the readout: click, right-click and the wheel
adjust it, and each layer's knobs appear only while that layer is live. Stills
take the same settings — `--bg`, `--bggain`, `--fg`, `--fgamt`, `--trails`,
`--traillen`, `--trailflow` — and a trail still replays the construction up to
the requested moment, so it shows a real trail rather than one static frame.

Symmetry is a live parameter: the compass walk closes for any order because the
span is set to the inscribed N-gon's side, so a sevenfold Seed of Life is a
correct sevenfold construction, not a stretched hexagon.

## Licence

GPL-3.0-or-later.
