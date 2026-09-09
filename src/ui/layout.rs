//! Renderer-independent logical layout helpers built on Taffy.

use fontdue::Font;
use std::collections::HashMap;
use taffy::prelude::{AvailableSpace, LengthPercentage, NodeId, Size, Style, TaffyTree};

pub const SPACE_1: f64 = 4.0;
pub const SPACE_2: f64 = 8.0;
pub const SPACE_3: f64 = 12.0;
pub const SPACE_5: f64 = 20.0;
pub const RADIUS_CARD: f64 = 10.0;
pub const RADIUS_CONTROL: f64 = 7.0;
pub const WINDOW_HEADER_HEIGHT: f64 = 40.0;
pub const WINDOW_HEADER_TITLE_SIZE: f32 = 14.0;
pub const WINDOW_HEADER_ICON_SIZE: f64 = 28.0;
pub const WINDOW_HEADER_ACTION_SIZE: f64 = 32.0;
pub const WINDOW_HEADER_PADDING: f64 = 8.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn inset(self, amount: f64) -> Self {
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.w - amount * 2.0).max(0.0),
            (self.h - amount * 2.0).max(0.0),
        )
    }

    pub fn right(self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(self) -> f64 {
        self.y + self.h
    }
}

#[derive(Clone, Debug)]
pub struct TextNode {
    text: String,
    size: f32,
}

/// Small high-level wrapper that keeps parent links so nested Taffy layouts
/// can be exposed as absolute logical rectangles to painting and hit-testing.
pub struct LayoutTree {
    tree: TaffyTree<TextNode>,
    parents: HashMap<NodeId, NodeId>,
}

impl LayoutTree {
    pub fn new() -> Self {
        let mut tree = TaffyTree::new();
        tree.disable_rounding();
        Self {
            tree,
            parents: HashMap::new(),
        }
    }

    pub fn leaf(&mut self, style: Style) -> Result<NodeId, String> {
        self.tree
            .new_leaf(style)
            .map_err(|e| format!("create layout leaf: {e}"))
    }

    pub fn text(
        &mut self,
        text: impl Into<String>,
        size: f32,
        style: Style,
    ) -> Result<NodeId, String> {
        self.tree
            .new_leaf_with_context(
                style,
                TextNode {
                    text: text.into(),
                    size,
                },
            )
            .map_err(|e| format!("create text layout leaf: {e}"))
    }

    pub fn container(&mut self, style: Style, children: &[NodeId]) -> Result<NodeId, String> {
        let parent = self
            .tree
            .new_with_children(style, children)
            .map_err(|e| format!("create layout container: {e}"))?;
        for child in children {
            self.parents.insert(*child, parent);
        }
        Ok(parent)
    }

    pub fn compute(
        &mut self,
        root: NodeId,
        width: f64,
        height: Option<f64>,
        font: &Font,
    ) -> Result<(), String> {
        let available = Size {
            width: AvailableSpace::Definite(width as f32),
            height: height
                .map(|value| AvailableSpace::Definite(value as f32))
                .unwrap_or(AvailableSpace::MaxContent),
        };
        self.tree
            .compute_layout_with_measure(root, available, |known, available, _, context, _| {
                let measured = context
                    .map(|text| measure_text(font, &text.text, text.size))
                    .unwrap_or(Size::ZERO);
                Size {
                    width: known
                        .width
                        .unwrap_or_else(|| constrain(measured.width, available.width)),
                    height: known
                        .height
                        .unwrap_or_else(|| constrain(measured.height, available.height)),
                }
            })
            .map_err(|e| format!("compute layout: {e}"))
    }

    pub fn rect(&self, node: NodeId) -> Result<Rect, String> {
        let layout = self
            .tree
            .layout(node)
            .map_err(|e| format!("read layout: {e}"))?;
        let mut x = layout.location.x as f64;
        let mut y = layout.location.y as f64;
        let mut parent = self.parents.get(&node).copied();
        while let Some(id) = parent {
            let layout = self
                .tree
                .layout(id)
                .map_err(|e| format!("read parent layout: {e}"))?;
            x += layout.location.x as f64;
            y += layout.location.y as f64;
            parent = self.parents.get(&id).copied();
        }
        Ok(Rect::new(
            x,
            y,
            layout.size.width as f64,
            layout.size.height as f64,
        ))
    }
}

fn measure_text(font: &Font, text: &str, size: f32) -> Size<f32> {
    let width = text
        .chars()
        .map(|character| font.metrics(character, size).advance_width)
        .sum();
    let height = font
        .horizontal_line_metrics(size)
        .map(|metrics| metrics.new_line_size)
        .unwrap_or(size * 1.2);
    Size { width, height }
}

fn constrain(value: f32, available: AvailableSpace) -> f32 {
    match available {
        AvailableSpace::Definite(limit) => value.min(limit),
        AvailableSpace::MinContent | AvailableSpace::MaxContent => value,
    }
}

pub fn points(all: f64) -> taffy::geometry::Rect<LengthPercentage> {
    taffy::geometry::Rect {
        left: LengthPercentage::length(all as f32),
        right: LengthPercentage::length(all as f32),
        top: LengthPercentage::length(all as f32),
        bottom: LengthPercentage::length(all as f32),
    }
}
