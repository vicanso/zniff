//! Declarative TSX preview card renderer (`<Section>`, `<KeyValue>`, `<Badge>`, `<Code>`).

#[allow(unused_imports)]
use super::*;

use crate::hook_spec::{PreviewCardNode, PreviewTone};

fn map_badge_tone(tone: PreviewTone) -> BadgeTone {
    match tone {
        PreviewTone::Neutral => BadgeTone::Neutral,
        PreviewTone::Ok => BadgeTone::Ok,
        PreviewTone::Warn => BadgeTone::Warn,
        PreviewTone::Danger => BadgeTone::Err,
        PreviewTone::Accent => BadgeTone::Accent,
    }
}

fn render_card_node(node: PreviewCardNode) -> AnyElement {
    match node {
        PreviewCardNode::Section { title, children } => {
            let mut body = view().w_full().gap_2();
            if !title.is_empty() {
                body = body.child(
                    text(title)
                        .text_size(px(TEXT_SM))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg_primary()),
                );
            }
            for child in children {
                body = body.child(render_card_node(child));
            }
            view()
                .w_full()
                .p_3()
                .rounded(px(RADIUS_LG))
                .border_1()
                .border_color(border_strong())
                .bg(bg_card())
                .child(body)
                .into_element()
        }
        PreviewCardNode::KeyValue { label, value } => view()
            .w_full()
            .flex_row()
            .items_start()
            .gap_2p5()
            .child(
                text(label)
                    .min_w(px(88.0))
                    .text_size(px(TEXT_XS))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(fg_secondary()),
            )
            .child(
                mono(value)
                    .flex_1()
                    .text_size(scaled_px(TEXT_XS))
                    .text_color(fg_primary()),
            )
            .into_element(),
        PreviewCardNode::Badge { tone, text } => view()
            .w_full()
            .flex_row()
            .items_center()
            .child(ui_badge(text, map_badge_tone(tone)))
            .into_element(),
        PreviewCardNode::Code { language, code } => {
            let mut box_view = view()
                .w_full()
                .p_2p5()
                .gap_1()
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(border_subtle())
                .bg(bg_code());
            if let Some(lang) = language.filter(|l| !l.trim().is_empty()) {
                box_view = box_view.child(
                    text(lang)
                        .text_size(px(TEXT_XS))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(fg_muted()),
                );
            }
            box_view
                .child(
                    mono(code)
                        .text_size(scaled_px(TEXT_XS))
                        .text_color(fg_primary()),
                )
                .into_element()
        }
    }
}

/// Renders structured `onPreview` / `onRun` TSX cards above detail request/response cards.
pub(crate) fn hook_preview_cards(cards: Signal<Vec<PreviewCardNode>>) -> impl IntoElement {
    when(
        move || !cards.with(Vec::is_empty),
        move || {
            let items = cards.get();
            let mut stack = view().w_full().gap_2p5();
            for node in items {
                stack = stack.child(render_card_node(node));
            }
            stack
        },
    )
}
