pub mod flower;
pub mod hex;
pub mod merkaba;
pub mod metatron;
pub mod phi;
pub mod seed;
pub mod star;
pub mod tree;
pub mod vesica;
pub mod yantra;

#[cfg(test)]
mod tests;

use crate::geom::build::Build;
use crate::geom::construction::Construction;
use crate::params::Params;

pub struct FigureDef {
    pub name: &'static str,
    pub build: fn(&mut Build, &Params),
    /// Which seed settings actually change this figure — the HUD only offers
    /// the knobs that do something.
    pub knobs: &'static [Knob],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Knob {
    Symmetry,
    Rings,
    Ratio,
    Skip,
    Twist,
}

const LATTICE: &[Knob] = &[Knob::Symmetry, Knob::Rings, Knob::Ratio, Knob::Twist];
const ROSETTE: &[Knob] = &[Knob::Symmetry, Knob::Ratio, Knob::Twist];
const STARRY: &[Knob] = &[Knob::Symmetry, Knob::Skip, Knob::Ratio, Knob::Twist];

pub const FIGURES: &[FigureDef] = &[
    FigureDef { name: "Vesica Piscis", build: vesica::vesica, knobs: &[Knob::Ratio, Knob::Twist] },
    FigureDef { name: "Seed of Life", build: seed::seed_of_life, knobs: ROSETTE },
    FigureDef { name: "Egg of Life", build: seed::egg_of_life, knobs: ROSETTE },
    FigureDef { name: "Flower of Life", build: flower::flower_of_life, knobs: LATTICE },
    FigureDef { name: "Fruit of Life", build: flower::fruit_of_life, knobs: ROSETTE },
    FigureDef { name: "Metatron's Cube", build: metatron::metatron, knobs: ROSETTE },
    FigureDef { name: "Hexagram", build: star::star_polygon, knobs: STARRY },
    FigureDef {
        name: "Pentagram",
        build: star::pentagram,
        knobs: &[Knob::Ratio, Knob::Twist],
    },
    FigureDef { name: "Star Tetrahedron", build: merkaba::merkaba, knobs: ROSETTE },
    FigureDef { name: "64 Tetrahedron Grid", build: merkaba::grid_64, knobs: LATTICE },
    FigureDef { name: "Golden Spiral", build: phi::golden_spiral, knobs: &[Knob::Rings] },
    FigureDef { name: "Vector Equilibrium", build: merkaba::vector_equilibrium, knobs: ROSETTE },
    FigureDef { name: "Sri Yantra", build: yantra::sri_yantra, knobs: &[Knob::Twist] },
    FigureDef { name: "Tree of Life", build: tree::tree_of_life, knobs: &[Knob::Twist] },
];

pub fn build_with(i: usize, p: &Params) -> Construction {
    let def = &FIGURES[i % FIGURES.len()];
    let p = p.clamped();
    let mut b = Build::new();
    (def.build)(&mut b, &p);
    b.finish(def.name)
}

pub fn build(i: usize) -> Construction {
    build_with(i, &Params::default())
}
