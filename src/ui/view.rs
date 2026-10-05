use super::{
    Completion, Cronk, Dialog, DialogKind, Msg, Scope, State, detail_section_offset,
    interaction::{self, click},
    list_height_for_tab, list_row_height,
};
use crate::{
    build_info,
    config::{Config, UserFormatter},
    model::*,
};
use std::{any::Any, collections::HashMap};
use tui_lipan::{
    TextAreaColorInput, TextAreaColorLines, TextAreaColorStrategy,
    prelude::*,
    style::{RowStylePolicy, ThemePalette},
};

#[derive(Clone, Copy)]
struct Colors {
    background: Color,
    surface: Color,
    selection: Color,
    foreground: Color,
    muted: Color,
    accent: Color,
    green: Color,
    yellow: Color,
    red: Color,
    cyan: Color,
    purple: Color,
    pending: Color,
}

impl Colors {
    fn new(config: &Config) -> Self {
        let values = match config.theme.as_str() {
            "dracula" => [
                0x282a36, 0x21222c, 0x44475a, 0xf8f8f2, 0xa5a8c0, 0xbd93f9, 0x50fa7b, 0xf1fa8c,
                0xff5555, 0x8be9fd, 0xff79c6,
            ],
            "light" => [
                0xf7f8fc, 0xeceff6, 0xdce5fa, 0x24283b, 0x626b83, 0x5746bd, 0x187348, 0x946200,
                0xc2354b, 0x007a91, 0x9146ae,
            ],
            _ => [
                0x101521, 0x192131, 0x293654, 0xe5eaf5, 0x96a3bc, 0x9b9fff, 0x63dba5, 0xf0c674,
                0xff758f, 0x70d7ec, 0xc69cf4,
            ],
        };
        let c = &config.colors;
        let custom = |value: &Option<String>, fallback| {
            value
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(Color::hex_u24(fallback))
        };
        Self {
            background: custom(&c.background, values[0]),
            surface: custom(&c.surface, values[1]),
            selection: custom(&c.selection, values[2]),
            foreground: custom(&c.foreground, values[3]),
            muted: custom(&c.muted, values[4]),
            accent: custom(&c.accent, values[5]),
            green: Color::hex_u24(values[6]),
            yellow: Color::hex_u24(values[7]),
            red: Color::hex_u24(values[8]),
            cyan: Color::hex_u24(values[9]),
            purple: Color::hex_u24(values[10]),
            // A darker neutral keeps the light theme readable without using a warning color.
            pending: Color::hex_u24(if config.theme == "light" {
                0x626262
            } else {
                0xc7c7c7
            }),
        }
    }

    fn base(self) -> Style {
        Style::new().fg(self.foreground).bg(self.background)
    }

    fn selected(self) -> Style {
        Style::new().fg(self.foreground).bg(self.selection)
    }

    fn theme(self) -> Theme {
        let mut theme = ThemePalette::new(self.foreground, self.background, self.accent)
            .muted(self.muted)
            .border(self.muted)
            .success(self.green)
            .warning(self.yellow)
            .error(self.red)
            .info(self.cyan)
            .into_theme();
        theme.primary = self.base();
        theme.hover = interaction::hover_style();
        theme.selection = self.selected();
        theme.text_selection = self.selected();
        theme.surface.panel = self.surface;
        theme.surface.element = self.surface;
        theme.surface.menu = self.surface;
        theme.focus_decoration = false;
        theme.document.heading_styles = [Style::new().fg(self.accent).bold(); 6];
        theme.document.code_inline = Style::new().fg(self.cyan).bg(self.surface);
        theme.document.code_block = Style::new().fg(self.foreground).bg(self.surface);
        theme.document.link = Style::new().fg(self.cyan).underline();
        theme.document.blockquote_bar = Style::new().fg(self.purple);
        theme.document.table_header = Style::new().fg(self.accent).bold();
        theme.document.table_border = Style::new().fg(self.muted);
        theme.document.hr = Style::new().fg(self.muted);
        theme
    }

    fn status(self, status: &str) -> (char, Color) {
        match status {
            "opened" | "success" | "passed" | "resolved" => ('●', self.green),
            "running" => ('◐', self.cyan),
            "failed" | "error" | "unresolved" => ('●', self.red),
            "merged" => ('●', self.purple),
            "pending" | "created" | "waiting_for_resource" | "preparing" => ('○', self.pending),
            "manual" | "scheduled" => ('○', self.accent),
            "warning" => ('○', self.yellow),
            _ => ('○', self.muted),
        }
    }
}

fn vertical_scrollbar(ctx: &Context<Cronk>, key: &str, colors: Colors) -> ScrollbarConfig {
    let thumb = interaction::style(ctx, key, Style::new().fg(colors.accent).bg(colors.surface));
    let track = interaction::style(ctx, key, Style::new().fg(colors.muted).bg(colors.surface));
    ScrollbarConfig::new()
        .variant(ScrollbarVariant::Standalone)
        .gap(1)
        .thumb('█')
        .thumb_style(thumb)
        .thumb_focus_style(thumb)
        .track_style(track)
}

fn content_padding(left: u16) -> Padding {
    Padding {
        left,
        right: 0,
        top: 0,
        bottom: 0,
    }
}

fn parse_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().map(Color::hex_u24)
}

fn line(value: impl Into<String>, style: Style) -> HStack {
    rich(vec![Span::new(value.into())], style)
}

fn rich(spans: Vec<Span>, style: Style) -> HStack {
    // Text paints only its glyph cells; the container supplies the full-width row background.
    HStack::new().height(Length::Px(1)).style(style).child(
        Text::from_spans(spans)
            .style(style)
            .width(Length::Flex(1))
            .height(Length::Px(1))
            .overflow(Overflow::Ellipsis),
    )
}

fn blank() -> Element {
    Spacer::new().height(Length::Px(1)).into()
}

fn project_name(state: &State, id: u64) -> String {
    state.project(id).map_or_else(
        || format!("Project {id}"),
        |project| {
            if project.alias.is_empty() {
                project
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&project.path)
                    .to_owned()
            } else {
                project.alias.clone()
            }
        },
    )
}

fn item_id(key: &ItemKey) -> String {
    format!("{}{}", key.kind.symbol(), key.iid)
}

fn item_key(key: &ItemKey) -> String {
    format!("{}-{}-{}", key.project, key.kind.segment(), key.iid)
}

fn status_help(subject: &str, status: &str) -> String {
    let explanation = match status {
        "opened" => "Open and active",
        "closed" => "Closed",
        "merged" => "Changes have been merged",
        "locked" => "Temporarily locked while being updated",
        "success" | "passed" => "Completed successfully",
        "running" => "Currently running",
        "failed" | "error" => "Finished with an error",
        "pending" => "Waiting for an available runner",
        "created" => "Created, not yet queued for execution",
        "waiting_for_resource" => "Waiting for a required resource",
        "preparing" => "Runner is preparing execution",
        "manual" => "Requires manual action to start",
        "scheduled" => "Scheduled to run later",
        "canceled" | "cancelled" => "Canceled before completion",
        "skipped" => "Not run because its conditions were not met",
        "resolved" => "All resolvable comments have been resolved",
        "unresolved" => "Contains comments that still need resolution",
        "comment" => "Comment that does not require resolution",
        "warning" => "See the adjacent message for additional information",
        _ => "Status reported by GitLab",
    };
    format!("{subject}: {status} — {explanation}")
}

/// Only the single glyph is the trigger: adjacent padding/text never opens the tip.
/// A passive mouse region observes hover without intercepting the surrounding row click.
fn dot_tooltip(
    ctx: &Context<Cronk>,
    key: impl Into<String>,
    symbol: char,
    color: Color,
    help: impl Into<String>,
    style: Style,
) -> Element {
    let key = key.into();
    let move_key = key.clone();
    let leave_key = key.clone();
    let help = help.into();
    let link = ctx.link().clone();
    Element::from(
        MouseRegion::new()
            .hover_style(Style::new())
            .on_mouse_move(ctx.link().callback(move |event: MouseMoveEvent| {
                Msg::StatusHover(move_key.clone(), help.clone(), Some((event.x, event.y)))
            }))
            .on_hover_change(Callback::new(move |entered: bool| {
                if !entered {
                    link.send(Msg::StatusHover(leave_key.clone(), String::new(), None));
                }
            }))
            .child(
                Text::new(symbol.to_string())
                    .width(Length::Px(1))
                    .height(Length::Px(1))
                    .style(style.fg(color)),
            ),
    )
    .key(key)
}

fn status_dot(
    ctx: &Context<Cronk>,
    key: impl Into<String>,
    subject: &str,
    status: &str,
    style: Style,
    colors: Colors,
) -> Element {
    let (symbol, color) = colors.status(status);
    dot_tooltip(ctx, key, symbol, color, status_help(subject, status), style)
}

fn label_style(label: &Label, colors: Colors) -> Style {
    // GitLab supplies both colors. Neither selection nor contrast correction may change them.
    Style::new()
        .bg(parse_color(&label.color).unwrap_or(colors.surface))
        .fg(parse_color(&label.text_color).unwrap_or(colors.foreground))
        .contrast_policy(ContrastPolicy::Off)
}

fn label_pill_span(label: &Label, colors: Colors) -> Span {
    Span::new(format!(" {} ", label.name))
        .style(label_style(label, colors))
        .row_style_policy(RowStylePolicy::Disabled)
}

fn label_text_span(text: &str, label: &Label, colors: Colors) -> Span {
    Span::new(text.to_owned())
        .style(label_style(label, colors))
        .row_style_policy(RowStylePolicy::Disabled)
}

fn label_spans(labels: &[Label], colors: Colors) -> Vec<Span> {
    let mut spans = Vec::with_capacity(labels.len() * 2);
    for label in labels {
        spans.push(label_pill_span(label, colors));
        spans.push(Span::new(" "));
    }
    spans
}

#[derive(Clone)]
struct LabelColorStrategy {
    swatches: HashMap<String, Label>,
    colors: Colors,
}

impl LabelColorStrategy {
    fn line_spans(&self, line: &str) -> Vec<Span> {
        if line.is_empty() {
            return vec![Span::new("")];
        }
        let mut spans = Vec::new();
        let mut start = 0;
        for (index, ch) in line.char_indices() {
            if ch == ',' {
                self.push_token(&mut spans, &line[start..index]);
                spans.push(Span::new(","));
                start = index + ch.len_utf8();
            }
        }
        self.push_token(&mut spans, &line[start..]);
        spans
    }

    fn push_token(&self, spans: &mut Vec<Span>, token: &str) {
        if token.is_empty() {
            return;
        }
        let trimmed = token.trim();
        if trimmed.is_empty() {
            spans.push(Span::new(token.to_owned()));
            return;
        }
        let leading = token.len() - token.trim_start().len();
        let trailing = token.len() - token.trim_end().len();
        if leading > 0 {
            spans.push(Span::new(token[..leading].to_owned()));
        }
        if let Some(label) = self.swatches.get(trimmed) {
            spans.push(label_text_span(trimmed, label, self.colors));
        } else {
            spans.push(Span::new(trimmed.to_owned()));
        }
        if trailing > 0 {
            spans.push(Span::new(token[token.len() - trailing..].to_owned()));
        }
    }
}

impl TextAreaColorStrategy for LabelColorStrategy {
    fn highlight(&self, input: TextAreaColorInput<'_>) -> TextAreaColorLines {
        input
            .value
            .split('\n')
            .map(|line| self.line_spans(line))
            .collect()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

pub(super) fn view(ctx: &Context<Cronk>) -> Element {
    let state = &ctx.state;
    let colors = Colors::new(&state.config);
    let content = match state.scope {
        Scope::List => main_list(ctx, colors),
        Scope::Details | Scope::Section => detail(ctx, colors),
    };
    // Keep six rows above and two below the list, matching list_height exactly.
    let shell = VStack::new()
        .style(colors.base())
        .child(brand(state, colors))
        .child(tab_bar(ctx, colors))
        .child(breadcrumb(ctx, colors))
        .child(context_line(state, colors))
        .child(blank())
        .child(content)
        .child(footer(ctx, colors));
    let mut layers = ZStack::new().passthrough(true).child(shell);
    {
        // Keep the shared popup anchor mounted so opening it never reparents
        // hovered rows. Only its non-focus-capturing portal content is transient.
        let tip = state.feedback.tooltip.as_ref();
        let position = tip.map_or((0, 0), |tip| tip.position);
        let above = position.1 >= ctx.viewport().h / 2;
        layers = layers.child(
            Canvas::new().passthrough(true).child_at(
                Rect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                },
                Element::from(
                    Popover::new()
                        .trigger(Spacer::new().width(Length::Px(0)).height(Length::Px(0)))
                        .anchor(Some((
                            position.0,
                            if above {
                                position.1
                            } else {
                                position.1.saturating_add(1)
                            },
                        )))
                        .placement(if above {
                            PopoverPlacement::AboveStart
                        } else {
                            PopoverPlacement::BelowStart
                        })
                        .auto_flip(false)
                        .open(tip.is_some() && state.dialog.is_none())
                        .capture_focus(false)
                        .auto_focus(false)
                        .min_trigger_width(false)
                        .max_width(Length::Px(80))
                        .content(
                            Element::from(
                                Frame::new()
                                    .border(true)
                                    .padding((0, 1))
                                    .style(colors.base().bg(colors.surface))
                                    .child(
                                        Text::new(
                                            tip.map_or_else(String::new, |tip| tip.text.clone()),
                                        )
                                        .overflow(Overflow::Wrap)
                                        .style(colors.base().bg(colors.surface)),
                                    ),
                            )
                            .key("status-tooltip"),
                        ),
                )
                .key("status-tooltip-anchor"),
            ),
        );
    }
    if let Some(dialog) = &state.dialog {
        layers = layers.child(dialog_view(ctx, dialog, colors));
    }
    ThemeProvider::new(colors.theme()).child(layers).into()
}

fn tab_bar(ctx: &Context<Cronk>, colors: Colors) -> Element {
    // Tabs only supports a single style per label. Compose spans to underline just the shortcut.
    let mut tabs = HStack::new().height(Length::Px(1)).gap(1);
    let mut offset = 0;
    let mut active_range = (0, 0);
    for (index, name) in ctx.state.tab_names().into_iter().enumerate() {
        let active = index == ctx.state.config.active_tab;
        let style = if active {
            colors.selected().fg(colors.accent).bold()
        } else {
            Style::new().fg(colors.muted).bg(colors.surface)
        };
        let style = interaction::style(ctx, &format!("workspace-tab-{index}"), style);
        let mut spans = vec![Span::new(" ")];
        if index < 4 {
            spans.push(Span::new(name[..1].to_owned()).style(Style::new().underline()));
            spans.push(Span::new(name[1..].to_owned()));
        } else {
            if index < 14 {
                let shortcut = if index == 13 { 0 } else { index - 3 };
                spans.push(
                    Span::new(shortcut.to_string())
                        .style(Style::new().fg(colors.accent).underline()),
                );
                spans.push(Span::new(" "));
            }
            spans.push(Span::new(name));
        }
        spans.push(Span::new(" "));
        let width = tui_lipan::utils::spans::line_width(&spans).min(u16::MAX as usize);
        if active {
            active_range = (offset, offset + width);
        }
        tabs = tabs.child(click(
            ctx,
            format!("workspace-tab-{index}"),
            HStack::new()
                .width(Length::Px(width as u16))
                .height(Length::Px(1))
                .style(style)
                .child(Text::from_spans(spans).style(style).height(Length::Px(1))),
            move || Msg::Tab(index),
        ));
        offset += width + 1;
    }
    ScrollView::new()
        .axis(ScrollAxis::Horizontal)
        .height(Length::Px(2))
        .padding((0, 1))
        .style(Style::new().bg(colors.surface))
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .scrollbar(false)
        .h_scrollbar(false)
        .reveal_horizontal_range(active_range.0, active_range.1)
        .child(tabs.width(Length::Px(
            offset.saturating_sub(1).min(u16::MAX as usize) as u16
        )))
        .key("workspace-tabs")
}

fn brand(state: &State, colors: Colors) -> Element {
    let mode = if state.demo {
        "DEMO · offline fixtures"
    } else {
        "GitLab workspace"
    };
    let identity =
        if state.user.id == 0 && state.user.username.is_empty() && state.user.name.is_empty() {
            None
        } else {
            Some(state.render_user(&state.user))
        };
    HStack::new()
        .height(Length::Px(1))
        .padding((0, 2))
        .style(Style::new().bg(colors.surface))
        .child(rich(
            vec![
                Span::new("CRONK").fg(colors.accent).bold(),
                Span::new(format!("   {mode}")).fg(colors.muted),
            ],
            Style::new(),
        ))
        .child(
            Text::new(if let Some(identity) = identity {
                format!(
                    "{identity}  ·  {}  ·  {}",
                    state.config.theme,
                    build_info::display_version()
                )
            } else {
                format!(
                    "{}  ·  {}",
                    state.config.theme,
                    build_info::display_version()
                )
            })
            .style(Style::new().fg(colors.muted))
            .height(Length::Px(1)),
        )
        .into()
}

fn breadcrumb(ctx: &Context<Cronk>, colors: Colors) -> Element {
    let state = &ctx.state;
    let mut row = HStack::new().height(Length::Px(1)).padding((0, 2));
    if state.scope != Scope::List {
        row = row.child(click(
            ctx,
            "breadcrumb-back",
            Text::new("‹ Back  ").style(interaction::style(
                ctx,
                "breadcrumb-back",
                colors.base().fg(colors.accent),
            )),
            || Msg::Back,
        ));
    }
    let mut spans = vec![Span::new(state.tab_name().to_owned()).fg(colors.muted)];
    if matches!(state.scope, Scope::Details | Scope::Section) {
        if let Some(key) = &state.config.route {
            spans.push(Span::new(format!(
                "  /  {}  /  {}",
                project_name(state, key.project),
                item_id(key)
            )));
        }
        if state.scope == Scope::Section {
            spans.push(Span::new(format!("  /  {}", state.section_name())).fg(colors.accent));
        }
    }
    row.child(rich(spans, colors.base())).into()
}

fn context_line(state: &State, colors: Colors) -> Element {
    let (heading, hint) = match state.scope {
        Scope::List if state.config.active_tab == 1 => (
            format!("{} PROJECTS", state.config.projects.len()),
            "Space toggles visibility · Enter opens project issues".to_owned(),
        ),
        Scope::List => {
            let query = state.query_text();
            (
                format!("{} ITEMS", state.visible_items().len()),
                if !query.is_empty() {
                    format!("Filter: {query}")
                } else if state.config.active_tab == 0 {
                    "Your open work · role and reason · ordered by urgency".into()
                } else {
                    "Across visible projects · most recently updated first".into()
                },
            )
        }
        Scope::Details => (
            state.section_name().to_uppercase(),
            "Tab / ↑ ↓ choose section · Enter focuses · PgUp/PgDn scroll".into(),
        ),
        Scope::Section => (
            state.section_name().to_uppercase(),
            section_hint(state.section_name()).into(),
        ),
    };
    HStack::new()
        .height(Length::Px(1))
        .padding((0, 2))
        .child(rich(
            vec![
                Span::new(heading).fg(colors.accent).bold(),
                Span::new(format!("   {hint}")).fg(colors.muted),
            ],
            colors.base(),
        ))
        .into()
}

fn section_hint(section: &str) -> &'static str {
    match section {
        "Fields" => "↑ ↓ choose a field · Enter edits",
        "Jobs" => "↑ ↓ choose a job · Enter expands · running jobs stay live",
        "Discussions" => "↑ ↓ choose a discussion · reply / resolve via commands",
        "Pipeline" => "All running jobs stream together · ↑ ↓ scroll",
        "Changes" => "Unified diff · ↑ ↓ scroll",
        _ => "↑ ↓ scroll · Esc returns to section navigation",
    }
}

fn main_list(ctx: &Context<Cronk>, colors: Colors) -> Element {
    let state = &ctx.state;
    let items = state.visible_items();
    let projects = state.config.active_tab == 1;
    let len = if projects {
        state.config.projects.len()
    } else {
        items.len()
    };
    let row_height = list_row_height(state.config.active_tab);
    let slots = list_height_for_tab(ctx.viewport().h, state.config.active_tab);
    let mut scroll = state.scroll.clone();
    scroll.normalize(len, slots);
    if len == 0 {
        let (title, help) = if state.config.projects.is_empty() {
            (
                "Your workspace is ready",
                "Add an existing GitLab project from the command palette.",
            )
        } else if !state.query_text().is_empty() {
            (
                "No items match this filter",
                "Change or clear the filter to see more work.",
            )
        } else if !state.list_pending.is_empty() {
            (
                "Loading your workspace",
                "Fetching work from the visible projects…",
            )
        } else {
            (
                "Nothing needs your attention",
                "Refresh, choose another tab, or check project visibility.",
            )
        };
        return empty_state(title, help, colors);
    }
    let mut rows = Vec::with_capacity(len);
    if projects {
        for (index, project) in state.config.projects.iter().enumerate() {
            rows.push(project_row(
                ctx,
                project,
                index,
                scroll.selected == index,
                colors,
            ));
        }
    } else {
        for (index, item) in items.iter().enumerate() {
            rows.push(work_row(ctx, item, index, scroll.selected == index, colors));
        }
    }
    let offset = scroll.offset;
    let scrollbar_key = format!("main-list-{}-scrollbar", state.config.active_tab);
    let list = ScrollView::new()
        .height(Length::Px(
            (slots * row_height).min(u16::MAX as usize) as u16
        ))
        .offset(offset * row_height)
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .scrollbar(true)
        .scrollbar_config(vertical_scrollbar(ctx, &scrollbar_key, colors))
        .scroll_wheel_multiplier(row_height as u16)
        .smooth_wheel_scroll(false)
        .estimated_child_height(row_height as u16)
        .on_scroll(
            ctx.link()
                .callback(|event: ScrollEvent| Msg::ListScroll(event.offset)),
        )
        .on_viewport_change(
            ctx.link()
                .callback(|_: ScrollViewportEvent| Msg::ListViewportChanged),
        )
        .children(rows);
    VStack::new()
        .child(interaction::scrollbar(
            ctx,
            scrollbar_key,
            Element::from(list).key(format!("main-list-{}", state.config.active_tab)),
        ))
        .into()
}

/// Keep the selection edge on the normal canvas, outside the row's selection/hover fill.
fn list_selection_line(content: impl Into<Element>, selected: bool, colors: Colors) -> Element {
    HStack::new()
        .height(Length::Px(1))
        .style(colors.base())
        .child(
            Text::new(if selected { "▕" } else { " " })
                .width(Length::Px(1))
                .height(Length::Px(1))
                .style(colors.base().fg(colors.accent)),
        )
        .child(content)
        .into()
}

fn work_row(
    ctx: &Context<Cronk>,
    item: &WorkItem,
    index: usize,
    selected: bool,
    colors: Colors,
) -> Element {
    let state = &ctx.state;
    let style = if selected {
        colors.selected()
    } else {
        colors.base()
    };
    let style = interaction::style(ctx, &format!("item-{}", item_key(&item.key)), style);
    let mut title = vec![Span::new(format!("{}  ", item_id(&item.key))).fg(colors.accent)];
    if item.draft {
        title.push(Span::new("Draft · ").fg(colors.yellow));
    }
    title.push(Span::new(item.title.clone()).bold());
    let mut labels = vec![Span::new("   ")];
    labels.extend(label_spans(&item.labels, colors));
    let mut labels_line = HStack::new()
        .height(Length::Px(1))
        .style(style)
        .child(Text::from_spans(labels).height(Length::Px(1)).style(style));
    if let Some(pipeline) = &item.pipeline {
        labels_line = labels_line
            .child(status_dot(
                ctx,
                format!("item-{}-pipeline-status", item_key(&item.key)),
                "Pipeline",
                &pipeline.status,
                style,
                colors,
            ))
            .child(
                Text::new(format!(" {}  ·  ", pipeline.status))
                    .height(Length::Px(1))
                    .style(style.fg(colors.status(&pipeline.status).1)),
            );
    }
    labels_line = labels_line.child(
        Text::new(state.render_user(&item.author))
            .width(Length::Flex(1))
            .height(Length::Px(1))
            .overflow(Overflow::Ellipsis)
            .style(style.fg(colors.muted)),
    );
    let mut attention = Vec::new();
    if state.config.active_tab == 0 {
        let roles = item.dashboard_roles(state.user.id);
        attention.push(Span::new("   Why: ").fg(colors.muted));
        for (index, reason) in item
            .attention_reasons(state.user.id)
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                attention.push(Span::new(", ").fg(colors.muted));
            }
            attention.push(Span::new(reason).fg(colors.accent));
        }
        if !roles.is_empty() {
            attention.push(Span::new("  ·  You: ").fg(colors.muted));
            attention.push(Span::new(roles.join(" + ")).fg(colors.accent));
        }
    }
    let marked = selected && state.scope == Scope::List;
    let attention_line = list_selection_line(rich(attention, style), marked, colors);
    let identity = project_name(state, item.key.project);
    let top = HStack::new()
        .height(Length::Px(1))
        .style(style)
        .child(Text::new(" ").width(Length::Px(1)).style(style))
        .child(status_dot(
            ctx,
            format!("item-{}-status", item_key(&item.key)),
            if item.key.kind == ItemKind::Issue {
                "Issue"
            } else {
                "Merge request"
            },
            &item.state,
            style,
            colors,
        ))
        .child(Text::new(" ").width(Length::Px(1)).style(style))
        .child(rich(title, style))
        .child(
            Text::new(format!(" {identity}  "))
                .style(Style::new().fg(colors.muted))
                .width(Length::Px((ctx.viewport().w / 3).min(32)))
                .height(Length::Px(1))
                .overflow(Overflow::Ellipsis),
        );
    let dashboard = state.config.active_tab == 0;
    let mut row = VStack::new()
        .height(Length::Px(if dashboard { 4 } else { 3 }))
        .style(colors.base())
        .child(list_selection_line(top, marked, colors))
        .child(list_selection_line(labels_line, marked, colors));
    row = if dashboard {
        row.child(attention_line).child(blank())
    } else {
        row.child(blank())
    };
    click(
        ctx,
        format!("item-{}", item_key(&item.key)),
        row,
        move || Msg::Activate(index),
    )
}

fn project_row(
    ctx: &Context<Cronk>,
    project: &Project,
    index: usize,
    selected: bool,
    colors: Colors,
) -> Element {
    let state = &ctx.state;
    let style = if selected {
        colors.selected()
    } else {
        colors.base()
    };
    let style = interaction::style(ctx, &format!("project-{}", project.id), style);
    let checkbox_style = interaction::style(ctx, &format!("project-visible-{}", project.id), style);
    let link = ctx.link().clone();
    let checkbox = click(
        ctx,
        format!("project-visible-{}", project.id),
        Text::new(if project.visible { " [x] " } else { " [ ] " }).style(checkbox_style.fg(
            if project.visible {
                colors.green
            } else {
                colors.muted
            },
        )),
        move || {
            link.send(Msg::Select(index));
            Msg::ToggleProject
        },
    );
    let count = state
        .items
        .iter()
        .filter(|item| item.key.project == project.id)
        .count();
    let top = HStack::new()
        .height(Length::Px(1))
        .style(style)
        .child(checkbox)
        .child(line(project_name(state, project.id), style.bold()))
        .child(Text::new(format!("{count} items  ")).style(Style::new().fg(colors.muted)));
    click(
        ctx,
        format!("project-{}", project.id),
        VStack::new()
            .height(Length::Px(3))
            .style(colors.base())
            .child(top)
            .child(rich(
                vec![
                    Span::new(format!("     {}  ·  ", project.path)).fg(colors.muted),
                    Span::new(if project.visible {
                        "visible"
                    } else {
                        "hidden from work lists"
                    })
                    .fg(if project.visible {
                        colors.green
                    } else {
                        colors.muted
                    }),
                ],
                style,
            ))
            .child(blank()),
        move || Msg::Activate(index),
    )
}

fn empty_state(title: &str, help: &str, colors: Colors) -> Element {
    VStack::new()
        .padding((1, 3))
        .gap(1)
        .child(line(title, Style::new().fg(colors.accent).bold()))
        .child(
            Text::new(help.to_owned())
                .style(Style::new().fg(colors.muted))
                .width(Length::Flex(1))
                .height(Length::Auto)
                .overflow(Overflow::Wrap),
        )
        .into()
}

fn scroll_content(ctx: &Context<Cronk>, children: Vec<Element>, colors: Colors) -> ScrollView {
    ScrollView::new()
        // Measure variable-height content before dragging so the thumb's range stays accurate.
        .virtualize(false)
        .offset(ctx.state.content_offset)
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .scrollbar(true)
        .scrollbar_config(vertical_scrollbar(ctx, "detail-scrollbar", colors))
        .scroll_wheel_multiplier(3)
        .smooth_wheel_scroll(false)
        .padding(content_padding(0))
        .style(colors.base())
        .on_scroll_to(ctx.link().callback(Msg::DetailScrolled))
        .children(children)
}

fn markdown(value: &str, colors: Colors) -> DocumentView {
    DocumentView::new(
        if value.trim().is_empty() {
            "*No description provided.*"
        } else {
            value
        }
        .to_owned(),
    )
    .markdown()
    .markdown_compact(true)
    .render_diagrams(false)
    .style(colors.base())
    // Passive document text is selectable, but is not a clickable control surface.
    .hover_style(Style::new())
    .border(false)
    .padding(0)
    .line_numbers(false)
    .wrap(true)
    .focusable(false)
    .tab_stop(false)
    .table_outer_frame(false)
}

fn detail(ctx: &Context<Cronk>, colors: Colors) -> Element {
    let state = &ctx.state;
    let Some(details) = state
        .details
        .as_ref()
        .filter(|d| state.config.route.as_ref() == Some(&d.item.key))
    else {
        return empty_state(
            if state.detail_pending.is_some() {
                "Loading details…"
            } else {
                "Details unavailable"
            },
            "The list is still available. Go back, or refresh to retry.",
            colors,
        );
    };
    detail_document(ctx, details, colors)
}

fn metadata(label: &str, value: impl Into<String>, colors: Colors) -> Element {
    let value = value.into();
    HStack::new()
        .style(colors.base())
        .height(Length::Auto)
        .gap(1)
        .child(
            Text::new(label.to_owned())
                .width(Length::Px(14))
                .style(Style::new().fg(colors.muted)),
        )
        .child(
            Text::new(if value.is_empty() {
                "—".to_owned()
            } else {
                value
            })
            .width(Length::Flex(1))
            .height(Length::Auto)
            .overflow(Overflow::Wrap)
            .style(Style::new().fg(colors.foreground)),
        )
        .into()
}

struct DetailRow {
    content: Element,
    key: Option<String>,
    focused: bool,
    on_click: Option<Box<dyn Fn() -> Msg>>,
    status: Option<(String, String, String)>,
}

impl<T: Into<Element>> From<T> for DetailRow {
    fn from(content: T) -> Self {
        Self {
            content: content.into(),
            key: None,
            focused: false,
            on_click: None,
            status: None,
        }
    }
}

impl DetailRow {
    fn with_status(mut self, key: impl Into<String>, subject: &str, status: &str) -> Self {
        self.status = Some((key.into(), subject.into(), status.into()));
        self
    }

    fn keyed(key: String, content: impl Into<Element>) -> Self {
        Self {
            key: Some(key),
            ..Self::from(content)
        }
    }

    fn interactive(
        key: String,
        content: impl Into<Element>,
        focused: bool,
        message: impl Fn() -> Msg + 'static,
    ) -> Self {
        Self {
            focused,
            on_click: Some(Box::new(message)),
            ..Self::keyed(key, content)
        }
    }
}

fn detail_selection_row(
    content: Element,
    status: Option<Element>,
    edge: Color,
    style: Style,
    colors: Colors,
) -> Element {
    HStack::new()
        .height(Length::Auto)
        .style(colors.base())
        .child(
            Divider::vertical()
                .ch(if edge == colors.background {
                    ' '
                } else {
                    '▕'
                })
                .style(colors.base().fg(edge)),
        )
        .child(
            HStack::new()
                .height(Length::Auto)
                .style(style)
                .child(Spacer::new().width(Length::Px(1)))
                .child(
                    VStack::new()
                        .width(Length::Px(1))
                        .height(Length::Auto)
                        .child(status.unwrap_or_else(|| {
                            Spacer::new()
                                .width(Length::Px(1))
                                .height(Length::Px(1))
                                .into()
                        })),
                )
                .child(Spacer::new().width(Length::Px(1)))
                .child(
                    VStack::new()
                        .height(Length::Auto)
                        .style(style)
                        .child(content),
                ),
        )
        .into()
}

fn detail_document(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Element {
    let state = &ctx.state;
    let mut content = Vec::new();
    for (index, section) in state.sections().iter().enumerate() {
        let selected = state.section_cursor == index;
        // Fade the broad section highlight away around the focused item on Enter,
        // and expand it again on Escape. Geometry and scroll anchors never animate.
        let broad =
            state.scope == Scope::Details || !matches!(*section, "Fields" | "Jobs" | "Discussions");
        let background = if state.config.animations {
            ctx.transition(
                format!("detail-focus-background-{index}"),
                if broad {
                    colors.selection
                } else {
                    colors.background
                },
                TransitionConfig {
                    duration: std::time::Duration::from_millis(160),
                    ..TransitionConfig::default()
                },
            )
        } else if broad {
            colors.selection
        } else {
            colors.background
        };
        let edge = if state.config.animations {
            ctx.transition(
                format!("detail-focus-edge-{index}"),
                if broad {
                    colors.accent
                } else {
                    colors.background
                },
                TransitionConfig {
                    duration: std::time::Duration::from_millis(160),
                    ..TransitionConfig::default()
                },
            )
        } else if broad {
            colors.accent
        } else {
            colors.background
        };
        let section_colors = Colors {
            background: if selected {
                background
            } else {
                colors.background
            },
            ..colors
        };
        let edge = if selected { edge } else { colors.background };
        let style = interaction::style(
            ctx,
            &format!("detail-section-{index}"),
            section_colors.base(),
        );
        content.push(click(
            ctx,
            format!("detail-section-{index}"),
            detail_selection_row(
                HStack::new()
                    .height(Length::Px(1))
                    .style(style)
                    .child(Text::new(format!("{section} ")).style(style.fg(colors.accent).bold()))
                    .child(Divider::horizontal().style(style.fg(colors.muted)))
                    .into(),
                None,
                edge,
                style,
                colors,
            ),
            move || Msg::DetailSection(index),
        ));
        let children: Vec<DetailRow> = match *section {
            "Fields" => fields(ctx, details, section_colors),
            "Description" => vec![
                markdown(&details.item.description, section_colors)
                    .height(Length::Auto)
                    .scrollbar(false)
                    .scroll_wheel(false)
                    .into(),
            ],
            "Activity" => {
                let mut notes: Vec<_> = details.notes.iter().collect();
                notes.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
                if notes.is_empty() {
                    vec![line("No activity yet.", style.fg(colors.muted)).into()]
                } else {
                    notes
                        .into_iter()
                        .map(|note| note_view(note, &ctx.state.user_formatter, section_colors))
                        .collect()
                }
            }
            "Pipeline" => pipeline(ctx, details, section_colors),
            "Jobs" => jobs(ctx, details, section_colors),
            "Discussions" => discussions(ctx, details, section_colors),
            "Changes" => changes(details, section_colors),
            _ => Vec::new(),
        };
        // Keep stable row keys on direct children: native scroll anchoring then
        // survives newly inserted notes and collapsing job panels above the viewport.
        for (row_index, child) in children.into_iter().enumerate() {
            let key = child
                .key
                .unwrap_or_else(|| format!("detail-section-{index}-row-{row_index}"));
            let row_style = if child.focused {
                colors.selected()
            } else {
                section_colors.base()
            };
            let row_style = interaction::style(ctx, &key, row_style);
            let status = child.status.map(|(key, subject, status)| {
                status_dot(ctx, key, &subject, &status, row_style, colors)
            });
            let row = detail_selection_row(
                child.content,
                status,
                if child.focused { colors.accent } else { edge },
                row_style,
                colors,
            );
            content.push(if let Some(message) = child.on_click {
                click(ctx, key, row, message)
            } else {
                row.key(key)
            });
        }
        content.push(blank().key(format!("detail-section-gap-{index}")));
    }
    let target = state.reveal_content.then(|| state.detail_target_key());
    let origin = state.content_offset;
    let route = details.item.key.clone();
    let epoch = state.detail_epoch;
    let callback_target = target.clone();
    let mut scroll = scroll_content(ctx, content, colors).on_viewport_change(ctx.link().callback(
        move |event: ScrollViewportEvent| {
            Msg::DetailViewport(
                Box::new(event),
                callback_target.clone(),
                origin,
                route.clone(),
                epoch,
            )
        },
    ));
    if let Some(target) = target {
        let measured = state
            .detail_viewport
            .as_ref()
            .filter(|(route, _)| *route == details.item.key)
            .and_then(|(_, event)| {
                event
                    .visible
                    .iter()
                    .find(|child| child.key.as_ref() == Some(&target.clone().into()))
                    .map(|child| (child.content_rect.y.max(0) as usize, event.metrics))
            });
        if let Some((selected, metrics)) = measured {
            let offset = if state.scope == Scope::Details {
                detail_section_offset(selected, origin, metrics)
            } else {
                let mut boundary = crate::scroll::BoundaryScroll {
                    selected,
                    offset: origin,
                };
                boundary.normalize(metrics.len, metrics.visible);
                boundary.offset
            };
            scroll = scroll.offset(offset);
        } else {
            // Unseen targets need a layout pass. The viewport callback then applies
            // the quarter-page band using actual wrapped heights and the original offset.
            scroll = scroll.scroll_to_key(target);
        }
    } else if let Some(offset) = state.detail_offset_request {
        // A semantic lookup can move the native widget even when the corrected
        // numeric prop equals its previous value (notably zero). Explicitly
        // synchronize once so remembered tab anchors also retain the correction.
        scroll = scroll.scroll_to_key_offset("detail-section-0", offset);
    }
    interaction::scrollbar(
        ctx,
        "detail-scrollbar",
        Element::from(scroll).key(format!(
            "detail-{}-{}",
            state.config.active_tab,
            item_key(&details.item.key)
        )),
    )
}

fn fields(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Vec<DetailRow> {
    let item = &details.item;
    let mut content = vec![
        DetailRow::from(rich(
            vec![
                Span::new(item.state.clone()).fg(colors.status(&item.state).1),
                Span::new(format!("   {}", item_id(&item.key))).fg(colors.accent),
            ],
            colors.base(),
        ))
        .with_status(
            "detail-item-status",
            if item.key.kind == ItemKind::Issue {
                "Issue"
            } else {
                "Merge request"
            },
            &item.state,
        ),
        metadata(
            "Project path",
            ctx.state
                .project(details.item.key.project)
                .map_or_else(|| details.item.key.project.to_string(), |p| p.path.clone()),
            colors,
        )
        .into(),
        blank().into(),
    ];
    for (index, (label, _, value)) in ctx.state.fields().into_iter().enumerate() {
        let selected = ctx.state.scope == Scope::Section
            && ctx.state.section_name() == "Fields"
            && ctx.state.config.field == index;
        let style = if selected {
            colors.selected()
        } else {
            colors.base()
        };
        let style = interaction::style(ctx, &format!("edit-field-{index}"), style);
        let mut spans = vec![Span::new(format!("{label:<19} ")).fg(colors.accent)];
        if label == "Labels" {
            spans.extend(label_spans(&details.item.labels, colors));
        } else if label == "Assignees" {
            let users = details
                .item
                .assignees
                .iter()
                .map(|user| ctx.state.render_user(user))
                .collect::<Vec<_>>()
                .join(", ");
            spans.push(Span::new(if users.is_empty() { "—" } else { &users }));
        } else if label == "Reviewers" {
            let users = details
                .item
                .reviewers
                .iter()
                .map(|user| ctx.state.render_user(user))
                .collect::<Vec<_>>()
                .join(", ");
            spans.push(Span::new(if users.is_empty() { "—" } else { &users }));
        } else {
            spans.push(Span::new(if value.is_empty() {
                "—".into()
            } else {
                value
            }));
        }
        content.push(DetailRow::interactive(
            format!("edit-field-{index}"),
            VStack::new()
                .height(Length::Auto)
                .style(style)
                .child(
                    Text::from_spans(spans)
                        .width(Length::Flex(1))
                        .height(Length::Auto)
                        .overflow(Overflow::Wrap)
                        .style(style),
                )
                .child(line(
                    if selected {
                        "   Enter or click to edit"
                    } else {
                        ""
                    },
                    Style::new().fg(colors.muted),
                )),
            selected,
            move || Msg::Field(index),
        ));
    }
    content.push(metadata("Author", ctx.state.render_user(&item.author), colors).into());
    content.push(metadata("Updated", item.updated_at.clone(), colors).into());
    if item.key.kind == ItemKind::MergeRequest {
        content.push(
            metadata(
                "Branches",
                format!("{} → {}", item.source_branch, item.target_branch),
                colors,
            )
            .into(),
        );
        content.push(
            metadata(
                "Review",
                format!(
                    "{} discussions · {} unresolved · {} changed files",
                    details.discussions.len(),
                    item.unresolved
                        .map_or_else(|| "unknown".into(), |n| n.to_string()),
                    details.diffs.len()
                ),
                colors,
            )
            .into(),
        );
    }
    content.push(metadata("Web URL", item.web_url.clone(), colors).into());
    for warning in &details.warnings {
        content.push(
            DetailRow::from(
                Text::new(warning.clone())
                    .width(Length::Flex(1))
                    .height(Length::Auto)
                    .overflow(Overflow::Wrap)
                    .style(colors.base().fg(colors.yellow)),
            )
            .with_status(
                format!("detail-warning-{}-status", content.len()),
                "Details warning",
                "warning",
            ),
        );
    }
    content
}

fn note_view(note: &Note, formatter: &UserFormatter, colors: Colors) -> DetailRow {
    let mut header = vec![
        Span::new(formatter.render(&note.author))
            .fg(colors.cyan)
            .bold(),
        Span::new(format!("   {}", note.created_at)).fg(colors.muted),
    ];
    if note.system {
        header.push(Span::new("  ·  system").fg(colors.muted));
    }
    let status = if !note.resolvable {
        "comment"
    } else if note.resolved {
        "resolved"
    } else {
        "unresolved"
    };
    if note.resolvable {
        header.push(Span::new("  ·  "));
        header.push(Span::new(status).fg(colors.status(status).1));
    }
    DetailRow::keyed(
        format!("note-{}", note.id),
        VStack::new()
            .height(Length::Auto)
            .style(colors.base())
            .child(rich(header, colors.base()))
            .child(
                markdown(&note.body, colors)
                    .height(Length::Auto)
                    .scrollbar(false)
                    .scroll_wheel(false),
            )
            .child(blank()),
    )
    .with_status(
        format!("note-{}-status", note.id),
        if note.system {
            "System note"
        } else {
            "Comment"
        },
        status,
    )
}

fn pipeline(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Vec<DetailRow> {
    let mut content = Vec::new();
    if let Some(pipeline) = &details.item.pipeline {
        content.push(
            DetailRow::from(rich(
                vec![
                    Span::new(format!("Pipeline #{}   ", pipeline.id)).bold(),
                    Span::new(pipeline.status.clone()).fg(colors.status(&pipeline.status).1),
                ],
                colors.base(),
            ))
            .with_status("detail-pipeline-status", "Pipeline", &pipeline.status),
        );
        content.push(metadata("Web URL", pipeline.web_url.clone(), colors).into());
    } else {
        content.push(
            line(
                "No pipeline is attached to this merge request.",
                Style::new().fg(colors.muted),
            )
            .into(),
        );
    }
    let count = |status: &str| details.jobs.iter().filter(|j| j.status == status).count();
    let mut summary = HStack::new().height(Length::Px(1)).style(colors.base());
    for (status, label, count) in [
        ("running", "running", count("running")),
        ("pending", "pending", count("pending") + count("created")),
        ("success", "passed", count("success")),
        ("failed", "failed", count("failed")),
    ] {
        summary = summary
            .child(status_dot(
                ctx,
                format!("pipeline-summary-{status}-status"),
                "Jobs",
                status,
                colors.base(),
                colors,
            ))
            .child(
                Text::new(format!(" {count} {label}   "))
                    .height(Length::Px(1))
                    .style(colors.base().fg(colors.status(status).1)),
            );
    }
    content.push(
        summary
            .child(
                Text::new(format!("{} total", details.jobs.len()))
                    .height(Length::Px(1))
                    .style(colors.base().fg(colors.muted)),
            )
            .into(),
    );
    content
}

fn job_header(job: &Job, expanded: bool, colors: Colors) -> Vec<Span> {
    let mut spans = vec![
        Span::new(if expanded { "− " } else { "+ " }).fg(colors.muted),
        Span::new(job.name.clone()).bold(),
        Span::new(format!("  {}  #{}  ", job.stage, job.id)).fg(colors.muted),
        Span::new(job.status.clone()).fg(colors.status(&job.status).1),
    ];
    if job.allow_failure {
        spans.push(Span::new("  ·  allowed failure").fg(colors.yellow));
    }
    spans
}

fn job_panel(ctx: &Context<Cronk>, job: &Job, lines: usize, colors: Colors) -> Element {
    let mut panel = VStack::new()
        .height(Length::Auto)
        .style(Style::new().bg(colors.surface));

    match ctx.state.traces.get(&job.id) {
        Some(trace) => {
            if let Some(error) = &trace.error {
                panel = panel.child(line(
                    format!("  Trace error: {error}"),
                    Style::new().fg(colors.red),
                ));
            }
            if trace.text.is_empty() {
                panel = panel.child(line(
                    if trace.finished {
                        "  Trace is empty."
                    } else {
                        "  Waiting for log output…"
                    },
                    Style::new().fg(colors.muted),
                ));
            } else {
                let tail = trace_tail(&trace.text, lines);
                let height = tail.lines().count().max(1).min(u16::MAX as usize) as u16;
                panel = panel.child(
                    Text::from_ansi(&tail)
                        .width(Length::Flex(1))
                        .height(Length::Px(height))
                        .overflow(Overflow::Clip)
                        .style(Style::new().fg(colors.foreground).bg(colors.surface)),
                );
            }
            panel = if job.running() {
                let style = colors.base().bg(colors.surface).fg(colors.cyan);
                panel.child(
                    HStack::new()
                        .height(Length::Px(1))
                        .style(style)
                        .child(Text::new("  ").style(style))
                        .child(dot_tooltip(
                            ctx,
                            format!("job-{}-live-status", job.id),
                            '◐',
                            colors.cyan,
                            "Live log: Output is polled while this job is running",
                            style,
                        ))
                        .child(Text::new(" LIVE · trailing output").style(style)),
                )
            } else {
                panel.child(line(
                    if trace.finished {
                        "  End of trace"
                    } else {
                        "  Loading trace…"
                    },
                    Style::new().fg(colors.muted),
                ))
            };
        }
        None => {
            panel = panel.child(line("  Waiting for trace…", Style::new().fg(colors.muted)));
        }
    }
    Element::from(panel).key(format!("trace-{}", job.id))
}

fn trace_tail(text: &str, count: usize) -> String {
    // Only retain the visible tail, avoiding a copy/ANSI parse of an ever-growing entire trace.
    let mut lines: Vec<_> = text.lines().rev().take(count).collect();
    lines.reverse();
    lines.join("\n")
}

fn jobs(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Vec<DetailRow> {
    if details.jobs.is_empty() {
        return vec![
            line(
                "No jobs in this pipeline yet.",
                colors.base().fg(colors.muted),
            )
            .into(),
        ];
    }
    let mut content = Vec::new();
    for (index, job) in details.jobs.iter().enumerate() {
        let selected = ctx.state.scope == Scope::Section
            && ctx.state.section_name() == "Jobs"
            && ctx.state.config.field == index;
        let expanded = job.running() || ctx.state.expanded.contains(&job.id);
        let id = job.id;
        let link = ctx.link().clone();
        content.push(
            DetailRow::interactive(
                format!("job-{id}"),
                rich(
                    job_header(job, expanded, colors),
                    interaction::style(
                        ctx,
                        &format!("job-{id}"),
                        if selected {
                            colors.selected()
                        } else {
                            colors.base()
                        },
                    ),
                ),
                selected,
                move || {
                    link.send(Msg::Section(3));
                    link.send(Msg::Select(index));
                    Msg::ToggleJob(id)
                },
            )
            .with_status(
                format!("job-{id}-status"),
                if job.allow_failure {
                    "Job (failure is allowed)"
                } else {
                    "Job"
                },
                &job.status,
            ),
        );
        if expanded {
            content.push(DetailRow::keyed(
                format!("trace-{}", job.id),
                job_panel(ctx, job, if job.running() { 8 } else { 18 }, colors),
            ));
        }
        content.push(blank().into());
    }
    content
}

fn discussion_state(discussion: &Discussion) -> &'static str {
    let resolvable: Vec<_> = discussion.notes.iter().filter(|n| n.resolvable).collect();
    if resolvable.is_empty() {
        "comment"
    } else if resolvable.iter().all(|n| n.resolved) {
        "resolved"
    } else {
        "unresolved"
    }
}

fn discussions(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Vec<DetailRow> {
    if details.discussions.is_empty() {
        return vec![line("No discussions yet.", colors.base().fg(colors.muted)).into()];
    }
    let mut content = Vec::new();
    for (index, discussion) in details.discussions.iter().enumerate() {
        let selected = ctx.state.scope == Scope::Section
            && ctx.state.section_name() == "Discussions"
            && ctx.state.config.field == index;
        let style = if selected {
            colors.selected()
        } else {
            colors.base()
        };
        let style = interaction::style(ctx, &format!("discussion-{}", discussion.id), style);
        let author = discussion.notes.first().map_or_else(
            || "unknown".into(),
            |note| ctx.state.render_user(&note.author),
        );
        let link = ctx.link().clone();
        content.push(
            DetailRow::interactive(
                format!("discussion-{}", discussion.id),
                rich(
                    vec![
                        Span::new(format!("{author}  ")).fg(colors.cyan),
                        Span::new(discussion_state(discussion))
                            .fg(colors.status(discussion_state(discussion)).1),
                        Span::new(format!("  ·  {} notes", discussion.notes.len()))
                            .fg(colors.muted),
                    ],
                    style,
                ),
                selected,
                move || {
                    link.send(Msg::Section(4));
                    Msg::Select(index)
                },
            )
            .with_status(
                format!("discussion-{}-status", discussion.id),
                "Discussion",
                discussion_state(discussion),
            ),
        );
        content.extend(
            discussion
                .notes
                .iter()
                .map(|note| note_view(note, &ctx.state.user_formatter, colors)),
        );
        content.push(blank().into());
    }
    content
}

fn changes(details: &Details, colors: Colors) -> Vec<DetailRow> {
    if details.diffs.is_empty() {
        return vec![line("No changes available.", colors.base().fg(colors.muted)).into()];
    }
    let mut content = Vec::new();
    for (index, diff) in details.diffs.iter().enumerate() {
        let path = if diff.old_path == diff.new_path {
            diff.new_path.clone()
        } else {
            format!("{} → {}", diff.old_path, diff.new_path)
        };
        let kind = if diff.new_file {
            "added"
        } else if diff.deleted_file {
            "deleted"
        } else {
            "modified"
        };
        content.push(DetailRow::keyed(
            format!("diff-file-{index}"),
            line(
                format!("{path}  ·  {kind}"),
                Style::new().fg(colors.accent).bg(colors.surface).bold(),
            ),
        ));
        if diff.too_large || diff.collapsed {
            content.push(DetailRow::from(line(if diff.too_large { "GitLab omitted this diff because it is too large." }
                else { "GitLab returned a collapsed diff; open the merge request URL for the full patch." }, Style::new().fg(colors.yellow)))
                .with_status(format!("diff-{index}-warning-status"), "Diff warning", "warning"));
        }
        if diff.diff.is_empty() && !diff.too_large && !diff.collapsed {
            content.push(
                line(
                    "No textual diff (the file may be binary).",
                    Style::new().fg(colors.muted),
                )
                .into(),
            );
        }
        for (number, text) in diff.diff.lines().enumerate() {
            let foreground = if text.starts_with("@@") {
                colors.cyan
            } else if text.starts_with('+') {
                colors.green
            } else if text.starts_with('-') {
                colors.red
            } else if text.starts_with("\\ No newline") {
                colors.yellow
            } else {
                colors.muted
            };
            // Wrapping preserves the complete patch on narrow terminals without a second scroll axis.
            content.push(DetailRow::keyed(
                format!("diff-{index}-line-{number}"),
                Text::new(text.to_owned())
                    .width(Length::Flex(1))
                    .height(Length::Auto)
                    .overflow(Overflow::Wrap)
                    .style(colors.base().fg(foreground)),
            ));
        }
        content.push(blank().into());
    }
    content
}

fn footer(ctx: &Context<Cronk>, colors: Colors) -> Element {
    let state = &ctx.state;
    let busy = state.mutation_pending
        || !state.list_pending.is_empty()
        || state.detail_pending.is_some()
        || !state.trace_pending.is_empty();
    let scope = match state.scope {
        Scope::List => "LIST",
        Scope::Details => "DETAILS",
        Scope::Section => "SECTION",
    };
    let mut status = vec![
        Span::new(format!(" {scope} ")).fg(colors.accent).bold(),
        Span::new("  "),
    ];
    if state.demo {
        status.push(Span::new("DEMO  ").fg(colors.yellow).bold());
    }
    let style = colors.base().bg(colors.surface);
    let mut status_line = HStack::new()
        .height(Length::Px(1))
        .style(style)
        .child(Text::from_spans(status).height(Length::Px(1)).style(style));
    if busy {
        status_line = status_line
            .child(dot_tooltip(
                ctx,
                "sync-status",
                if state.tick.is_multiple_of(2) {
                    '●'
                } else {
                    '○'
                },
                colors.cyan,
                "Syncing: Requests to GitLab are in progress",
                style,
            ))
            .child(
                Text::new(" Syncing  ")
                    .height(Length::Px(1))
                    .style(style.fg(colors.cyan)),
            );
    }
    status_line = status_line.child(
        Text::new(if let Some(error) = &state.error {
            format!("Error: {error}")
        } else {
            state.status.clone()
        })
        .width(Length::Flex(1))
        .height(Length::Px(1))
        .overflow(Overflow::Ellipsis)
        .style(style.fg(if state.error.is_some() {
            colors.red
        } else {
            colors.muted
        })),
    );
    let shortcuts = match state.scope {
        Scope::List => "↑ ↓ move   Enter open   / filter   : commands   r refresh   ? help",
        Scope::Details => " ↑ ↓ section   Enter focus   PgUp/PgDn scroll   Esc back   : commands",
        Scope::Section => {
            " ↑ ↓ navigate   Enter act   Esc sections   PgUp/PgDn scroll   : commands"
        }
    };
    VStack::new()
        .height(Length::Px(2))
        .style(Style::new().bg(colors.surface))
        .child(status_line)
        .child(line(
            format!(" D/P/I/M tabs   1–0 views   {shortcuts}"),
            Style::new().fg(colors.muted).bg(colors.surface),
        ))
        .into()
}

fn dialog_key(
    key: KeyEvent,
    index: usize,
    selected: usize,
    fields: usize,
    multiline: bool,
    commands: bool,
) -> Option<Msg> {
    match key.code {
        KeyCode::Esc => Some(Msg::CloseDialog),
        KeyCode::Enter => Some(Msg::Submit),
        KeyCode::Char('j') if key.mods.ctrl && multiline => Some(Msg::Newline(index)),
        KeyCode::Down | KeyCode::Tab if commands && !key.mods.shift => Some(Msg::Move(1)),
        KeyCode::Up | KeyCode::Tab | KeyCode::BackTab if commands => Some(Msg::Move(-1)),
        // Palette arrows and Tab belong to the parent's option navigation, not form focus.
        KeyCode::Tab | KeyCode::BackTab if !commands && fields > 0 => {
            let backwards = key.code == KeyCode::BackTab || key.mods.shift;
            Some(Msg::DialogField(if backwards {
                (selected + fields - 1) % fields
            } else {
                (selected + 1) % fields
            }))
        }
        _ => None,
    }
}

fn dialog_view(ctx: &Context<Cronk>, dialog: &Dialog, colors: Colors) -> Element {
    let commands = matches!(dialog.kind, DialogKind::Commands);
    // Keep reveal targets in separate child subtrees so keyboard focus can scroll to each one.
    let mut body = ScrollView::new()
        .height(Length::Auto)
        .style(Style::new().fg(colors.foreground).bg(colors.surface));

    if !dialog.help.is_empty() {
        body = body
            .child(
                Text::new(dialog.help.clone())
                    .width(Length::Flex(1))
                    .height(Length::Auto)
                    .overflow(Overflow::Wrap)
                    .style(Style::new().fg(colors.muted)),
            )
            .child(blank());
    }
    for (index, field) in dialog.fields.iter().enumerate() {
        let selected = dialog.selected;
        let count = dialog.fields.len();
        let multiline = field.multiline;
        let lookup = field.completion.is_some();
        let interceptor = ctx.link().key_handler(move |key| {
            if lookup {
                match key.code {
                    KeyCode::Down => return Some(Msg::LookupMove(index, 1)),
                    KeyCode::Up => return Some(Msg::LookupMove(index, -1)),
                    KeyCode::Enter => return Some(Msg::LookupEnter(index)),
                    _ => {}
                }
            }
            dialog_key(key, index, selected, count, multiline, commands)
        });
        let field_key = format!("dialog-field-{index}");
        let normal = interaction::style(ctx, &field_key, colors.base());
        let focus = interaction::style(ctx, &field_key, colors.selected());
        let activate = interaction::activation(ctx, field_key, move || Msg::DialogField(index));
        let editor: Element = if field.uses_editor() {
            let mut editor = TextArea::bound(&field.editor)
                .height(if field.multiline {
                    Length::Px((ctx.viewport().h / 5).clamp(3, 8))
                } else {
                    Length::Px(1)
                })
                .border(false)
                .padding(if field.multiline {
                    content_padding(1)
                } else {
                    (0, 1).into()
                })
                .scrollbar(field.multiline)
                .scrollbar_config(vertical_scrollbar(
                    ctx,
                    &format!("dialog-field-{index}-scrollbar"),
                    colors,
                ))
                .line_numbers(false)
                .wrap(field.multiline)
                .style(normal)
                .focus_style(focus)
                .focus_content_style(focus)
                .selection_style(colors.selected())
                .caret_color(colors.accent)
                .key_interceptor(interceptor)
                .insert_tab(false)
                .on_click(activate)
                .on_change(ctx.link().callback(move |event| Msg::Editor(index, event)));
            if let Some(c) = field
                .completion
                .as_ref()
                .filter(|c| c.kind == LookupKind::Labels)
            {
                editor = editor.color_strategy(LabelColorStrategy {
                    swatches: c.swatches.clone(),
                    colors,
                });
            }
            editor.into()
        } else {
            Input::bound(&field.input)
                .height(Length::Px(1))
                .border(false)
                .padding((0, 1))
                .style(normal)
                .focus_style(focus)
                .focus_content_style(focus)
                .selection_style(colors.selected())
                .caret_color(colors.accent)
                .key_interceptor(interceptor)
                .on_click(activate)
                .on_change(ctx.link().callback(move |event| Msg::Input(index, event)))
                .into()
        };
        body = body
            .child(
                VStack::new()
                    .height(Length::Auto)
                    .child(line(
                        field.label.clone(),
                        Style::new().fg(if commands || selected == index {
                            colors.accent
                        } else {
                            colors.muted
                        }),
                    ))
                    .child(if field.multiline {
                        interaction::scrollbar(
                            ctx,
                            format!("dialog-field-{index}-scrollbar"),
                            editor.key(format!("dialog-field-{index}")),
                        )
                    } else {
                        editor.key(format!("dialog-field-{index}"))
                    })
                    .child(if selected == index {
                        field
                            .completion
                            .as_ref()
                            .filter(|c| c.open)
                            .map(|c| completion_view(ctx, index, c, colors))
                            .unwrap_or_else(|| VStack::new().height(Length::Px(0)).into())
                    } else {
                        VStack::new().height(Length::Px(0)).into()
                    }),
            )
            .child(blank());
    }
    if commands {
        let options = ctx.state.command_options();
        if options.is_empty() {
            body = body.child(line("No matching commands.", Style::new().fg(colors.muted)));
        }
        for (index, (label, _)) in options.iter().enumerate() {
            body = body.child(dialog_option(
                ctx,
                index,
                label,
                dialog.selected == index,
                colors,
                true,
            ));
        }
    } else if matches!(dialog.kind, DialogKind::Themes) {
        for (index, (name, hint)) in [
            ("Midnight", "Cool ink · violet · glacier blue"),
            ("Dracula", "Charcoal · orchid · electric green"),
            ("Light", "Paper · indigo · forest green"),
        ]
        .iter()
        .enumerate()
        {
            body = body
                .child(dialog_option(
                    ctx,
                    index,
                    &format!("{name:<10} {hint}"),
                    dialog.selected == index,
                    colors,
                    false,
                ))
                .child(blank());
        }
        if [
            &ctx.state.config.colors.background,
            &ctx.state.config.colors.surface,
            &ctx.state.config.colors.selection,
            &ctx.state.config.colors.foreground,
            &ctx.state.config.colors.muted,
            &ctx.state.config.colors.accent,
        ]
        .iter()
        .any(|color| color.is_some())
        {
            body = body.child(line(
                "Custom config.colors overrides remain active.",
                Style::new().fg(colors.yellow),
            ));
        }
    } else if matches!(dialog.kind, DialogKind::Help) {
        body = body.child(
            markdown(HELP, colors)
                .height(Length::Auto)
                .scrollbar(false)
                .scroll_wheel(false),
        );
    } else if matches!(dialog.kind, DialogKind::Confirm(_)) {
        body = body.child(line(
            "This action requires confirmation.",
            Style::new().fg(colors.yellow),
        ));
    }
    if let Some(error) = &dialog.error {
        body = body.child(blank()).child(
            Text::new(format!("Error: {error}"))
                .width(Length::Flex(1))
                .height(Length::Auto)
                .overflow(Overflow::Wrap)
                .style(Style::new().fg(colors.red)),
        );
    }
    let hint = if commands {
        "↑ ↓ / Tab choose · Enter run · Esc cancel"
    } else if matches!(dialog.kind, DialogKind::Themes) {
        "↑ ↓ choose · Enter apply · Esc cancel"
    } else if dialog.fields.iter().any(|f| f.multiline) {
        "Enter submit · Tab next · Ctrl+J newline · Esc cancel"
    } else if matches!(dialog.kind, DialogKind::Help) {
        "Esc closes help"
    } else {
        "Enter confirm · Tab next field · Esc cancel"
    };
    let mut actions = HStack::new()
        .height(Length::Px(1))
        .gap(2)
        .child(line(hint, Style::new().fg(colors.muted)));
    if !matches!(
        dialog.kind,
        DialogKind::Help | DialogKind::Commands | DialogKind::Themes
    ) {
        actions = actions.child(click(
            ctx,
            "dialog-submit",
            Text::new(" Confirm ").style(interaction::style(
                ctx,
                "dialog-submit",
                colors.selected().fg(colors.accent).bold(),
            )),
            || Msg::Submit,
        ));
    }
    actions = actions.child(click(
        ctx,
        "dialog-close",
        Text::new(" Close ").style(interaction::style(
            ctx,
            "dialog-close",
            Style::new().fg(colors.muted).bg(colors.surface),
        )),
        || Msg::CloseDialog,
    ));
    let mut scroll = body
        .child(blank())
        .child(actions)
        .virtualize(false)
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .scrollbar(true)
        .scrollbar_config(vertical_scrollbar(ctx, "dialog-scrollbar", colors))
        .smooth_wheel_scroll(false)
        .on_scroll(ctx.link().callback(|_: ScrollEvent| Msg::DialogScrolled));
    if dialog.reveal_selection {
        if commands || matches!(dialog.kind, DialogKind::Themes) {
            scroll = scroll.reveal_key(format!("dialog-option-{}", dialog.selected));
        } else if !dialog.fields.is_empty() {
            scroll = scroll.reveal_key(format!("dialog-field-{}", dialog.selected));
        }
    }
    let body = interaction::scrollbar(
        ctx,
        "dialog-scrollbar",
        Element::from(scroll)
            .max_height(Length::Px(ctx.viewport().h.saturating_sub(6).max(1)))
            .key("dialog-scroll"),
    );
    let body = if dialog.fields.is_empty() {
        // Keep modal keyboard handling outside the viewport so scrolling cannot clip its focus.
        VStack::new()
            .height(Length::Auto)
            .child(
                KeyCapture::new()
                    .on_key(ctx.link().key_handler(|key| match key.code {
                        KeyCode::Enter => Some(Msg::Submit),
                        KeyCode::Esc => Some(Msg::CloseDialog),
                        KeyCode::Down | KeyCode::Tab if !key.mods.shift => Some(Msg::Move(1)),
                        KeyCode::Up | KeyCode::Tab | KeyCode::BackTab => Some(Msg::Move(-1)),
                        _ => None,
                    }))
                    .key("dialog-actions"),
            )
            .child(body)
            .into()
    } else {
        body
    };
    Element::from(
        Modal::new()
            .title(dialog.title.clone())
            .width(Length::Px(ctx.viewport().w.saturating_sub(4).clamp(1, 88)))
            .height(Length::Auto)
            .max_height(Length::Px(ctx.viewport().h.saturating_sub(2).max(1)))
            .border_style(BorderStyle::Rounded)
            .padding(1)
            .frame_style(Style::new().fg(colors.accent).bg(colors.surface))
            .focus_style(Style::new().fg(colors.accent).bg(colors.surface))
            .title_style(Style::new().fg(colors.accent).bold())
            .dismiss_on_escape(false)
            .on_close(ctx.link().callback(|_| Msg::CloseDialog))
            .child(body),
    )
    .key("dialog")
}

fn completion_view(ctx: &Context<Cronk>, index: usize, c: &Completion, colors: Colors) -> Element {
    let message = if c.pending {
        Some("Searching GitLab…")
    } else if let Some(error) = &c.error {
        Some(error.as_str())
    } else if c.options.is_empty() {
        Some(if c.kind == LookupKind::Labels {
            "No matches. Refine the label name."
        } else {
            "No matches. Refine the name, or enter an ID."
        })
    } else {
        None
    };
    if let Some(message) = message {
        return Text::new(message.to_owned())
            .height(Length::Auto)
            .width(Length::Flex(1))
            .overflow(Overflow::Wrap)
            .style(Style::new().fg(if c.error.is_some() {
                colors.red
            } else {
                colors.muted
            }))
            .into();
    }
    let mut rows = Vec::new();
    for (option_index, option) in c.options.iter().enumerate() {
        let style = if c.selected == option_index {
            colors.selected()
        } else {
            Style::new().fg(colors.foreground).bg(colors.surface)
        };
        let style = interaction::style(ctx, &format!("lookup-{index}-{}", option.id), style);
        let epoch = c.epoch;
        let header: Element = if c.kind == LookupKind::Labels {
            let label = Label {
                name: option.label.clone(),
                color: option.color.clone(),
                text_color: option.text_color.clone(),
            };
            let mut spans = vec![
                Span::new(format!(
                    " {} ",
                    if c.selected == option_index {
                        "›"
                    } else {
                        " "
                    }
                ))
                .style(style),
            ];
            if option.color.is_empty() {
                spans.push(Span::new(option.label.clone()).style(style));
            } else {
                spans.push(label_pill_span(&label, colors));
            }
            Text::from_spans(spans)
                .width(Length::Flex(1))
                .height(Length::Px(1))
                .style(style)
                .into()
        } else {
            line(
                format!(
                    " {} {}",
                    if c.selected == option_index {
                        "›"
                    } else {
                        " "
                    },
                    if c.kind == LookupKind::Users {
                        ctx.state.user_formatter.render_lookup(option)
                    } else {
                        option.label.clone()
                    }
                ),
                style,
            )
            .into()
        };
        rows.push(click(
            ctx,
            format!("lookup-{index}-{}", option.id),
            VStack::new()
                .height(Length::Px(2))
                .style(style)
                .child(header)
                .child(line(
                    format!("   {}", option.description),
                    style.fg(colors.muted),
                )),
            move || Msg::LookupAccept(index, epoch, option_index),
        ));
    }
    let mut choices = ScrollView::new()
        .height(Length::Px((ctx.viewport().h / 4).clamp(2, 8)))
        .scrollbar(true)
        .scrollbar_config(vertical_scrollbar(
            ctx,
            &format!("lookup-options-{index}-scrollbar"),
            colors,
        ))
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .smooth_wheel_scroll(false)
        .estimated_child_height(2)
        .children(rows);
    if let Some(option) = c.options.get(c.selected) {
        choices = choices.reveal_key(format!("lookup-{index}-{}", option.id));
    }
    VStack::new()
        .height(Length::Auto)
        .child(line(
            "↑/↓ choose · Enter accept · Tab next field · Esc cancel",
            Style::new().fg(colors.accent),
        ))
        .child(interaction::scrollbar(
            ctx,
            format!("lookup-options-{index}-scrollbar"),
            choices.key(format!("lookup-options-{index}")),
        ))
        .child(line(
            "Up to 20 matches · type more to narrow the search",
            Style::new().fg(colors.muted),
        ))
        .into()
}

fn dialog_option(
    ctx: &Context<Cronk>,
    index: usize,
    label: &str,
    selected: bool,
    colors: Colors,
    submit_on_click: bool,
) -> Element {
    let link = ctx.link().clone();
    click(
        ctx,
        format!("dialog-option-{index}"),
        line(
            format!(" {} {label}", if selected { "›" } else { " " }),
            interaction::style(
                ctx,
                &format!("dialog-option-{index}"),
                if selected {
                    colors.selected().fg(colors.accent).bold()
                } else {
                    Style::new().fg(colors.foreground).bg(colors.surface)
                },
            ),
        ),
        move || {
            if submit_on_click {
                link.send(Msg::DialogSelect(index));
                Msg::Submit
            } else {
                Msg::DialogSelect(index)
            }
        },
    )
}

const HELP: &str = "\
## Navigation
- **Shift+D / P / I / M** open Dashboard, Projects, Issues, or Merge Requests.
- **1–9 / 0** open saved views 1–10. Tab shortcuts do not run inside dialogs.
- Lists are active immediately. **Left / Right** switch tabs without wrapping, outside dialogs.
- Overflowing panes show a right-edge scrollbar. Click its track or drag its thumb to scroll.
- **Enter** opens the selected item, section, field, or job.
- Details show all sections in one document. **Tab / Shift+Tab / Up / Down** select sections.
- **PageUp / PageDown** scroll the whole detail document; **Enter** focuses section actions.
- **Esc** leaves section actions without resetting the viewport, then returns to the list.
- **Esc** goes back one level; at a list it does nothing. Click a row to open it directly.
- **Space** toggles visibility in Projects. Hidden projects leave the work lists.

## Workspace
- **/** filters the current view. Save a filter as a named tab from commands.
- **:** opens commands; type to search, then choose an action.
- **r** refreshes. **?** opens this help. **q** quits outside forms.
- The command palette exposes project aliases, creation, edits, comments,
  discussion replies/resolution, job retries, and themes.

## Forms and live data
- **Enter** submits the whole form. **Tab / Shift+Tab** switch fields.
- **Ctrl+J** inserts a newline in multiline fields. **Esc** cancels.
- ID-backed fields suggest names: **Up / Down** choose, **Enter** accepts, then saves.
- Separate assignees/reviewers with commas; only the trimmed token at the caret is searched.
- Running jobs expand automatically. All live traces appear inline in Jobs.
- Job logs follow their trailing output; finished jobs expand on demand.
- **DEMO** uses fictional offline data, never a live GitLab workspace.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_and_pending_status_styles_in_every_theme() {
        for theme in ["midnight", "dracula", "light"] {
            let colors = Colors::new(&Config {
                theme: theme.into(),
                ..Config::default()
            });
            assert_eq!(colors.status("running"), ('◐', colors.cyan));
            let neutral = Color::hex_u24(if theme == "light" { 0x626262 } else { 0xc7c7c7 });
            for status in ["pending", "created", "waiting_for_resource", "preparing"] {
                assert_eq!(colors.status(status), ('○', neutral));
                let help = status_help("Job", status);
                assert!(help.starts_with(&format!("Job: {status} — ")));
                assert!(!help.ends_with("Status reported by GitLab"));
            }
        }
    }

    #[test]
    fn status_tooltips_explain_known_states_and_preserve_unknown_values() {
        for status in [
            "opened",
            "closed",
            "merged",
            "locked",
            "success",
            "passed",
            "running",
            "failed",
            "error",
            "pending",
            "created",
            "waiting_for_resource",
            "preparing",
            "manual",
            "scheduled",
            "canceled",
            "cancelled",
            "skipped",
            "resolved",
            "unresolved",
            "comment",
            "warning",
        ] {
            let help = status_help("Status", status);
            assert!(help.starts_with(&format!("Status: {status} — ")));
            assert!(
                !help.ends_with("Status reported by GitLab"),
                "missing explanation: {status}"
            );
        }
        assert_eq!(
            status_help("Job", "custom-state"),
            "Job: custom-state — Status reported by GitLab"
        );
        assert!(status_help("Discussion", "comment").contains("does not require resolution"));
        assert!(status_help("Job", "manual").contains("manual action"));
    }

    #[test]
    fn gitlab_label_colors_are_exact_and_selection_exempt() {
        let colors = Colors::new(&Config::default());
        let spans = label_spans(
            &[Label {
                name: "urgent".into(),
                color: "#123456".into(),
                text_color: "#abcdef".into(),
            }],
            colors,
        );
        assert_eq!(spans[0].style.bg, Some(Color::hex_u24(0x123456).into()));
        assert_eq!(spans[0].style.fg, Some(Color::hex_u24(0xabcdef).into()));
        assert_eq!(spans[0].row_style_policy, RowStylePolicy::Disabled);
        assert_eq!(spans[0].style.contrast_policy, Some(ContrastPolicy::Off));
    }

    #[test]
    fn theme_overrides_are_applied_to_all_six_roles() {
        let mut config = Config::default();
        config.colors.background = Some("#010203".into());
        config.colors.surface = Some("#111213".into());
        config.colors.selection = Some("#212223".into());
        config.colors.foreground = Some("#313233".into());
        config.colors.muted = Some("#414243".into());
        config.colors.accent = Some("#515253".into());
        let colors = Colors::new(&config);
        assert_eq!(colors.background, Color::hex_u24(0x010203));
        assert_eq!(colors.surface, Color::hex_u24(0x111213));
        assert_eq!(colors.selection, Color::hex_u24(0x212223));
        assert_eq!(colors.foreground, Color::hex_u24(0x313233));
        assert_eq!(colors.muted, Color::hex_u24(0x414243));
        assert_eq!(colors.accent, Color::hex_u24(0x515253));
    }

    #[test]
    fn tails_keep_last_lines_without_splitting_unicode() {
        assert_eq!(trace_tail("first\nsecond\nthird\n", 2), "second\nthird");
        assert_eq!(trace_tail("α\nβ\nγ", 2), "β\nγ");
        assert_eq!(trace_tail("", 10), "");
    }

    #[test]
    fn selected_row_preserves_label_cells_in_every_theme() {
        for theme in ["midnight", "dracula", "light"] {
            let config = Config {
                theme: theme.into(),
                ..Config::default()
            };
            let colors = Colors::new(&config);
            let mockup = Mockup::new(move || {
                ThemeProvider::new(colors.theme())
                    .child(rich(
                        label_spans(
                            &[Label {
                                name: "urgent".into(),
                                color: "#123456".into(),
                                text_color: "#abcdef".into(),
                            }],
                            colors,
                        ),
                        colors.selected(),
                    ))
                    .into()
            });
            let backend = tui_lipan::TestBackend::new_with_viewport(
                mockup,
                Rect {
                    x: 0,
                    y: 0,
                    w: 30,
                    h: 1,
                },
            );
            let frame = backend.capture_frame();
            assert_eq!(frame.cell(1, 0).symbol, "u");
            assert_eq!(frame.cell(1, 0).fg, Color::hex_u24(0xabcdef));
            assert_eq!(frame.cell(1, 0).bg, Color::hex_u24(0x123456));
            assert_eq!(frame.cell(20, 0).bg, colors.selection);
        }
    }

    #[test]
    fn form_keys_submit_cancel_cycle_and_insert_newlines() {
        let key = |code, mods| KeyEvent { code, mods };
        assert!(matches!(
            dialog_key(key(KeyCode::Enter, KeyMods::NONE), 1, 1, 3, true, false),
            Some(Msg::Submit)
        ));
        assert!(matches!(
            dialog_key(key(KeyCode::Esc, KeyMods::NONE), 1, 1, 3, true, false),
            Some(Msg::CloseDialog)
        ));
        assert!(matches!(
            dialog_key(key(KeyCode::Tab, KeyMods::NONE), 2, 2, 3, false, false),
            Some(Msg::DialogField(0))
        ));
        assert!(matches!(
            dialog_key(key(KeyCode::BackTab, KeyMods::SHIFT), 0, 0, 3, true, false),
            Some(Msg::DialogField(2))
        ));
        assert!(matches!(
            dialog_key(key(KeyCode::Char('j'), KeyMods::CTRL), 1, 1, 3, true, false),
            Some(Msg::Newline(1))
        ));
        assert!(dialog_key(key(KeyCode::Char('j'), KeyMods::NONE), 1, 1, 3, true, false).is_none());
        assert!(matches!(
            dialog_key(key(KeyCode::Tab, KeyMods::NONE), 0, 4, 1, false, true),
            Some(Msg::Move(1))
        ));
        assert!(matches!(
            dialog_key(key(KeyCode::Down, KeyMods::NONE), 0, 4, 1, false, true),
            Some(Msg::Move(1))
        ));
    }

    #[test]
    fn invalid_colors_fall_back_instead_of_resetting_terminal_colors() {
        assert_eq!(parse_color("#aBcDeF"), Some(Color::hex_u24(0xabcdef)));
        assert_eq!(parse_color("#zzzzzz"), None);
        assert_eq!(parse_color("#123"), None);
        assert_eq!(parse_color("#１２３"), None);
    }
}
