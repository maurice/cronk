//! Shared pointer feedback. Render state is transient, never workspace configuration.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tui_lipan::prelude::*;

use super::{Cronk, Msg};

pub(super) const CLICK_FLASH: Duration = Duration::from_millis(140);

#[derive(Clone, PartialEq, Eq)]
pub(super) struct StatusTooltip {
    pub key: String,
    pub text: String,
    pub position: (u16, u16),
}

#[derive(Default)]
pub(super) struct Feedback {
    hovered: HashSet<String>,
    flashes: HashMap<String, u64>,
    generation: u64,
    pub tooltip: Option<StatusTooltip>,
}

impl Feedback {
    pub fn hover(&mut self, key: String, entered: bool) -> bool {
        if entered {
            self.hovered.insert(key)
        } else {
            self.hovered.remove(&key)
        }
    }

    pub fn status_hover(
        &mut self,
        key: String,
        text: String,
        position: Option<(u16, u16)>,
    ) -> bool {
        if position.is_none() && self.tooltip.as_ref().is_none_or(|tip| tip.key != key) {
            return false;
        }
        let tooltip = position.map(|position| StatusTooltip {
            key,
            text,
            position,
        });
        if self.tooltip == tooltip {
            return false;
        }
        self.tooltip = tooltip;
        true
    }

    pub fn clear(&mut self) {
        self.tooltip = None;
        self.hovered.clear();
        self.flashes.clear();
    }

    pub fn flash(&mut self, key: String) -> u64 {
        self.generation += 1;
        self.flashes.insert(key, self.generation);
        self.generation
    }

    pub fn end_flash(&mut self, key: &str, generation: u64) -> bool {
        if self.flashes.get(key) == Some(&generation) {
            self.flashes.remove(key);
            true
        } else {
            false
        }
    }
}

/// A transform, not a replacement selection color: selected and hovered rows stay distinct
/// on both dark and light themes. Foreground/status colors and protected label spans survive.
pub(super) fn hover_style() -> Style {
    Style::new().transform_bg(ColorTransform::elevate(0.08))
}

pub(super) fn style(ctx: &Context<Cronk>, key: &str, base: Style) -> Style {
    if ctx.state.config.animations && ctx.state.feedback.flashes.contains_key(key) {
        base.patch(Style::new().transform_bg(ColorTransform::elevate(0.18)))
    } else if ctx.state.feedback.hovered.contains(key) {
        base.patch(hover_style())
    } else {
        base
    }
}

/// Observe a standalone scrollbar's final column without capturing clicks, wheel events,
/// or drags. The toolkit's ScrollbarConfig has no hover slot of its own.
pub(super) fn scrollbar(ctx: &Context<Cronk>, key: impl Into<String>, child: Element) -> Element {
    let key = key.into();
    let move_key = key.clone();
    let leave_key = key.clone();
    let link = ctx.link().clone();
    Element::from(
        MouseRegion::new()
            .hover_style(Style::new())
            .on_mouse_move(ctx.link().callback(move |event: MouseMoveEvent| {
                Msg::Hover(move_key.clone(), event.local_x + 1 == event.target_w)
            }))
            .on_hover_change(Callback::new(move |entered: bool| {
                if !entered {
                    link.send(Msg::Hover(leave_key.clone(), false));
                }
            }))
            .child(child),
    )
    .key(format!("{key}-pointer"))
}

/// Native inputs can share click feedback without wrapping/capturing their mouse events.
pub(super) fn activation<T: 'static>(
    ctx: &Context<Cronk>,
    key: impl Into<String>,
    message: impl Fn() -> Msg + 'static,
) -> Callback<T> {
    let key = key.into();
    let link = ctx.link().clone();
    ctx.link().callback(move |_| {
        link.send(Msg::ClickFlash(key.clone()));
        message()
    })
}

/// Every custom clickable surface uses the same callbacks and timing policy.
/// Children receive feedback through `style`, rather than a post-process effect that would
/// recolor GitLab label pills along with the rest of the surface.
pub(super) fn click(
    ctx: &Context<Cronk>,
    key: impl Into<String>,
    child: impl Into<Element>,
    message: impl Fn() -> Msg + 'static,
) -> Element {
    let key = key.into();
    let hover_key = key.clone();
    Element::from(
        MouseRegion::new()
            .hover_style(Style::new())
            .on_hover_change(
                ctx.link()
                    .callback(move |entered| Msg::Hover(hover_key.clone(), entered)),
            )
            .on_click(activation(ctx, key.clone(), message))
            .child(child),
    )
    .key(key)
}
