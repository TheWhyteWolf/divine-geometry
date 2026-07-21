//! On-screen readout, drawn with the app's own stroke pipeline — and clickable.
//!
//! A single-stroke vector font rather than a UI toolkit: the same white glowing
//! line as everything else, which is the point. The glyphs live on a 4×8 grid
//! (cap height 8) with generous tracking and a hairline weight — closer to a
//! drafting stencil than the old 5×7 terminal block.
//!
//! Interactivity is hit regions, not widgets. `build` records a pixel rect for
//! every token that carries an action; the app resolves clicks and hovers
//! against those rects. A hovered token's strokes are assigned to a second,
//! brighter step slot — the highlight costs one extra 32-byte entry, no second
//! draw call.
//!
//! Text draws through a second scene uniform holding an identity camera (so it
//! stays pixel-locked while the figure zooms) with `shimmer = 0` — a glyph
//! stroke is a few pixels long, so shimmer would flicker it as a whole unit.

use glam::Vec2;

use crate::tess::Tess;

/// Glyph cell: 4 wide, 8 tall (cap height), origin bottom-left, y up.
const W: f32 = 4.0;
const ADVANCE: f32 = 6.5;
const LINE: f32 = 12.5;
/// Hairline weight — the text should whisper, not declaim.
const WEIGHT: f32 = 1.1;

type Glyph = &'static [&'static [(i8, i8)]];

#[rustfmt::skip]
fn glyph(c: char) -> Glyph {
    match c {
        'A' => &[&[(0,0),(2,8),(4,0)], &[(1,3),(3,3)]],
        'B' => &[&[(0,0),(0,8),(3,8),(4,7),(4,5),(3,4),(0,4)], &[(3,4),(4,3),(4,1),(3,0),(0,0)]],
        'C' => &[&[(4,7),(3,8),(1,8),(0,7),(0,1),(1,0),(3,0),(4,1)]],
        'D' => &[&[(0,0),(0,8),(2,8),(4,6),(4,2),(2,0),(0,0)]],
        'E' => &[&[(4,8),(0,8),(0,0),(4,0)], &[(0,4),(3,4)]],
        'F' => &[&[(4,8),(0,8),(0,0)], &[(0,4),(3,4)]],
        'G' => &[&[(4,7),(3,8),(1,8),(0,7),(0,1),(1,0),(3,0),(4,1),(4,4),(2,4)]],
        'H' => &[&[(0,0),(0,8)], &[(4,0),(4,8)], &[(0,4),(4,4)]],
        'I' => &[&[(2,8),(2,0)]],
        'J' => &[&[(4,8),(4,1),(3,0),(1,0),(0,1)]],
        'K' => &[&[(0,0),(0,8)], &[(4,8),(0,4),(4,0)]],
        'L' => &[&[(0,8),(0,0),(4,0)]],
        'M' => &[&[(0,0),(0,8),(2,5),(4,8),(4,0)]],
        'N' => &[&[(0,0),(0,8),(4,0),(4,8)]],
        'O' => &[&[(1,0),(0,1),(0,7),(1,8),(3,8),(4,7),(4,1),(3,0),(1,0)]],
        'P' => &[&[(0,0),(0,8),(3,8),(4,7),(4,5),(3,4),(0,4)]],
        'Q' => &[&[(1,0),(0,1),(0,7),(1,8),(3,8),(4,7),(4,1),(3,0),(1,0)], &[(2,2),(4,0)]],
        'R' => &[&[(0,0),(0,8),(3,8),(4,7),(4,5),(3,4),(0,4)], &[(2,4),(4,0)]],
        'S' => &[&[(4,7),(3,8),(1,8),(0,7),(0,5),(1,4),(3,4),(4,3),(4,1),(3,0),(1,0),(0,1)]],
        'T' => &[&[(0,8),(4,8)], &[(2,8),(2,0)]],
        'U' => &[&[(0,8),(0,1),(1,0),(3,0),(4,1),(4,8)]],
        'V' => &[&[(0,8),(2,0),(4,8)]],
        'W' => &[&[(0,8),(1,0),(2,4),(3,0),(4,8)]],
        'X' => &[&[(0,0),(4,8)], &[(0,8),(4,0)]],
        'Y' => &[&[(0,8),(2,4),(4,8)], &[(2,4),(2,0)]],
        'Z' => &[&[(0,8),(4,8),(0,0),(4,0)]],
        '0' => &[&[(1,0),(0,1),(0,7),(1,8),(3,8),(4,7),(4,1),(3,0),(1,0)]],
        '1' => &[&[(1,6),(2,8),(2,0)]],
        '2' => &[&[(0,7),(1,8),(3,8),(4,7),(4,6),(0,0),(4,0)]],
        '3' => &[&[(0,7),(1,8),(3,8),(4,7),(4,5),(3,4),(1,4)], &[(3,4),(4,3),(4,1),(3,0),(1,0),(0,1)]],
        '4' => &[&[(3,0),(3,8),(0,3),(4,3)]],
        '5' => &[&[(4,8),(0,8),(0,4),(3,4),(4,3),(4,1),(3,0),(1,0),(0,1)]],
        '6' => &[&[(4,7),(3,8),(1,8),(0,7),(0,1),(1,0),(3,0),(4,1),(4,3),(3,4),(0,4)]],
        '7' => &[&[(0,8),(4,8),(1,0)]],
        '8' => &[&[(1,4),(0,5),(0,7),(1,8),(3,8),(4,7),(4,5),(3,4),(1,4),(0,3),(0,1),(1,0),(3,0),(4,1),(4,3),(3,4)]],
        '9' => &[&[(0,1),(1,0),(3,0),(4,1),(4,7),(3,8),(1,8),(0,7),(0,5),(1,4),(4,4)]],
        // Marks, not ticks — punctuation doubles as key names in the help panel.
        '.' => &[&[(1,0),(2,0),(2,1),(1,1),(1,0)]],
        ',' => &[&[(1,1),(2,1),(2,0),(1,-1)]],
        ':' => &[&[(1,1),(2,1),(2,2),(1,2),(1,1)], &[(1,5),(2,5),(2,6),(1,6),(1,5)]],
        ';' => &[&[(1,5),(2,5),(2,6),(1,6),(1,5)], &[(1,2),(2,2),(2,1),(1,0)]],
        '-' => &[&[(1,4),(3,4)]],
        '+' => &[&[(2,2),(2,6)], &[(0,4),(4,4)]],
        '/' => &[&[(0,0),(4,8)]],
        '%' => &[&[(0,0),(4,8)], &[(0,7),(1,7),(1,8),(0,8),(0,7)], &[(3,0),(4,0),(4,1),(3,1),(3,0)]],
        '<' => &[&[(3,7),(0,4),(3,1)]],
        '>' => &[&[(1,7),(4,4),(1,1)]],
        '[' => &[&[(3,8),(1,8),(1,0),(3,0)]],
        ']' => &[&[(1,8),(3,8),(3,0),(1,0)]],
        '\'' => &[&[(2,8),(2,6)]],
        '=' => &[&[(0,3),(4,3)], &[(0,5),(4,5)]],
        _ => &[],
    }
}

/// Everything a click or wheel can act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Figure,
    Sym,
    Rings,
    Ratio,
    Skip,
    Twist,
    Seed,
    Speed,
    Pause,
    Palette,
    Marks,
    SnowMode,
    Gravity,
    Drift,
}

/// One run of text, optionally interactive.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub text: String,
    pub act: Option<Act>,
}

impl Token {
    pub fn plain(t: impl Into<String>) -> Self {
        Self { text: t.into(), act: None }
    }

    pub fn hot(t: impl Into<String>, act: Act) -> Self {
        Self { text: t.into(), act: Some(act) }
    }
}

/// Pixel-space hit rect for an interactive token (y down, window coords).
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub act: Act,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Status,
    Keys,
}

impl Mode {
    pub fn next(self) -> Self {
        match self {
            Mode::Off => Mode::Status,
            Mode::Status => Mode::Keys,
            Mode::Keys => Mode::Off,
        }
    }

    pub fn is_on(self) -> bool {
        self != Mode::Off
    }
}

pub const KEYS: &[&str] = &[
    "SPACE PAUSE     N/P FIGURE     R RESTART",
    "LEFT/RIGHT STEP     UP/DOWN SPEED",
    "A AUTOPLAY   S SCAFFOLD   X ENDLESS MODE",
    "SYMMETRY , .     RINGS ; '     RATIO 9 0",
    "SKIP K L     TWIST O I     G SEED   C RESET",
    "V PALETTE   T/Y HUE   U SAT   W SNOW   M MARKS",
    "SHIMMER [ ]    BLOOM - =    F FULLSCREEN    H HIDE",
    "MOUSE: CLICK NEXT   RIGHT CLICK BACK   WHEEL ADJUST",
];

/// Gap between tokens, in character cells.
const GAP: usize = 2;

/// Lay `lines` of tokens into `tess`, recording hit regions.
///
/// Strokes of the token whose action equals `hovered` go to `step_id + 1` — the
/// app binds that slot to a brighter glow.
pub fn build(
    tess: &mut Tess,
    lines: &[Vec<Token>],
    size: (u32, u32),
    scale: f32,
    step_id: u32,
    hovered: Option<Act>,
    regions: &mut Vec<Region>,
) {
    tess.clear();
    regions.clear();
    tess.detail_scale = 1.0;
    let margin = 16.0;
    let block_top = size.1 as f32 - margin - lines.len() as f32 * LINE * scale;

    let mut pts: Vec<Vec2> = Vec::with_capacity(16);
    for (row, line) in lines.iter().enumerate() {
        let baseline = block_top + row as f32 * LINE * scale + 8.0 * scale;
        let mut col = 0usize;
        for token in line {
            let tok_x0 = margin + col as f32 * ADVANCE * scale;
            let sid = if token.act.is_some() && token.act == hovered {
                step_id + 1
            } else {
                step_id
            };
            for ch in token.text.chars() {
                let x0 = margin + col as f32 * ADVANCE * scale;
                col += 1;
                if x0 > size.0 as f32 - margin - W * scale {
                    break;
                }
                for stroke in glyph(ch.to_ascii_uppercase()) {
                    pts.clear();
                    pts.extend(stroke.iter().map(|&(gx, gy)| {
                        // Glyph y runs up; pixel y runs down; the shader negates
                        // y once more through the identity camera.
                        let px = x0 + gx as f32 * scale;
                        let py = baseline - gy as f32 * scale;
                        Vec2::new(px, -py)
                    }));
                    tess.polyline(&pts, false, WEIGHT, sid);
                }
            }
            if let Some(act) = token.act {
                let tok_x1 = margin + (col as f32 - 0.4) * ADVANCE * scale;
                // Pad the rect a little so near-misses still land.
                regions.push(Region {
                    x0: tok_x0 - 3.0,
                    y0: baseline - 9.0 * scale,
                    x1: tok_x1 + 3.0,
                    y1: baseline + 3.0 * scale,
                    act,
                });
            }
            col += GAP;
        }
    }
}

pub fn hit(regions: &[Region], x: f32, y: f32) -> Option<Act> {
    regions
        .iter()
        .find(|r| x >= r.x0 && x <= r.x1 && y >= r.y0 && y <= r.y1)
        .map(|r| r.act)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_cover_hot_tokens_and_only_those() {
        let mut tess = Tess::default();
        let mut regions = Vec::new();
        let lines = vec![
            vec![Token::hot("FLOWER OF LIFE", Act::Figure), Token::plain("4/16")],
            vec![Token::hot("SYM 6", Act::Sym), Token::hot("RINGS 2", Act::Rings)],
        ];
        build(&mut tess, &lines, (900, 900), 1.5, 0, None, &mut regions);
        assert_eq!(regions.len(), 3, "one region per hot token");
        assert!(!tess.verts.is_empty(), "text produced no geometry");

        // The centre of each region resolves to its action; far away, nothing.
        for r in &regions {
            let (cx, cy) = ((r.x0 + r.x1) * 0.5, (r.y0 + r.y1) * 0.5);
            assert_eq!(hit(&regions, cx, cy), Some(r.act));
        }
        assert_eq!(hit(&regions, 890.0, 10.0), None);
    }

    #[test]
    fn hover_reassigns_exactly_the_hovered_tokens_strokes() {
        let mut tess = Tess::default();
        let mut regions = Vec::new();
        let lines = vec![vec![Token::hot("AAA", Act::Sym), Token::hot("BBB", Act::Rings)]];
        build(&mut tess, &lines, (900, 900), 1.5, 7, Some(Act::Rings), &mut regions);
        let ids: std::collections::HashSet<u32> =
            tess.verts.iter().map(|v| v.step_id).collect();
        assert!(ids.contains(&7) && ids.contains(&8), "expected both slots in use: {ids:?}");
    }

    #[test]
    fn every_printable_glyph_is_drawable() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,:;-+/%<>[]'= ".chars() {
            for stroke in glyph(c) {
                assert!(stroke.len() != 1, "glyph {c:?} has a single-point stroke");
            }
        }
    }
}
