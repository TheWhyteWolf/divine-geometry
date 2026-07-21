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

## Figures

Vesica Piscis, Seed of Life, Egg of Life, Flower of Life, Fruit of Life,
Metatron's Cube, Hexagram, Pentagram, Star Tetrahedron, 64 Tetrahedron Grid,
Golden Spiral (φ solved by compass swing), Vector Equilibrium, Sri Yantra,
Tree of Life — plus two endless modes: an ever-growing Flower lattice the
camera pulls back through, and an Apollonian gasket it descends into forever
(Descartes' theorem per circle).

Symmetry is a live parameter: the compass walk closes for any order because the
span is set to the inscribed N-gon's side, so a sevenfold Seed of Life is a
correct sevenfold construction, not a stretched hexagon.

## Licence

GPL-3.0-or-later.
