use super::{
    Action, Completion, Cronk, Dialog, DialogKind, Msg, Scope, State,
    interaction::{self, click},
    list_height_for_tab, list_row_height,
};
use crate::{build_info, config::Config, model::*};
use std::{any::Any, collections::HashMap, time::Duration};
use tui_lipan::{
    TextAreaColorInput, TextAreaColorLines, TextAreaColorStrategy,
    prelude::*,
    style::{CellEffect, EffectCell, EffectContext, RowStylePolicy, ThemePalette},
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
    blue: Color,
    purple: Color,
    pending: Color,
}

impl Colors {
    fn new(config: &Config) -> Self {
        // Blue keeps the demo area-label hue (#1f78d1). Its OKLab lightness targets
        // the midpoint of red/green, reducing chroma only to fit sRGB. The light
        // theme is slightly darker to retain >= 4.5:1 contrast on selection fill.
        let values = match config.theme.as_str() {
            "dracula" => [
                0x282a36, 0x21222c, 0x44475a, 0xf8f8f2, 0xa5a8c0, 0xbd93f9, 0x50fa7b, 0xf1fa8c,
                0xff5555, 0x7fbaff, 0xff79c6,
            ],
            "light" => [
                0xf7f8fc, 0xeceff6, 0xdce5fa, 0x24283b, 0x626b83, 0x5746bd, 0x187348, 0x946200,
                0xc2354b, 0x0066bc, 0x9146ae,
            ],
            "blade-runner" => [
                0x0b0714, 0x140d24, 0x2c1a52, 0xf4ecff, 0xb4a6d6, 0xff3fd8, 0x3dffa2, 0xffe34d,
                0xff5578, 0x2ee6ff, 0xb78cff,
            ],
            "tokyo-night" => [
                0x1a1b26, 0x16161e, 0x28365a, 0xd5dcff, 0xaab5df, 0x7aa2f7, 0x9ece6a, 0xe0af68,
                0xf7768e, 0x7dcfff, 0xbb9af7,
            ],
            "gruvbox-dark" => [
                0x282828, 0x1d2021, 0x4a3f2c, 0xf2e5bc, 0xc4b79b, 0xfe8019, 0xb8bb26, 0xfabd2f,
                0xff6350, 0x83a598, 0xd3869b,
            ],
            "nord" => [
                0x2e3440, 0x272c36, 0x354563, 0xeceff4, 0xb4bfd3, 0x88c0d0, 0xa3be8c, 0xebcb8b,
                0xe5808b, 0x8fb4e8, 0xc79bc0,
            ],
            "solarized-dark" => [
                0x002b36, 0x073642, 0x0d4452, 0xeee8d5, 0xaebbbb, 0x3fb8ae, 0xa4bd1a, 0xe0b030,
                0xff6b66, 0x4fa8f0, 0xa5aaf0,
            ],
            "solarized-light" => [
                0xfdf6e3, 0xeee8d5, 0xd9d1b5, 0x073642, 0x42585f, 0x0f6f9f, 0x4f6200, 0x805c00,
                0xbf2220, 0x1a62a8, 0x5c5fb0,
            ],
            "sepia-dark" => [
                0x2a2118, 0x20190f, 0x4e3a22, 0xf0e4cc, 0xcdb99a, 0xe6a85c, 0xa6cc82, 0xebc66f,
                0xf08068, 0x86b7dd, 0xcfa3c8,
            ],
            "sepia-light" => [
                0xf4ecd8, 0xe9dfc4, 0xd9c9a3, 0x3b2f20, 0x5a4a35, 0x8a4510, 0x3a6326, 0x745400,
                0xa3301d, 0x2a557f, 0x7a447a,
            ],
            _ => [
                0x101521, 0x192131, 0x293654, 0xe5eaf5, 0x96a3bc, 0x9b9fff, 0x63dba5, 0xf0c674,
                0xff758f, 0x7bb8ff, 0xc69cf4,
            ],
        };
        let c = &config.colors;
        let custom = |value: &Option<String>, fallback| {
            value
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(Color::hex_u24(fallback))
        };
        let background = custom(&c.background, values[0]);
        Self {
            background,
            surface: custom(&c.surface, values[1]),
            selection: custom(&c.selection, values[2]),
            foreground: custom(&c.foreground, values[3]),
            muted: custom(&c.muted, values[4]),
            accent: custom(&c.accent, values[5]),
            green: Color::hex_u24(values[6]),
            yellow: Color::hex_u24(values[7]),
            red: Color::hex_u24(values[8]),
            blue: Color::hex_u24(values[9]),
            purple: Color::hex_u24(values[10]),
            // A darker neutral keeps the light theme readable without using a warning color.
            pending: Color::hex_u24(if background.luminance() > 0.5 {
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
            .info(self.blue)
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
        theme.document.code_inline = Style::new().fg(self.blue).bg(self.surface);
        theme.document.code_block = Style::new().fg(self.foreground).bg(self.surface);
        theme.document.link = Style::new().fg(self.blue).underline();
        theme.document.blockquote_bar = Style::new().fg(self.purple);
        theme.document.table_header = Style::new().fg(self.accent).bold();
        theme.document.table_border = Style::new().fg(self.muted);
        theme.document.hr = Style::new().fg(self.muted);
        theme
    }

    fn status(self, status: &str) -> (char, Color) {
        match status {
            "opened" | "success" | "passed" | "resolved" => ('●', self.green),
            "running" => ('◐', self.blue),
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
        "visible" => "Included in Dashboard, Issues, and Merge Requests",
        "hidden" => "Excluded from work lists until shown again",
        _ => "Status reported by GitLab",
    };
    format!("{subject}: {status} — {explanation}")
}

/// Only the single glyph is the trigger: adjacent padding/text never opens the tip.
/// A passive mouse region observes hover without intercepting the surrounding row click.
fn user_help(user: &User) -> String {
    let mut details = Vec::new();
    if !user.name.trim().is_empty() {
        details.push(format!("Name: {}", user.name));
    }
    if !user.username.trim().is_empty() {
        details.push(format!("Username: @{}", user.username));
    }
    if user.id > 0 {
        details.push(format!("GitLab user ID: {}", user.id));
    }
    if let Some(email) = user
        .public_email
        .as_deref()
        .filter(|email| !email.trim().is_empty())
    {
        details.push(format!("Public email: {email}"));
    }
    if user.bot {
        details.push("Bot account".to_owned());
    }
    if details.is_empty() {
        "GitLab user details are unavailable".to_owned()
    } else {
        details.join("\n")
    }
}

/// A passive hover region around a user label; it leaves containing row clicks intact.
fn user_tooltip(
    ctx: &Context<Cronk>,
    key: impl Into<String>,
    user: &User,
    child: impl Into<Element>,
) -> Element {
    let key = key.into();
    let move_key = key.clone();
    let leave_key = key.clone();
    let help = user_help(user);
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
            .child(child),
    )
    .key(key)
}

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
    Span::new(label.name.clone())
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
    for (index, label) in labels.iter().enumerate() {
        if index > 0 {
            spans.push(Span::new(", "));
        }
        spans.push(label_pill_span(label, colors));
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
        Scope::Details | Scope::Section if state.log_zoom => {
            if let Some(job) = state.selected_job() {
                job_panel(ctx, job, state.log_height(ctx.viewport().h, job.id), colors)
            } else {
                detail(ctx, colors)
            }
        }
        Scope::Details | Scope::Section if state.project_details() => project_detail(ctx, colors),
        Scope::Details | Scope::Section => detail(ctx, colors),
    };
    let search_row = if let Some(input) = &state.log_search {
        Input::bound(input)
            .height(Length::Px(1))
            .border(false)
            .padding((0, 2))
            .style(colors.base())
            .focus_style(colors.selected())
            .caret_color(colors.accent)
            .key_interceptor(ctx.link().key_handler(|key: KeyEvent| match key.code {
                KeyCode::Enter => Some(Msg::SubmitLogSearch),
                KeyCode::Esc => Some(Msg::Back),
                _ => None,
            }))
            .on_change(ctx.link().callback(Msg::LogSearchInput))
            .key("log-search")
    } else {
        blank()
    };
    let shell = if state.log_zoom && state.selected_job().is_some() {
        // Fullscreen logs keep only the contextual shortcut header and log
        // status, reclaiming the normal tabs/breadcrumb/footer chrome.
        let mut shell = VStack::new()
            .style(colors.base())
            .child(context_line(state, colors));
        if state.log_search.is_some() {
            shell = shell.child(search_row);
        }
        shell.child(content)
    } else {
        // Keep six rows above and two below the list, matching list_height exactly.
        VStack::new()
            .style(colors.base())
            .child(brand(state, colors))
            .child(tab_bar(ctx, colors))
            .child(breadcrumb(ctx, colors))
            .child(context_line(state, colors))
            .child(search_row)
            .child(content)
            .child(footer(ctx, colors))
    };
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

/// Display name and one-line description for the theme chooser.
fn theme_description(theme: &str) -> (&'static str, &'static str) {
    match theme {
        "midnight" => ("Midnight", "Cool ink · violet · blue"),
        "dracula" => ("Dracula", "Charcoal · orchid · electric green"),
        "light" => ("Light", "Paper · indigo · forest green"),
        "blade-runner" => ("Blade Runner", "Neon noir · magenta · cyan"),
        "tokyo-night" => ("Tokyo Night", "Deep navy · periwinkle · mint"),
        "gruvbox-dark" => ("Gruvbox Dark", "Warm charcoal · orange · olive"),
        "nord" => ("Nord", "Arctic slate · frost · sage"),
        "solarized-dark" => ("Solarized Dark", "Deep teal · cream · cyan"),
        "solarized-light" => ("Solarized Light", "Cream · navy ink · ocean blue"),
        "sepia-dark" => ("Sepia Dark", "Coffee · parchment · amber"),
        "sepia-light" => ("Sepia Light", "Aged paper · umber · rust"),
        _ => ("Custom", ""),
    }
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
        if let Some(id) = state
            .config
            .project_route
            .filter(|_| state.project_details())
        {
            spans.push(Span::new(format!("  /  {}", project_name(state, id))));
        } else if let Some(key) = &state.config.route {
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
    let (heading, hint) =
        if let Some(job) = state.selected_job().filter(|_| state.log_focus.is_some()) {
            (
                format!(
                    "{} #{}{}",
                    job.name,
                    job.id,
                    if state.log_zoom {
                        " · ZOOM"
                    } else {
                        " · LOGS"
                    }
                ),
                if state.log_search.is_some() {
                    "Search logs · Enter finds · Esc cancels".into()
                } else {
                    "z zoom · Esc back · ↑↓ PgUp/PgDn · Home/End · / search · n/N matches".into()
                },
            )
        } else {
            match state.scope {
                Scope::List if state.config.active_tab == 1 => (
                    format!("{} PROJECTS", state.config.projects.len()),
                    "Space toggles visibility · Enter opens project details".to_owned(),
                ),
                Scope::List => {
                    let query = state.query_text();
                    (
                        format!("{} ITEMS", state.visible_item_count()),
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
            }
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
        "Jobs" => "↑ ↓ choose · Space toggle · Enter logs · z zoom · Ctrl+↑/↓/PgUp/PgDn logs",
        "Discussions" => "↑ ↓ choose a discussion · reply / resolve via commands",
        "Pipeline" => "All running jobs stream together · ↑ ↓ scroll",
        "Changes" => "Unified diff · ↑ ↓ scroll",
        _ => "↑ ↓ scroll · Esc returns to section navigation",
    }
}

fn main_list(ctx: &Context<Cronk>, colors: Colors) -> Element {
    let state = &ctx.state;
    let projects = state.config.active_tab == 1;
    let items = if projects {
        Vec::new()
    } else {
        state.visible_items()
    };
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
    // ScrollView virtualises mounting, not construction of its Element children.
    // Build rich rows only in the viewport plus one viewport of overscan on each
    // side. Two spacers retain the native scrollbar's full extent; callbacks
    // still use global row indices rather than positions within the window.
    let window = list_render_window(scroll.offset, slots, len);
    let mut rows = Vec::with_capacity(window.len() + 2);
    list_spacers(&mut rows, window.start * row_height);
    for index in window.clone() {
        rows.push(if projects {
            project_row(
                ctx,
                &state.config.projects[index],
                index,
                scroll.selected == index,
                colors,
            )
        } else {
            work_row(ctx, items[index], index, scroll.selected == index, colors)
        });
    }
    list_spacers(&mut rows, (len - window.end) * row_height);
    let offset = scroll.offset;
    let scrollbar_key = format!("main-list-{}-scrollbar", state.config.active_tab);
    let list = ScrollView::new()
        // The child tree is already windowed. Measure it exactly so large
        // spacer heights never pollute the widget's per-row height estimates.
        .virtualize(false)
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

fn list_spacers(rows: &mut Vec<Element>, mut height: usize) {
    // Length::Px is u16; never truncate a large logical height on conversion.
    while height > 0 {
        let chunk = height.min(u16::MAX as usize);
        rows.push(Spacer::new().height(Length::Px(chunk as u16)).into());
        height -= chunk;
    }
}

fn list_render_window(offset: usize, slots: usize, len: usize) -> std::ops::Range<usize> {
    offset.saturating_sub(slots)..offset.saturating_add(slots.saturating_mul(2)).min(len)
}

/// Keep the selection edge on the normal canvas, outside the row's selection/hover fill.
fn list_selection_line(content: impl Into<Element>, selected: bool, colors: Colors) -> Element {
    selection_line(content, selected, colors, colors.base())
}

fn selection_line(
    content: impl Into<Element>,
    selected: bool,
    colors: Colors,
    style: Style,
) -> Element {
    HStack::new()
        .height(Length::Px(1))
        .style(style)
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
    if !item.labels.is_empty() {
        // Keep the labels apart from the pipeline dot that follows them.
        labels.push(Span::new(" "));
    }
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
    labels_line = labels_line.child(user_tooltip(
        ctx,
        format!("item-{}-author", item_key(&item.key)),
        &item.author,
        Text::new(state.render_user(&item.author))
            .width(Length::Flex(1))
            .height(Length::Px(1))
            .overflow(Overflow::Ellipsis)
            .style(style.fg(colors.muted)),
    ));
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
    let (symbol, color, status) = if project.visible {
        ('●', colors.green, "visible")
    } else {
        ('○', colors.muted, "hidden")
    };
    let link = ctx.link().clone();
    let visibility = click(
        ctx,
        format!("project-visible-{}", project.id),
        dot_tooltip(
            ctx,
            format!("project-visible-tip-{}", project.id),
            symbol,
            color,
            status_help("Project", status),
            style,
        ),
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
        .child(Text::new(" ").width(Length::Px(1)).style(style))
        .child(visibility)
        .child(Text::new(" ").width(Length::Px(1)).style(style))
        .child(line(project_name(state, project.id), style.bold()))
        .child(Text::new(format!("{count} items  ")).style(Style::new().fg(colors.muted)));
    let marked = selected && state.scope == Scope::List;
    click(
        ctx,
        format!("project-{}", project.id),
        VStack::new()
            .height(Length::Px(3))
            .style(colors.base())
            .child(list_selection_line(top, marked, colors))
            .child(list_selection_line(
                rich(
                    vec![
                        Span::new(format!("   {}  ·  ", project.path)).fg(colors.muted),
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
                ),
                marked,
                colors,
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

fn project_detail(ctx: &Context<Cronk>, colors: Colors) -> Element {
    let state = &ctx.state;
    let Some(project) = state
        .config
        .project_route
        .and_then(|id| state.project(id))
        .cloned()
    else {
        return empty_state(
            "Project unavailable",
            "The project list is still available. Go back, or refresh to retry.",
            colors,
        );
    };
    let mut content = Vec::new();
    let selected = state.section_cursor == 0;
    // Match issue/MR details: broad section fill on Details, contract to the focused
    // field after Enter, and expand again on Escape.
    let broad = state.scope == Scope::Details;
    let background = if state.config.animations {
        ctx.transition(
            "detail-focus-background-0",
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
            "detail-focus-edge-0",
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
    let style = interaction::style(ctx, "detail-section-0", section_colors.base());
    content.push(click(
        ctx,
        "detail-section-0",
        detail_selection_row(
            HStack::new()
                .height(Length::Px(1))
                .style(style)
                .child(Text::new("Fields ").style(style.fg(colors.accent).bold()))
                .child(Divider::horizontal().style(style.fg(colors.muted)))
                .into(),
            None,
            edge,
            style,
            colors,
        ),
        || Msg::DetailSection(0),
    ));
    for (row_index, child) in project_fields(ctx, &project, section_colors)
        .into_iter()
        .enumerate()
    {
        let key = child
            .key
            .unwrap_or_else(|| format!("detail-section-0-row-{row_index}"));
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
    content.push(blank().key("detail-section-gap-0"));
    let target = state.reveal_content.then(|| state.detail_target_key());
    let mut scroll = scroll_content(ctx, content, colors);
    if let Some(target) = target {
        scroll = scroll.reveal_key(target);
    } else if let Some(offset) = state.detail_offset_request {
        scroll = scroll.scroll_to_key_offset("detail-section-0", offset);
    }
    interaction::scrollbar(
        ctx,
        "detail-scrollbar",
        Element::from(scroll).key(format!("project-detail-{}", project.id)),
    )
}

fn project_fields(ctx: &Context<Cronk>, project: &Project, colors: Colors) -> Vec<DetailRow> {
    let state = &ctx.state;
    let issues = state
        .items
        .iter()
        .filter(|item| item.key.project == project.id && item.key.kind == ItemKind::Issue)
        .count();
    let merge_requests = state
        .items
        .iter()
        .filter(|item| item.key.project == project.id && item.key.kind == ItemKind::MergeRequest)
        .count();
    let visibility = if project.visible { "visible" } else { "hidden" };
    let mut content = vec![
        DetailRow::from(rich(
            vec![
                Span::new(if project.visible { "Visible" } else { "Hidden" }).fg(
                    if project.visible {
                        colors.green
                    } else {
                        colors.muted
                    },
                ),
                Span::new(format!("   {}", project.path)).fg(colors.accent),
            ],
            colors.base(),
        ))
        .with_status("detail-project-status", "Project", visibility),
        blank().into(),
    ];
    let alias_selected = state.scope == Scope::Section
        && state.section_name() == "Fields"
        && state.config.field == 0;
    let alias_style = if alias_selected {
        colors.selected()
    } else {
        colors.base()
    };
    let alias_style = interaction::style(ctx, "edit-field-0", alias_style);
    let alias_value = if project.alias.is_empty() {
        "—".into()
    } else {
        project.alias.clone()
    };
    let label_width = ["Alias", "Path", "Project ID", "Visibility", "Open work"]
        .into_iter()
        .map(str::len)
        .max()
        .unwrap_or(0);
    content.push(DetailRow::interactive(
        "edit-field-0".into(),
        field_row(
            "Alias",
            label_width,
            vec![Span::new(alias_value)],
            colors.accent,
            alias_style,
        ),
        alias_selected,
        || Msg::Field(0),
    ));
    for (label, value) in [
        ("Path", project.path.clone()),
        ("Project ID", project.id.to_string()),
        (
            "Visibility",
            if project.visible {
                "Included in work lists".into()
            } else {
                "Hidden from work lists".into()
            },
        ),
        (
            "Open work",
            format!("{issues} issues · {merge_requests} merge requests"),
        ),
    ] {
        content.push(
            field_row(
                label,
                label_width,
                vec![Span::new(value)],
                colors.muted,
                colors.base(),
            )
            .into(),
        );
    }
    content.push(blank().into());
    let remove_selected = state.scope == Scope::Section
        && state.section_name() == "Fields"
        && state.config.field == 1;
    let remove_style = if remove_selected {
        colors.selected().fg(colors.red).bold()
    } else {
        colors.base().fg(colors.red).bold()
    };
    let remove_style = interaction::style(ctx, "project-remove", remove_style);
    content.push(DetailRow::interactive(
        "project-remove".into(),
        Element::from(Text::new(" Remove from workspace ").style(remove_style)),
        remove_selected,
        || Msg::Action(Action::RemoveProject),
    ));
    content.push(
        line(
            "Removes this project from Cronk only. GitLab data is unchanged.",
            colors.base().fg(colors.muted),
        )
        .into(),
    );
    content
}

/// A `label  value` row with the label right-aligned in a `label_width` column
/// and the value wrapping within its own column.
fn field_row(
    label: &str,
    label_width: usize,
    value: Vec<Span>,
    label_color: Color,
    style: Style,
) -> Element {
    field_row_element(
        label,
        label_width,
        Text::from_spans(value)
            .width(Length::Flex(1))
            .height(Length::Auto)
            .overflow(Overflow::Wrap)
            .style(style),
        label_color,
        style,
    )
}

fn field_row_element(
    label: &str,
    label_width: usize,
    value: impl Into<Element>,
    label_color: Color,
    style: Style,
) -> Element {
    HStack::new()
        .style(style)
        .height(Length::Auto)
        .gap(2)
        .child(
            Text::new(format!("{label:>label_width$}"))
                .width(Length::Px(label_width as u16))
                .style(style.fg(label_color)),
        )
        .child(value)
        .into()
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
        let part = match *section {
            "Fields" | "Description" => DetailPart::Core,
            "Activity" => DetailPart::Activity,
            "Discussions" => DetailPart::Discussions,
            "Pipeline" | "Jobs" => DetailPart::Pipeline,
            _ => DetailPart::Changes,
        };
        let missing = state.detail_pending.is_some() && !details.loaded.contains(&part);
        let mut children: Vec<DetailRow> = if missing {
            let shade = if state.config.animations && state.tick.is_multiple_of(2) {
                colors.blue
            } else {
                colors.muted
            };
            vec![
                line(format!("Loading {section}…"), style.fg(colors.muted)).into(),
                line(" ▰▰▰▰▰▰▰▰  ▰▰▰▰", style.fg(shade)).into(),
                line(" ▰▰▰▰▰▰  ▰▰▰▰▰▰▰▰", style.fg(shade)).into(),
            ]
        } else {
            match *section {
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
                            .map(|note| note_view(ctx, note, section_colors))
                            .collect()
                    }
                }
                "Pipeline" => pipeline(ctx, details, section_colors),
                "Jobs" => jobs(ctx, details, section_colors),
                "Discussions" => discussions(ctx, details, section_colors),
                "Changes" => changes(details, section_colors),
                _ => Vec::new(),
            }
        };
        let warning_label = match part {
            DetailPart::Core => "Label colors",
            DetailPart::Activity => "Notes",
            DetailPart::Discussions => "Discussions",
            DetailPart::Pipeline => "Pipeline",
            DetailPart::Changes => "diff",
        };
        for warning in details
            .warnings
            .iter()
            .filter(|w| w.to_lowercase().contains(&warning_label.to_lowercase()))
        {
            children.push(line(format!("⚠ {warning}"), style.fg(colors.yellow)).into());
        }
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
                // Without a known end the extent needs a layout pass (below).
                let (top, end) = super::detail_span(event, &target)?;
                end.map(|end| {
                    crate::scroll::reveal_span(
                        origin,
                        top,
                        Some(end),
                        event.metrics.len,
                        event.metrics.visible,
                    )
                })
            });
        if let Some(offset) = measured {
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
    let state = &ctx.state;
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
        blank().into(),
    ];
    let editable = ctx.state.fields();
    let mut readonly = vec![
        ("Project path", {
            ctx.state
                .project(details.item.key.project)
                .map_or_else(|| details.item.key.project.to_string(), |p| p.path.clone())
        }),
        ("Author", ctx.state.render_user(&item.author)),
        ("Updated", item.updated_at.clone()),
    ];
    if item.key.kind == ItemKind::MergeRequest {
        readonly.push((
            "Branches",
            format!("{} → {}", item.source_branch, item.target_branch),
        ));
        readonly.push((
            "Review",
            format!(
                "{} discussions · {} unresolved · {} changed files",
                if state.detail_pending.is_some()
                    && !details.loaded.contains(&DetailPart::Discussions)
                {
                    "unknown".into()
                } else {
                    details.discussions.len().to_string()
                },
                item.unresolved
                    .map_or_else(|| "unknown".into(), |n| n.to_string()),
                if state.detail_pending.is_some() && !details.loaded.contains(&DetailPart::Changes)
                {
                    "unknown".into()
                } else {
                    details.diffs.len().to_string()
                }
            ),
        ));
    }
    readonly.push(("Web URL", item.web_url.clone()));
    // One label column shared by editable and readonly rows so values align.
    let label_width = editable
        .iter()
        .map(|(label, _, _)| *label)
        .chain(readonly.iter().map(|(label, _)| *label))
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0);
    for (index, (label, _, value)) in editable.into_iter().enumerate() {
        let selected = ctx.state.scope == Scope::Section
            && ctx.state.section_name() == "Fields"
            && ctx.state.config.field == index;
        let style = if selected {
            colors.selected()
        } else {
            colors.base()
        };
        let style = interaction::style(ctx, &format!("edit-field-{index}"), style);
        let value_element: Element = if label == "Labels" && !details.item.labels.is_empty() {
            Text::from_spans(label_spans(&details.item.labels, colors))
                .width(Length::Flex(1))
                .height(Length::Auto)
                .overflow(Overflow::Wrap)
                .style(style)
                .into()
        } else if matches!(label, "Assignees" | "Reviewers") {
            let users = if label == "Assignees" {
                &details.item.assignees
            } else {
                &details.item.reviewers
            };
            if users.is_empty() {
                Text::new("—").style(style).into()
            } else {
                let mut list = HStack::new().height(Length::Auto).style(style);
                for (user_index, user) in users.iter().enumerate() {
                    if user_index > 0 {
                        list = list.child(Text::new(", ").style(style));
                    }
                    list = list.child(user_tooltip(
                        ctx,
                        format!("item-{}-{label}-{user_index}", item_key(&item.key)),
                        user,
                        Text::new(ctx.state.render_user(user)).style(style),
                    ));
                }
                list.into()
            }
        } else {
            Text::new(if value.is_empty() {
                "—".into()
            } else {
                value
            })
            .width(Length::Flex(1))
            .height(Length::Auto)
            .overflow(Overflow::Wrap)
            .style(style)
            .into()
        };
        content.push(DetailRow::interactive(
            format!("edit-field-{index}"),
            field_row_element(label, label_width, value_element, colors.accent, style),
            selected,
            move || Msg::Field(index),
        ));
    }
    if !readonly.is_empty() {
        content.push(blank().into());
    }
    for (label, value) in readonly {
        if label == "Author" {
            content.push(
                field_row_element(
                    label,
                    label_width,
                    user_tooltip(
                        ctx,
                        format!("detail-author-{}", item_key(&item.key)),
                        &item.author,
                        Text::new(state.render_user(&item.author))
                            .width(Length::Flex(1))
                            .height(Length::Auto)
                            .overflow(Overflow::Wrap)
                            .style(colors.base()),
                    ),
                    colors.muted,
                    colors.base(),
                )
                .into(),
            );
        } else {
            let value = if value.is_empty() {
                "—".into()
            } else {
                value
            };
            content.push(
                field_row(
                    label,
                    label_width,
                    vec![Span::new(value)],
                    colors.muted,
                    colors.base(),
                )
                .into(),
            );
        }
    }
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

fn note_view(ctx: &Context<Cronk>, note: &Note, colors: Colors) -> DetailRow {
    let mut header = HStack::new()
        .height(Length::Px(1))
        .style(colors.base())
        .child(user_tooltip(
            ctx,
            format!("note-{}-author", note.id),
            &note.author,
            Text::new(ctx.state.render_user(&note.author))
                .style(colors.base().fg(colors.blue).bold()),
        ))
        .child(Text::new(format!("   {}", note.created_at)).style(colors.base().fg(colors.muted)));
    if note.system {
        header = header.child(Text::new("  ·  system").style(colors.base().fg(colors.muted)));
    }
    let status = if !note.resolvable {
        "comment"
    } else if note.resolved {
        "resolved"
    } else {
        "unresolved"
    };
    if note.resolvable {
        header = header
            .child(Text::new("  ·  "))
            .child(Text::new(status).style(colors.base().fg(colors.status(status).1)));
    }
    DetailRow::keyed(
        format!("note-{}", note.id),
        VStack::new()
            .height(Length::Auto)
            .style(colors.base())
            .child(header)
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

fn log_window(ctx: &Context<Cronk>, job: &Job, height: usize, colors: Colors) -> Element {
    let id = job.id;
    let trace = ctx.state.traces.get(&id);
    let total = trace.map_or(0, super::Trace::line_count);
    let default_view = super::LogView::default();
    let view = ctx.state.log_views.get(&id).unwrap_or(&default_view);
    let height = if ctx.state.log_zoom {
        height
    } else {
        height.min(total.max(1))
    }
    .max(1)
    .min(u16::MAX as usize / 3);
    let offset = view.position(total, height);
    let style = colors.base().bg(colors.surface);
    let current_match = view
        .current_match
        .and_then(|i| view.matches.get(i))
        .copied();
    let mut visible = Vec::new();
    for row in offset..offset + height {
        if row > offset {
            visible.push(Span::new("\n"));
        }
        if total == 0 && row == 0 {
            visible.push(Span::new(if trace.is_some_and(|t| t.finished) {
                "  Trace is empty."
            } else {
                "  Waiting for log output…"
            }));
        } else if let Some(trace) = trace {
            visible.extend(trace.line_spans_with_current(
                row,
                &view.query,
                current_match == Some(row),
            ));
        }
    }
    let rows = Text::from_spans(visible)
        .height(Length::Px(height as u16))
        .width(Length::Flex(1))
        .overflow(Overflow::Clip)
        .style(style);
    // A viewport-sized wheel proxy, not a layout of the full trace. Logical
    // history and the scrollbar below use usize line positions, so logs longer
    // than terminal Rect's i16 coordinates are still navigable. Remount only the
    // wheel proxy after a gesture to reset its local offset without moving text.
    let focused = ctx.state.log_zoom && ctx.state.log_focus == Some(id);
    let wheel = ScrollView::new()
        .height(Length::Px(height as u16))
        .width(Length::Flex(1))
        .offset(height)
        .virtualize(false)
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .scrollbar(false)
        .scroll_wheel(ctx.state.log_focus == Some(id))
        .smooth_wheel_scroll(false)
        .scroll_wheel_multiplier(3)
        .padding(0)
        .style(style)
        .child(Spacer::new().height(Length::Px(height as u16)))
        .child(rows.clone())
        .child(Spacer::new().height(Length::Px(height as u16)))
        .on_scroll(ctx.link().callback(move |event: ScrollEvent| {
            Msg::LogWheel(id, offset, event.offset as isize - height as isize)
        }))
        .key(format!(
            "log-wheel-{id}-{offset}-{}-{height}",
            view.wheel_epoch
        ));
    let mut window = HStack::new()
        .height(Length::Px(height as u16))
        .style(style)
        .child(if focused { wheel } else { Element::from(rows) });
    if total > height {
        let (top, thumb) = super::logs::scrollbar(total, height, offset);
        let track_style =
            interaction::style(ctx, &format!("log-scrollbar-{id}"), style.fg(colors.muted));
        let thumb_style = track_style.fg(colors.accent);
        let mut track = VStack::new()
            .width(Length::Px(1))
            .height(Length::Px(height as u16))
            .style(track_style);
        for row in 0..height {
            track = track.child(
                Text::new(if (top..top + thumb).contains(&row) {
                    "█"
                } else {
                    "│"
                })
                .width(Length::Px(1))
                .height(Length::Px(1))
                .style(if (top..top + thumb).contains(&row) {
                    thumb_style
                } else {
                    track_style
                }),
            );
        }
        let hover_key = format!("log-scrollbar-{id}");
        window = window.child(Spacer::new().width(Length::Px(1))).child(
            Element::from(
                MouseRegion::new()
                    .hover_style(Style::new())
                    .capture_click(true)
                    .drag_threshold(1, 1)
                    .on_hover_change(
                        ctx.link()
                            .callback(move |entered| Msg::Hover(hover_key.clone(), entered)),
                    )
                    .on_mouse_down(ctx.link().callback(move |event: MouseRegionEvent| {
                        Msg::LogBar(id, event.local_y as usize, event.target_h as usize, true)
                    }))
                    .on_drag(ctx.link().callback(move |event: MouseDragEvent| {
                        Msg::LogBar(id, event.local_y as usize, event.target_h as usize, false)
                    }))
                    .on_drag_end(
                        ctx.link()
                            .callback(move |_: MouseDragEvent| Msg::LogBarEnd(id)),
                    )
                    .on_mouse_up(
                        ctx.link()
                            .callback(move |_: MouseRegionEvent| Msg::LogBarEnd(id)),
                    )
                    .child(track),
            )
            .key(format!("log-scrollbar-{id}")),
        );
    }
    Element::from(window).key(format!("log-window-{id}"))
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
            panel = panel.child(log_window(ctx, job, lines, colors));
            panel = if job.running() {
                let style = colors.base().bg(colors.surface).fg(colors.blue);
                panel.child(
                    HStack::new()
                        .height(Length::Px(1))
                        .style(style)
                        .child(Text::new("  ").style(style))
                        .child(dot_tooltip(
                            ctx,
                            format!("job-{}-live-status", job.id),
                            '◐',
                            colors.blue,
                            "Live log: Output is polled while this job is running",
                            style,
                        ))
                        .child(
                            Text::new(
                                if ctx.state.log_views.get(&job.id).is_none_or(|v| v.follow) {
                                    " LIVE · following tail"
                                } else {
                                    " LIVE · paused · End resumes following"
                                },
                            )
                            .style(style),
                        ),
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
            panel = panel
                .child(log_window(ctx, job, lines, colors))
                .child(line("  Waiting for trace…", colors.base().fg(colors.muted)));
        }
    }
    if let Some(view) = ctx
        .state
        .log_views
        .get(&job.id)
        .filter(|v| !v.query.is_empty())
    {
        panel = panel.child(line(
            format!(
                "  Search {:?}: {}{} · n/N next/previous",
                view.query,
                view.current_match.map_or(0, |i| i + 1),
                if view.matches.is_empty() {
                    " · no matches".into()
                } else {
                    format!("/{}", view.matches.len())
                }
            ),
            colors.base().fg(colors.yellow),
        ));
    }
    Element::from(panel).key(format!("trace-{}", job.id))
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
        let expanded = ctx.state.job_expanded(job);
        let id = job.id;
        let link = ctx.link().clone();
        let job_colors = if selected {
            Colors {
                background: colors.selection,
                surface: colors.selection,
                ..colors
            }
        } else {
            colors
        };
        let style = interaction::style(ctx, &format!("job-{id}"), job_colors.base());
        content.push(
            DetailRow::interactive(
                format!("job-{id}"),
                rich(job_header(job, expanded, colors), style),
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
            let mut row = DetailRow::keyed(
                format!("trace-{id}"),
                job_panel(ctx, job, if job.running() { 8 } else { 18 }, job_colors),
            );
            row.focused = selected;
            content.push(row);
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
        let author = discussion.notes.first().map(|note| &note.author);
        let link = ctx.link().clone();
        content.push(
            DetailRow::interactive(
                format!("discussion-{}", discussion.id),
                HStack::new()
                    .height(Length::Px(1))
                    .style(style)
                    .child(if let Some(author) = author {
                        user_tooltip(
                            ctx,
                            format!("discussion-{}-author", discussion.id),
                            author,
                            Text::new(ctx.state.render_user(author)).style(style.fg(colors.blue)),
                        )
                    } else {
                        Text::new("unknown").style(style.fg(colors.blue)).into()
                    })
                    .child(Text::new("  ").style(style))
                    .child(
                        Text::new(discussion_state(discussion))
                            .style(style.fg(colors.status(discussion_state(discussion)).1)),
                    )
                    .child(
                        Text::new(format!("  ·  {} notes", discussion.notes.len()))
                            .style(style.fg(colors.muted)),
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
                .map(|note| note_view(ctx, note, colors)),
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
                colors.blue
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

/// A paint-only animation: network polling and the rest of the UI keep their own cadence.
#[derive(Debug)]
struct SyncColorCycle {
    palette: [Color; 5],
    background_luminance: f64,
}

impl SyncColorCycle {
    fn new(colors: Colors) -> Self {
        Self {
            // Adjacent hues around the wheel, beginning at the existing sync blue.
            palette: [
                colors.blue,
                colors.green,
                colors.yellow,
                colors.red,
                colors.purple,
            ],
            background_luminance: Self::luminance(colors.surface),
        }
    }

    fn luminance(color: Color) -> f64 {
        let (r, g, b) = color.to_rgb().expect("sync colors are RGB");
        let [r, g, b] = [r, g, b].map(|channel| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn contrast(&self, color: Color) -> f64 {
        let fg = Self::luminance(color);
        let bg = self.background_luminance;
        (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05)
    }

    fn color(&self, elapsed: Duration) -> Color {
        let phase = elapsed.as_secs_f64().rem_euclid(5.0);
        let index = phase.floor() as usize;
        // One second per hue, with zero velocity at both ends (including the loop seam).
        let fraction = (phase - index as f64) as f32;
        let eased = Easing::EaseInOutSine.apply(fraction);
        let color =
            self.palette[index].blend_toward(self.palette[(index + 1) % self.palette.len()], eased);
        self.readable(color)
    }

    fn readable(&self, color: Color) -> Color {
        // Interpolated colors can have worse contrast than either endpoint. Check every
        // frame, including custom surfaces; retain hue by blending only toward a neutral.
        // A little headroom above 4.5:1 absorbs 8-bit channel rounding.
        const MINIMUM: f64 = 4.52;
        if self.contrast(color) >= MINIMUM {
            return color;
        }
        let black = Color::Rgb(0, 0, 0);
        let white = Color::Rgb(255, 255, 255);
        let target = if self.contrast(black) > self.contrast(white) {
            black
        } else {
            white
        };
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..12 {
            let mid = (low + high) / 2.0;
            if self.contrast(color.blend_toward(target, mid)) >= MINIMUM {
                high = mid;
            } else {
                low = mid;
            }
        }
        color.blend_toward(target, high)
    }
}

impl CellEffect for SyncColorCycle {
    fn apply(&self, cell: &mut EffectCell, ctx: &EffectContext) {
        cell.fg = self.color(ctx.elapsed).to_rgb().unwrap().into();
    }

    fn is_animated(&self) -> bool {
        true
    }

    fn animation_interval(&self) -> Duration {
        Duration::from_millis(33)
    }
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
        let cycle = SyncColorCycle::new(colors);
        let dot = dot_tooltip(
            ctx,
            "sync-status",
            '●',
            cycle.readable(colors.blue),
            "Syncing: Requests to GitLab are in progress",
            style,
        );
        let dot = if state.config.animations {
            EffectScope::new().custom_effect(cycle).child(dot).into()
        } else {
            dot
        };
        status_line = status_line.child(dot).child(
            Text::new(" Syncing  ")
                .height(Length::Px(1))
                .style(style.fg(colors.blue)),
        );
    }
    status_line = status_line.child(
        Text::new(if let Some(error) = &state.error {
            if ctx.elapsed() < state.blocked_until {
                format!(
                    "Retry in {}s · {error}",
                    (state.blocked_until - ctx.elapsed()).as_secs()
                )
            } else {
                format!("Error: {error}")
            }
        } else if ctx.elapsed() < state.blocked_until {
            format!(
                "Waiting for GitLab · retry in {}s",
                (state.blocked_until - ctx.elapsed()).as_secs()
            )
        } else if let Some((id, progress)) = state.sync_progress.first_key_value() {
            let project = state.project(*id).map_or("Project", |p| {
                if p.alias.is_empty() {
                    &p.path
                } else {
                    &p.alias
                }
            });
            let others = state.sync_progress.len().saturating_sub(1);
            format!(
                "{project} · {}{}",
                progress.label(),
                if others > 0 {
                    format!(" · +{others} projects")
                } else {
                    String::new()
                }
            )
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
                    .child(if matches!(dialog.kind, DialogKind::Onboarding) {
                        Element::from(
                            Text::new(
                                ctx.state
                                    .onboarding
                                    .feedback
                                    .get(index)
                                    .cloned()
                                    .unwrap_or_else(|| "Checking…".into()),
                            )
                            .height(Length::Auto)
                            .overflow(Overflow::Wrap)
                            .style(Style::new().fg(colors.muted)),
                        )
                    } else {
                        VStack::new().height(Length::Px(0)).into()
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
        for (index, theme) in crate::config::THEMES.iter().enumerate() {
            let (name, hint) = theme_description(theme);
            body = body.child(dialog_option(
                ctx,
                index,
                &format!("{name:<16} {hint}"),
                dialog.selected == index,
                colors,
                false,
            ));
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
    let hint = if matches!(dialog.kind, DialogKind::Onboarding) {
        "Enter save · Tab next · Ctrl+J newline · Esc skip"
    } else if commands {
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
            Text::new(if matches!(dialog.kind, DialogKind::Onboarding) {
                " Save setup "
            } else {
                " Confirm "
            })
            .style(interaction::style(
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
        Text::new(if matches!(dialog.kind, DialogKind::Onboarding) {
            " Skip setup "
        } else {
            " Close "
        })
        .style(interaction::style(
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
    let rows = c
        .options
        .iter()
        .enumerate()
        .map(|(option_index, _)| completion_row(ctx, index, c, option_index, colors))
        .collect::<Vec<_>>();
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

fn completion_row(
    ctx: &Context<Cronk>,
    index: usize,
    c: &Completion,
    option_index: usize,
    colors: Colors,
) -> Element {
    let option = &c.options[option_index];
    let selected = c.selected == option_index;
    let canvas = Colors {
        background: colors.surface,
        ..colors
    };
    let key = format!("lookup-{index}-{}", option.id);
    let style = interaction::style(
        ctx,
        &key,
        if selected {
            colors.selected()
        } else {
            canvas.base()
        },
    );
    let mut spans = vec![Span::new("  ").style(style)];
    if c.kind == LookupKind::Labels && !option.color.is_empty() {
        spans.push(label_pill_span(
            &Label {
                name: option.label.clone(),
                color: option.color.clone(),
                text_color: option.text_color.clone(),
            },
            colors,
        ));
    } else {
        spans.push(
            Span::new(if c.kind == LookupKind::Users {
                ctx.state.user_formatter.render_lookup(option)
            } else {
                option.label.clone()
            })
            .style(style),
        );
    }
    let header = selection_line(
        Text::from_spans(spans)
            .width(Length::Flex(1))
            .height(Length::Px(1))
            .overflow(Overflow::Ellipsis)
            .style(style),
        selected,
        canvas,
        style,
    );
    let description = selection_line(
        Text::new(format!("  {}", option.description))
            .width(Length::Flex(1))
            .height(Length::Px(1))
            .overflow(Overflow::Ellipsis)
            .style(style.fg(colors.muted)),
        selected,
        canvas,
        style,
    );
    let row = VStack::new()
        .height(Length::Px(2))
        .style(canvas.base())
        .child(header)
        .child(description);
    let epoch = c.epoch;
    click(ctx, key, row, move || {
        Msg::LookupAccept(index, epoch, option_index)
    })
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
        list_selection_line(
            line(
                format!("  {label}"),
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
            selected,
            Colors {
                background: colors.surface,
                ..colors
            },
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
- **Space** toggles visibility in Projects (list or details). Hidden projects leave the work lists.
- **Enter** opens project details. Alias is editable there; remove is a red action at the bottom.

## Workspace
- **/** filters the current view. Save a filter as a named tab from commands.
- **:** opens commands; type to search, then choose an action.
- **r** refreshes. **?** opens this help. **q** quits outside forms.
- The command palette exposes project creation, edits, comments,
  discussion replies/resolution, job retries, and themes. Project alias and
  removal live on the Projects detail view.

## Forms and live data
- **Enter** submits the whole form. **Tab / Shift+Tab** switch fields.
- **Ctrl+J** inserts a newline in multiline fields. **Esc** cancels.
- ID-backed fields suggest names: **Up / Down** choose, **Enter** accepts, then saves.
- Separate assignees/reviewers with commas; only the trimmed token at the caret is searched.
- Running jobs expand automatically; Space or the +/− header toggles any job.
- Manual collapse survives refresh and restart. Selection styling covers expanded logs.
- Enter focuses a job's logs; ↑/↓ and PgUp/PgDn scroll; Home/End go to start/live tail.
- Ctrl+↑/↓ and Ctrl+PgUp/PgDn scroll the selected job without focusing it.
- z zooms a selected/focused job. Zoom offers a draggable scrollbar and mouse-wheel scrolling.
- / searches loaded log history (literal, case-insensitive); n/N move between matching lines.
- Esc closes search, then zoom, then log focus, before leaving Jobs.
- Scrolling away from the bottom pauses following; End resumes following incoming output.
- Fetched log history stays in memory until the item's cache is discarded; it is never saved to disk.
- **DEMO** uses fictional offline data, never a live GitLab workspace.
";

#[cfg(test)]
mod tests {
    use super::*;

    fn linear_rgb(color: Color) -> [f64; 3] {
        let (r, g, b) = color.to_rgb().unwrap();
        [r, g, b].map(|channel| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        })
    }

    fn contrast_ratio(foreground: Color, background: Color) -> f64 {
        let luminance = |color| {
            let [r, g, b] = linear_rgb(color);
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let fg = luminance(foreground);
        let bg = luminance(background);
        (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05)
    }

    fn oklab(color: Color) -> [f64; 3] {
        let [r, g, b] = linear_rgb(color);
        let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
        let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
        let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
        [
            0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
        ]
    }

    #[test]
    fn sync_cycle_is_smooth_periodic_and_readable_in_every_theme() {
        for theme in crate::config::THEMES {
            let colors = themed(theme);
            let cycle = SyncColorCycle::new(colors);
            let start = cycle.color(Duration::ZERO);
            assert_eq!(start, cycle.color(Duration::from_secs(5)), "{theme}");
            assert_eq!(start, cycle.color(Duration::from_secs(500)), "{theme}");
            let mut previous = cycle.color(Duration::from_millis(4990));
            let mut unique = std::collections::HashSet::new();
            for millis in (0..5000).step_by(10) {
                let color = cycle.color(Duration::from_millis(millis));
                assert!(
                    contrast_ratio(color, colors.surface) >= 4.5,
                    "{theme}: {millis}ms"
                );
                let rgb = color.to_rgb().unwrap();
                unique.insert(rgb);
                let before = previous.to_rgb().unwrap();
                for (a, b) in [(rgb.0, before.0), (rgb.1, before.1), (rgb.2, before.2)] {
                    assert!(a.abs_diff(b) <= 5, "{theme}: abrupt change at {millis}ms");
                }
                previous = color;
            }
            assert!(unique.len() > 200, "{theme}: too few intermediate colors");
        }
    }

    #[test]
    fn sync_cycle_preserves_contrast_on_custom_surfaces() {
        // Include mid-grey (the hardest background), plus colored custom surfaces.
        for surface in [0x000000, 0xffffff, 0x757575, 0x808080, 0x236981, 0x985a76] {
            let colors = Colors {
                surface: Color::hex_u24(surface),
                ..themed("midnight")
            };
            let cycle = SyncColorCycle::new(colors);
            assert!(contrast_ratio(cycle.readable(colors.blue), colors.surface) >= 4.5);
            for millis in (0..5000).step_by(10) {
                let color = cycle.color(Duration::from_millis(millis));
                assert!(
                    contrast_ratio(color, colors.surface) >= 4.5,
                    "{surface:06x}: {millis}ms"
                );
            }
        }
    }

    #[test]
    fn user_tooltip_includes_available_profile_details_without_private_email_assumptions() {
        let user = User {
            id: 42,
            username: "opaque-9f3c".into(),
            name: "Alex Example".into(),
            public_email: Some("alex@example.org".into()),
            bot: true,
        };
        let help = user_help(&user);
        assert!(help.contains("Name: Alex Example"));
        assert!(help.contains("Username: @opaque-9f3c"));
        assert!(help.contains("GitLab user ID: 42"));
        assert!(help.contains("Public email: alex@example.org"));
        assert!(help.contains("Bot account"));

        let minimal = user_help(&User {
            username: "service-token-abc".into(),
            ..User::default()
        });
        assert!(minimal.contains("Username: @service-token-abc"));
        assert!(!minimal.contains("Public email"));
        assert!(!minimal.contains("Bot account"));
    }

    #[test]
    fn blue_balances_status_lightness_preserves_hue_and_has_readable_contrast() {
        let source = oklab(Color::hex_u24(0x1f78d1));
        let source_hue = source[2].atan2(source[1]);
        for theme in ["midnight", "dracula", "light"] {
            let colors = Colors::new(&Config {
                theme: theme.into(),
                ..Config::default()
            });
            let [lightness, a, b] = oklab(colors.blue);
            let target = (oklab(colors.green)[0] + oklab(colors.red)[0]) / 2.0;
            assert!(
                (lightness - target).abs() < 0.015,
                "{theme}: L={lightness}, target={target}"
            );
            assert!(
                (b.atan2(a) - source_hue).abs() < 0.05,
                "{theme}: blue hue drift"
            );
            for background in [colors.background, colors.surface, colors.selection] {
                let contrast = contrast_ratio(colors.blue, background);
                assert!(contrast >= 4.5, "{theme}: blue contrast {contrast:.2}:1");
            }
            if theme != "light" {
                let blue = contrast_ratio(colors.blue, colors.background);
                let red = contrast_ratio(colors.red, colors.background);
                let green = contrast_ratio(colors.green, colors.background);
                assert!(blue >= red.min(green) && blue <= red.max(green));
            }
        }
    }

    #[test]
    fn running_and_pending_status_styles_in_every_theme() {
        for theme in ["midnight", "dracula", "light"] {
            let colors = Colors::new(&Config {
                theme: theme.into(),
                ..Config::default()
            });
            let blue = Color::hex_u24(match theme {
                "dracula" => 0x7fbaff,
                "light" => 0x0066bc,
                _ => 0x7bb8ff,
            });
            assert_eq!(colors.blue, blue);
            assert_eq!(colors.status("running"), ('◐', blue));
            assert_eq!(colors.theme().document.link.fg, Some(blue.into()));
            assert_eq!(colors.theme().document.code_inline.fg, Some(blue.into()));
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
    fn list_construction_is_bounded_by_the_viewport() {
        for len in [20usize, 3_000, 30_000] {
            for offset in [0, len / 2, len.saturating_sub(20)] {
                let window = list_render_window(offset, 20, len);
                assert!(window.len() <= 60);
                assert!(window.contains(&offset));
                assert_eq!(window.end, (offset + 40).min(len));
            }
        }
        assert!(list_render_window(0, 20, 0).is_empty());
    }

    fn channel(value: u8) -> f64 {
        let v = f64::from(value) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let lum = |color: Color| {
            let (r, g, b) = color.to_rgb().unwrap();
            0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
        };
        let (hi, lo) = (lum(a).max(lum(b)), lum(a).min(lum(b)));
        (hi + 0.05) / (lo + 0.05)
    }

    fn themed(theme: &str) -> Colors {
        Colors::new(&Config {
            theme: theme.into(),
            ..Config::default()
        })
    }

    #[test]
    fn every_theme_has_a_description_and_a_distinct_palette() {
        let mut seen = std::collections::HashSet::new();
        for theme in crate::config::THEMES {
            assert_ne!(theme_description(theme).0, "Custom", "{theme}");
            let c = themed(theme);
            assert!(
                seen.insert(format!("{:?}{:?}{:?}", c.background, c.accent, c.selection)),
                "{theme} duplicates another palette"
            );
        }
    }

    #[test]
    fn new_themes_have_high_contrast_text_and_status_colors() {
        let mut failures = Vec::new();
        // The three original themes predate this bar and keep their palettes.
        for theme in &crate::config::THEMES[3..] {
            let c = themed(theme);
            for (surface_name, surface) in [
                ("background", c.background),
                ("surface", c.surface),
                ("selection", c.selection),
            ] {
                // Accent and status colors are glyphs and short words drawn over
                // selected rows too, where the non-text 3:1 bar applies.
                let colored = if surface_name == "selection" {
                    3.0
                } else {
                    4.5
                };
                for (name, fg, minimum) in [
                    ("foreground", c.foreground, 7.0),
                    ("muted", c.muted, 4.5),
                    ("accent", c.accent, colored),
                    ("green", c.green, colored),
                    ("yellow", c.yellow, colored),
                    ("red", c.red, colored),
                    ("blue", c.blue, colored),
                    ("purple", c.purple, colored),
                    ("pending", c.pending, colored),
                ] {
                    let ratio = contrast(fg, surface);
                    if ratio < minimum {
                        failures.push(format!(
                            "{theme}: {name} on {surface_name} is {ratio:.2}:1, need {minimum}:1"
                        ));
                    }
                }
            }
            // The selection must be visibly different from the page it sits on.
            if contrast(c.selection, c.background) < 1.15 {
                failures.push(format!("{theme}: selection too close to background"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn hover_is_visibly_different_from_rest_and_selection_in_new_themes() {
        let hover = ColorTransform::elevate(0.08);
        let mut failures = Vec::new();
        for theme in &crate::config::THEMES[3..] {
            let c = themed(theme);
            for (name, rest) in [
                ("background", c.background),
                ("surface", c.surface),
                ("selection", c.selection),
            ] {
                let hovered = hover.apply(rest);
                let ratio = contrast(hovered, rest);
                if ratio < 1.06 {
                    failures.push(format!("{theme}: hover on {name} only {ratio:.3}:1"));
                }
                // Hovered text must stay readable.
                let relaxed = name == "selection";
                for (text, fg, minimum) in [
                    ("foreground", c.foreground, if relaxed { 6.0 } else { 7.0 }),
                    ("muted", c.muted, if relaxed { 4.0 } else { 4.5 }),
                ] {
                    if contrast(fg, hovered) < minimum {
                        failures.push(format!(
                            "{theme}: {text} {:.2}:1 on hovered {name}",
                            contrast(fg, hovered)
                        ));
                    }
                }
            }
            // A hovered ordinary row must not be mistaken for a selected one. Selection
            // may differ by hue rather than brightness, so compare RGB distance.
            let (h, sel) = (
                hover.apply(c.background).to_rgb().unwrap(),
                c.selection.to_rgb().unwrap(),
            );
            let distance = ((i32::from(h.0) - i32::from(sel.0)).pow(2)
                + (i32::from(h.1) - i32::from(sel.1)).pow(2)
                + (i32::from(h.2) - i32::from(sel.2)).pow(2)) as f64;
            if distance.sqrt() < 14.0 {
                failures.push(format!(
                    "{theme}: hover resembles selection (distance {:.1})",
                    distance.sqrt()
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
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
            assert_eq!(frame.cell(0, 0).symbol, "u");
            assert_eq!(frame.cell(0, 0).fg, Color::hex_u24(0xabcdef));
            assert_eq!(frame.cell(0, 0).bg, Color::hex_u24(0x123456));
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
