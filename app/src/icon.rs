//! Small stroke icons drawn on a canvas so they follow the text color.

use nsh_core::canvas::{LineCap, LineJoin};
use nsh_core::prelude::*;

#[derive(Clone, Copy)]
pub enum Glyph {
    Trash,
    Play,
    Stop,
    Chevron,
    ChevronUp,
    ChevronRight,
    Sliders,
    Download,
    Shield,
    Badge,
    List,
    Prompt,
    Columns,
    Check,
    Close,
    History,
    Alert,
}

pub fn glyph(
    kind: impl Fn() -> Glyph + 'static,
    paint: impl Fn() -> Hsla + 'static,
) -> impl IntoElement {
    glyph_px(13.0, kind, paint)
}

pub fn glyph_px(
    size: f32,
    kind: impl Fn() -> Glyph + 'static,
    paint: impl Fn() -> Hsla + 'static,
) -> impl IntoElement {
    canvas(move || draw(kind(), paint(), size))
        .w(px(size))
        .h(px(size))
        .flex_shrink_0()
}

fn draw(kind: Glyph, paint: Hsla, size: f32) -> Painter {
    let scale = (size - 2.0).max(1.0) / 16.0;
    let mut painter = Painter::new()
        .translate(1.0, 1.0)
        .scale(scale, scale)
        .line_width(1.65)
        .line_cap(LineCap::Round)
        .line_join(LineJoin::Round)
        .stroke_color(paint)
        .fill_color(paint);
    painter = match kind {
        Glyph::Trash => painter
            .stroke_svg("M3 5.2 H13")
            .stroke_svg("M6.2 5.2 V3.4 H9.8 V5.2")
            .stroke_svg("M4.6 5.2 L5.3 13.2 H10.7 L11.4 5.2")
            .stroke_svg("M7.1 7.4 V11")
            .stroke_svg("M8.9 7.4 V11"),
        Glyph::Play => painter.fill_svg("M5.2 3.1 L12.4 8 L5.2 12.9 Z"),
        Glyph::Stop => painter.fill_svg("M4.2 4.2 H11.8 V11.8 H4.2 Z"),
        Glyph::Chevron => painter.stroke_svg("M4 6.2 L8 10.2 L12 6.2"),
        Glyph::ChevronUp => painter.stroke_svg("M4 9.8 L8 5.8 L12 9.8"),
        Glyph::ChevronRight => painter.stroke_svg("M6.2 4 L10.2 8 L6.2 12"),
        Glyph::Sliders => painter
            .stroke_svg("M2.2 4.4 H3.4")
            .stroke_svg("M7.8 4.4 H13.8")
            .stroke_svg("M2.2 8 H8.2")
            .stroke_svg("M12.6 8 H13.8")
            .stroke_svg("M2.2 11.6 H4.4")
            .stroke_svg("M8.8 11.6 H13.8")
            .fill_svg("M5.6 4.4 m-1.8 0 a1.8 1.8 0 1 0 3.6 0 a1.8 1.8 0 1 0 -3.6 0")
            .fill_svg("M10.4 8 m-1.8 0 a1.8 1.8 0 1 0 3.6 0 a1.8 1.8 0 1 0 -3.6 0")
            .fill_svg("M6.6 11.6 m-1.8 0 a1.8 1.8 0 1 0 3.6 0 a1.8 1.8 0 1 0 -3.6 0"),
        Glyph::Download => painter
            .stroke_svg("M8 2.4 V9.2")
            .stroke_svg("M5 6.6 L8 9.8 L11 6.6")
            .stroke_svg("M3.2 13.2 H12.8"),
        Glyph::Shield => painter.stroke_svg("M8 2.2 L12.8 4.3 V8.2 L8 13.6 L3.2 8.2 V4.3 Z"),
        Glyph::Badge => painter
            .stroke_svg("M4.2 2.6 H11.8 V13.4 H4.2 Z")
            .stroke_svg("M6.2 6 H9.8")
            .stroke_svg("M6.2 8.4 H9.8")
            .stroke_svg("M6.2 10.8 H8.2"),
        Glyph::List => painter
            .stroke_svg("M3 4.4 H13")
            .stroke_svg("M3 8 H13")
            .stroke_svg("M3 11.6 H9"),
        Glyph::Columns => painter
            .stroke_svg("M2.6 3.2 H13.4 V12.8 H2.6 Z")
            .stroke_svg("M6.2 3.2 V12.8")
            .stroke_svg("M9.8 3.2 V12.8"),
        Glyph::Prompt => painter
            .stroke_svg("M3.2 3.2 H12.8 V12.8 H3.2 Z")
            .stroke_svg("M5.2 8.2 L7.2 9.6 L5.2 11")
            .stroke_svg("M8.4 11 H10.8"),
        Glyph::Check => painter.stroke_svg("M3.4 8.2 L6.6 11.4 L12.6 4.6"),
        Glyph::Close => painter
            .stroke_svg("M4.2 4.2 L11.8 11.8")
            .stroke_svg("M11.8 4.2 L4.2 11.8"),
        Glyph::History => painter
            .stroke_svg("M3.2 5.4 A5.2 5.2 0 1 1 2.9 8.8")
            .stroke_svg("M2.8 2.8 V5.6 H5.6")
            .stroke_svg("M8 5.4 V8.2 L10 9.4"),
        Glyph::Alert => painter
            .stroke_svg("M8 2.4 L13.8 13.4 H2.2 Z")
            .stroke_svg("M8 6.4 V9.4")
            .fill_svg("M8 11.6 m-0.85 0 a0.85 0.85 0 1 0 1.7 0 a0.85 0.85 0 1 0 -1.7 0"),
    };
    painter
}
