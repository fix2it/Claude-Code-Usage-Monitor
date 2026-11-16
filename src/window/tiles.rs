//! Hoverable widget tiles: one table of (tile, theme object) drives the
//! hit-test rects of every surface, the hover flags and the hover phases that
//! the theme bindings read.

use std::ops::Index;
use std::time::Duration;

use super::PhaseAnimation;
use crate::theme_engine::{self, SplitPool};

pub(super) type Rect = (f64, f64, f64, f64);

/// Every tile hovers in 200 ms.
pub(super) const TILE_HOVER_DURATION_MS: u64 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tile {
    /// Fades agy's Gemini row to its Claude/GPT row (or the hidden pool).
    Antigravity,
    Claude,
    Codex,
    OpenCode,
    Qoder,
    /// The agy external-models widget.
    AgyExternal,
    /// A second pool in its own tile ("Pools on one widget" off).
    Split(SplitPool),
}

const TILE_COUNT: usize = 6 + SplitPool::ALL.len();

impl Tile {
    pub(super) const ALL: [Tile; TILE_COUNT] = [
        Tile::Antigravity,
        Tile::Claude,
        Tile::Codex,
        Tile::OpenCode,
        Tile::Qoder,
        Tile::AgyExternal,
        Tile::Split(SplitPool::ALL[0]),
        Tile::Split(SplitPool::ALL[1]),
        Tile::Split(SplitPool::ALL[2]),
        Tile::Split(SplitPool::ALL[3]),
    ];

    const fn index(self) -> usize {
        match self {
            Tile::Antigravity => 0,
            Tile::Claude => 1,
            Tile::Codex => 2,
            Tile::OpenCode => 3,
            Tile::Qoder => 4,
            Tile::AgyExternal => 5,
            Tile::Split(pool) => 6 + pool.index(),
        }
    }

    /// Theme object whose bounds are the tile's hover area.
    pub(super) const fn object_id(self) -> &'static str {
        match self {
            Tile::Antigravity => "antigravity-provider",
            Tile::Claude => "claude-provider",
            Tile::Codex => "codex-provider",
            Tile::OpenCode => "opencode-provider",
            Tile::Qoder => "qoder-provider",
            Tile::AgyExternal => theme_engine::ANTIGRAVITY_EXTERNAL_WIDGET,
            Tile::Split(pool) => pool.object_id(),
        }
    }

    /// Phase without hover. agy's phase is the Gemini row's opacity, so it
    /// rests at 1 and falls on hover; every other phase rises from 0.
    const fn rest_phase(self) -> f64 {
        match self {
            Tile::Antigravity => 1.0,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct TileHover {
    pub(super) hovered: bool,
    pub(super) phase: f64,
    pub(super) animation: Option<PhaseAnimation>,
}

/// Hover state of every tile plus hit-test rects per surface. A rect is
/// rebuilt only while it is `None`; see `render_layered_impl`.
#[derive(Clone, Debug)]
pub(super) struct Tiles {
    hovers: [TileHover; TILE_COUNT],
    rects: Vec<[Option<Rect>; TILE_COUNT]>,
}

impl Default for Tiles {
    fn default() -> Self {
        Self {
            hovers: Tile::ALL.map(|tile| TileHover {
                hovered: false,
                phase: tile.rest_phase(),
                animation: None,
            }),
            rects: Vec::new(),
        }
    }
}

impl Index<Tile> for Tiles {
    type Output = TileHover;

    fn index(&self, tile: Tile) -> &TileHover {
        &self.hovers[tile.index()]
    }
}

impl Tiles {
    /// Record the cursor entering or leaving `tile`. With `animate` the phase
    /// starts moving toward its new target; returns true when it did.
    pub(super) fn set_hovered(&mut self, tile: Tile, hovered: bool, animate: bool) -> bool {
        let hover = &mut self.hovers[tile.index()];
        if hover.hovered == hovered {
            return false;
        }
        hover.hovered = hovered;
        if !animate {
            return false;
        }
        let rest = tile.rest_phase();
        let target = if hovered { 1.0 - rest } else { rest };
        hover.animation = Some(PhaseAnimation::hover(
            hover.phase,
            target,
            Duration::from_millis(TILE_HOVER_DURATION_MS),
        ));
        true
    }

    /// Drop hover and animation: the tile is gone or its provider is off.
    pub(super) fn reset(&mut self, tile: Tile) {
        self.hovers[tile.index()] = TileHover {
            hovered: false,
            phase: tile.rest_phase(),
            animation: None,
        };
    }

    pub(super) fn any_hovered(&self) -> bool {
        self.hovers.iter().any(|hover| hover.hovered)
    }

    pub(super) fn any_animating(&self) -> bool {
        self.hovers.iter().any(|hover| hover.animation.is_some())
    }

    /// Advance every hover animation. Returns (phase changed, still running).
    pub(super) fn tick(&mut self, throttle: bool) -> (bool, bool) {
        self.hovers
            .iter_mut()
            .fold((false, false), |(changed, active), hover| {
                let (c, a) =
                    PhaseAnimation::tick_slot(&mut hover.animation, &mut hover.phase, throttle);
                (changed | c, active | a)
            })
    }

    pub(super) fn split_phases(&self) -> [f64; SplitPool::ALL.len()] {
        SplitPool::ALL.map(|pool| self[Tile::Split(pool)].phase)
    }

    /// Hit-test rect of `tile` on `surface`, in theme units.
    pub(super) fn rect(&self, surface: usize, tile: Tile) -> Option<Rect> {
        self.rects.get(surface)?[tile.index()]
    }

    /// Some rect of `surface` is unknown (never built, or the theme has no
    /// such object, which keeps it `None`).
    pub(super) fn rects_incomplete(&self, surface: usize) -> bool {
        self.rects
            .get(surface)
            .is_none_or(|rects| rects.iter().any(Option::is_none))
    }

    /// Store the bounds of `Tile::ALL`'s objects, in that order.
    pub(super) fn set_rects(&mut self, surface: usize, bounds: &[Option<Rect>]) {
        if self.rects.len() <= surface {
            self.rects.resize(surface + 1, [None; TILE_COUNT]);
        }
        for (slot, rect) in self.rects[surface].iter_mut().zip(bounds) {
            *slot = *rect;
        }
    }

    /// Widget geometry changed: rebuild every rect on the next render.
    pub(super) fn clear_rects(&mut self) {
        self.rects.clear();
    }

    pub(super) fn object_ids() -> [&'static str; TILE_COUNT] {
        Tile::ALL.map(Tile::object_id)
    }
}
