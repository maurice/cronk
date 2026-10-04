use super::{Cronk, Dialog, DialogKind, Msg, Scope, State, list_height};
use crate::{build_info, config::Config, model::*};
use tui_lipan::{
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
            "running" => ('●', self.cyan),
            "failed" | "error" | "unresolved" => ('●', self.red),
            "merged" => ('●', self.purple),
            "pending" | "created" | "waiting_for_resource" | "preparing" => ('○', self.yellow),
            "manual" | "scheduled" => ('○', self.accent),
            _ => ('○', self.muted),
        }
    }
}

fn vertical_scrollbar(colors: Colors) -> ScrollbarConfig {
    ScrollbarConfig::new()
        .variant(ScrollbarVariant::Standalone)
        .gap(1)
        .thumb('█')
        .thumb_style(Style::new().fg(colors.accent).bg(colors.surface))
        .thumb_focus_style(Style::new().fg(colors.accent).bg(colors.surface))
        .track_style(Style::new().fg(colors.muted).bg(colors.surface))
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

fn click(
    ctx: &Context<Cronk>,
    key: impl Into<String>,
    child: impl Into<Element>,
    message: impl Fn() -> Msg + 'static,
) -> Element {
    Element::from(
        MouseRegion::new()
            .on_click(ctx.link().callback(move |_| message()))
            .child(child),
    )
    .key(key.into())
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

fn status_span(status: &str, colors: Colors) -> Span {
    let (symbol, color) = colors.status(status);
    Span::new(format!("{symbol} {status}")).fg(color)
}

fn label_spans(labels: &[Label], colors: Colors) -> Vec<Span> {
    let mut spans = Vec::with_capacity(labels.len() * 2);
    for label in labels {
        // GitLab supplies both colors. Neither selection nor contrast correction may change them.
        let style = Style::new()
            .bg(parse_color(&label.color).unwrap_or(colors.surface))
            .fg(parse_color(&label.text_color).unwrap_or(colors.foreground))
            .contrast_policy(ContrastPolicy::Off);
        spans.push(
            Span::new(format!(" {} ", label.name))
                .style(style)
                .row_style_policy(RowStylePolicy::Disabled),
        );
        spans.push(Span::new(" "));
    }
    spans
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
        .child(footer(state, colors));
    let mut layers = ZStack::new().child(shell);
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
            Text::new(if state.user.username.is_empty() {
                format!(
                    "{}  ·  {}",
                    state.config.theme,
                    build_info::display_version()
                )
            } else {
                format!(
                    "@{}  ·  {}  ·  {}",
                    state.user.username,
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
            Text::new("‹ Back  ").style(Style::new().fg(colors.accent)),
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
                    "Your open work · ordered by attention".into()
                } else {
                    "Across visible projects · most recently updated first".into()
                },
            )
        }
        Scope::Details => (
            "OVERVIEW".into(),
            "Choose a section · Enter to explore".into(),
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
        _ => "↑ ↓ scroll · Esc returns to overview",
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
    let slots = list_height(ctx.viewport().h);
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
    let list = ScrollView::new()
        .height(Length::Px((slots * 3).min(u16::MAX as usize) as u16))
        .offset(offset * 3)
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .scrollbar(true)
        .scrollbar_config(vertical_scrollbar(colors))
        .scroll_wheel_multiplier(3)
        .smooth_wheel_scroll(false)
        .estimated_child_height(3)
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
        .child(Element::from(list).key(format!("main-list-{}", state.config.active_tab)))
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
    let (circle, color) = colors.status(&item.state);
    let mut title = vec![
        Span::new(if selected && state.scope == Scope::List {
            "▎ "
        } else {
            "  "
        })
        .fg(colors.accent),
        Span::new(format!("{circle} ")).fg(color),
        Span::new(format!("{}  ", item_id(&item.key))).fg(colors.accent),
    ];
    if item.draft {
        title.push(Span::new("Draft · ").fg(colors.yellow));
    }
    title.push(Span::new(item.title.clone()).bold());
    let mut labels = vec![Span::new("    ")];
    labels.extend(label_spans(&item.labels, colors));
    if let Some(pipeline) = &item.pipeline {
        labels.push(status_span(&pipeline.status, colors));
        labels.push(Span::new("  ·  ").fg(colors.muted));
    }
    labels.push(Span::new(format!("@{}", item.author.username)).fg(colors.muted));
    if state.config.active_tab == 0 {
        labels.push(Span::new(format!("  ·  {}", item.attention_reason())).fg(colors.muted));
    }
    let identity = project_name(state, item.key.project);
    let top = HStack::new()
        .height(Length::Px(1))
        .style(style)
        .child(rich(title, style))
        .child(
            Text::new(format!(" {identity}  "))
                .style(Style::new().fg(colors.muted))
                .width(Length::Px((ctx.viewport().w / 3).min(32)))
                .height(Length::Px(1))
                .overflow(Overflow::Ellipsis),
        );
    click(
        ctx,
        format!("item-{}", item_key(&item.key)),
        VStack::new()
            .height(Length::Px(3))
            .style(colors.base())
            .child(top)
            .child(rich(labels, style))
            .child(blank()),
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
    let link = ctx.link().clone();
    let checkbox = click(
        ctx,
        format!("project-visible-{}", project.id),
        Text::new(if project.visible { " [x] " } else { " [ ] " }).style(Style::new().fg(
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

fn scroll_content(
    ctx: &Context<Cronk>,
    key: String,
    children: Vec<Element>,
    colors: Colors,
) -> Element {
    Element::from(
        ScrollView::new()
            // Measure variable-height content before dragging so the thumb's range stays accurate.
            .virtualize(false)
            .offset(ctx.state.content_offset)
            .scroll_keys(ScrollKeymap::NONE)
            .focusable(false)
            .tab_stop(false)
            .scrollbar(true)
            .scrollbar_config(vertical_scrollbar(colors))
            .scroll_wheel_multiplier(3)
            .smooth_wheel_scroll(false)
            .padding(content_padding(2))
            .style(colors.base())
            .on_scroll_to(ctx.link().callback(Msg::ContentScroll))
            .on_viewport_change(
                ctx.link()
                    .callback(|event: ScrollViewportEvent| Msg::ContentScroll(event.offset)),
            )
            .children(children),
    )
    .key(key)
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
    .border(false)
    .padding(0)
    .line_numbers(false)
    .wrap(true)
    .focusable(false)
    .tab_stop(false)
    .table_outer_frame(false)
}

fn section_document(ctx: &Context<Cronk>, value: &str, key: String, colors: Colors) -> Element {
    scroll_content(
        ctx,
        key,
        vec![
            markdown(value, colors)
                .height(Length::Auto)
                .scrollbar(false)
                .scroll_wheel(false)
                .into(),
        ],
        colors,
    )
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
    let key = item_key(&details.item.key);
    if state.scope == Scope::Details {
        return overview(ctx, details, colors);
    }
    match state.section_name() {
        "Fields" => fields(ctx, details, colors),
        "Description" => section_document(
            ctx,
            &details.item.description,
            format!("description-{key}"),
            colors,
        ),
        "Activity" => {
            let mut content = Vec::new();
            for note in &details.notes {
                content.push(note_view(note, colors));
            }
            if content.is_empty() {
                content.push(line("No activity yet.", Style::new().fg(colors.muted)).into());
            }
            scroll_content(ctx, format!("activity-{key}"), content, colors)
        }
        "Pipeline" => pipeline(ctx, details, colors),
        "Jobs" => jobs(ctx, details, colors),
        "Discussions" => discussions(ctx, details, colors),
        "Changes" => changes(ctx, details, colors),
        _ => empty_state(
            "Section unavailable",
            "Return to the overview to choose a section.",
            colors,
        ),
    }
}

fn metadata(label: &str, value: impl Into<String>, colors: Colors) -> Element {
    let value = value.into();
    HStack::new()
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

fn users(users: &[User]) -> String {
    users
        .iter()
        .map(|u| format!("@{}", u.username))
        .collect::<Vec<_>>()
        .join(", ")
}

fn overview(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Element {
    let state = &ctx.state;
    let item = &details.item;
    let mut navigation = Vec::new();
    for (index, section) in state.sections().iter().enumerate() {
        let selected = state.section_cursor == index;
        let count = match *section {
            "Jobs" => format!("{}", details.jobs.len()),
            "Discussions" => format!("{}", details.discussions.len()),
            "Changes" => format!("{}", details.diffs.len()),
            "Activity" => format!("{}", details.notes.len()),
            _ => String::new(),
        };
        navigation.push(click(
            ctx,
            format!("overview-section-{index}"),
            VStack::new()
                .height(Length::Px(2))
                .style(if selected {
                    colors.selected()
                } else {
                    colors.base()
                })
                .child(line(
                    format!(" {} {section}  {count}", if selected { "›" } else { " " }),
                    Style::new()
                        .fg(if selected {
                            colors.accent
                        } else {
                            colors.foreground
                        })
                        .bold(),
                ))
                .child(line(
                    if selected { "   Enter to explore" } else { "" },
                    Style::new().fg(colors.muted),
                )),
            move || Msg::Section(index),
        ));
    }
    let mut content = vec![
        Text::new(item.title.clone())
            .width(Length::Flex(1))
            .height(Length::Auto)
            .overflow(Overflow::Wrap)
            .style(Style::new().fg(colors.foreground).bold())
            .into(),
        rich(
            vec![
                status_span(&item.state, colors),
                Span::new(format!("   {}", item_id(&item.key))).fg(colors.accent),
            ],
            colors.base(),
        )
        .into(),
        rich(label_spans(&item.labels, colors), colors.base()).into(),
        blank(),
        metadata(
            "Project",
            state
                .project(item.key.project)
                .map_or_else(|| item.key.project.to_string(), |p| p.path.clone()),
            colors,
        ),
        metadata("Author", format!("@{}", item.author.username), colors),
        metadata("Assignees", users(&item.assignees), colors),
        metadata("Milestone", item.milestone.clone(), colors),
        metadata("Updated", item.updated_at.clone(), colors),
    ];
    if item.key.kind == ItemKind::MergeRequest {
        content.push(metadata(
            "Branches",
            format!("{} → {}", item.source_branch, item.target_branch),
            colors,
        ));
        content.push(metadata("Reviewers", users(&item.reviewers), colors));
        content.push(metadata(
            "Pipeline",
            item.pipeline
                .as_ref()
                .map_or_else(|| "None".into(), |p| format!("#{} · {}", p.id, p.status)),
            colors,
        ));
        content.push(metadata(
            "Review",
            format!(
                "{} jobs · {} discussions · {} unresolved · {} changed files",
                details.jobs.len(),
                details.discussions.len(),
                item.unresolved
                    .map_or_else(|| "unknown".into(), |n| n.to_string()),
                details.diffs.len()
            ),
            colors,
        ));
    } else {
        content.push(metadata("Iteration", item.iteration.clone(), colors));
    }
    content.push(metadata("URL", item.web_url.clone(), colors));
    for warning in &details.warnings {
        content.push(
            Text::new(format!("○ {warning}"))
                .width(Length::Flex(1))
                .overflow(Overflow::Wrap)
                .style(Style::new().fg(colors.yellow))
                .into(),
        );
    }
    content.push(blank());
    content.push(line("DESCRIPTION", Style::new().fg(colors.accent).bold()).into());
    content.push(
        markdown(&item.description, colors)
            .height(Length::Auto)
            .scroll_wheel(false)
            .into(),
    );
    if item.key.kind == ItemKind::Issue {
        content.push(blank());
        content.push(
            line(
                format!("RECENT ACTIVITY  ·  {} events", details.notes.len()),
                Style::new().fg(colors.accent).bold(),
            )
            .into(),
        );
        let start = details.notes.len().saturating_sub(3);
        for note in &details.notes[start..] {
            content.push(note_view(note, colors));
        }
        if details.notes.is_empty() {
            content.push(line("No activity yet.", Style::new().fg(colors.muted)).into());
        }
    }
    let summary = scroll_content(
        ctx,
        format!("overview-{}", item_key(&item.key)),
        content,
        colors,
    );
    if ctx.viewport().w < 70 {
        VStack::new()
            .child(
                Tabs::new()
                    .tabs(state.sections().iter().map(|s| Tab::new(*s)))
                    .active(state.section_cursor)
                    .height(Length::Px(1))
                    .border(false)
                    .overflow(TabsOverflow::Ellipsis)
                    .focusable(false)
                    .tab_stop(false)
                    .active_style(colors.selected().fg(colors.accent))
                    .on_change(ctx.link().callback(|e: TabsEvent| Msg::Section(e.index))),
            )
            .child(summary)
            .into()
    } else {
        HStack::new()
            .child(
                ScrollView::new()
                    .width(Length::Px(23))
                    .reveal_key(format!("overview-section-{}", state.section_cursor))
                    .padding(content_padding(1))
                    .scroll_keys(ScrollKeymap::NONE)
                    .focusable(false)
                    .tab_stop(false)
                    .scrollbar(true)
                    .scrollbar_config(vertical_scrollbar(colors))
                    .gap(1)
                    .children(navigation),
            )
            .child(summary)
            .into()
    }
}

fn fields(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Element {
    let mut content = vec![
        metadata(
            "Project path",
            ctx.state
                .project(details.item.key.project)
                .map_or_else(|| details.item.key.project.to_string(), |p| p.path.clone()),
            colors,
        ),
        blank(),
    ];
    for (index, (label, _, value)) in ctx.state.fields().into_iter().enumerate() {
        let selected = ctx.state.config.field == index;
        let style = if selected {
            colors.selected()
        } else {
            colors.base()
        };
        let mut spans = vec![
            Span::new(format!(
                " {} {label:<19} ",
                if selected { "›" } else { " " }
            ))
            .fg(colors.accent),
        ];
        if label == "Labels" {
            spans.extend(label_spans(&details.item.labels, colors));
        } else {
            spans.push(Span::new(if value.is_empty() {
                "—".into()
            } else {
                value
            }));
        }
        content.push(click(
            ctx,
            format!("edit-field-{index}"),
            VStack::new()
                .height(Length::Px(2))
                .style(style)
                .child(rich(spans, style))
                .child(line(
                    if selected {
                        "   Enter or click to edit"
                    } else {
                        ""
                    },
                    Style::new().fg(colors.muted),
                )),
            move || Msg::Field(index),
        ));
    }
    content.push(blank());
    content.push(metadata(
        "Milestone",
        details.item.milestone.clone(),
        colors,
    ));
    if details.item.key.kind == ItemKind::Issue {
        content.push(metadata(
            "Iteration",
            details.item.iteration.clone(),
            colors,
        ));
    }
    content.push(metadata("Web URL", details.item.web_url.clone(), colors));
    Element::from(
        ScrollView::new()
            .virtualize(false)
            .offset(ctx.state.content_offset)
            .reveal_key(format!("edit-field-{}", ctx.state.config.field))
            .scroll_keys(ScrollKeymap::NONE)
            .focusable(false)
            .tab_stop(false)
            .scrollbar(true)
            .scrollbar_config(vertical_scrollbar(colors))
            .padding(content_padding(2))
            .smooth_wheel_scroll(false)
            .on_scroll_to(ctx.link().callback(Msg::ContentScroll))
            .on_viewport_change(
                ctx.link()
                    .callback(|event: ScrollViewportEvent| Msg::ContentScroll(event.offset)),
            )
            .children(content),
    )
    .key(format!("fields-{}", item_key(&details.item.key)))
}

fn note_view(note: &Note, colors: Colors) -> Element {
    let mut header = vec![
        Span::new(format!("@{}", note.author.username))
            .fg(colors.cyan)
            .bold(),
        Span::new(format!("   {}", note.created_at)).fg(colors.muted),
    ];
    if note.system {
        header.push(Span::new("  ·  system").fg(colors.muted));
    }
    if note.resolvable {
        header.push(Span::new("  ·  "));
        header.push(status_span(
            if note.resolved {
                "resolved"
            } else {
                "unresolved"
            },
            colors,
        ));
    }
    Element::from(
        VStack::new()
            .height(Length::Auto)
            .child(rich(header, colors.base()))
            .child(
                markdown(&note.body, colors)
                    .height(Length::Auto)
                    .scroll_wheel(false),
            )
            .child(blank()),
    )
    .key(format!("note-{}", note.id))
}

fn pipeline(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Element {
    let mut content = Vec::new();
    if let Some(pipeline) = &details.item.pipeline {
        content.push(
            rich(
                vec![
                    Span::new(format!("Pipeline #{}   ", pipeline.id)).bold(),
                    status_span(&pipeline.status, colors),
                ],
                colors.base(),
            )
            .into(),
        );
        content.push(metadata("Web URL", pipeline.web_url.clone(), colors));
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
    content.push(
        rich(
            vec![
                Span::new(format!("● {} running   ", count("running"))).fg(colors.cyan),
                Span::new(format!(
                    "○ {} pending   ",
                    count("pending") + count("created")
                ))
                .fg(colors.yellow),
                Span::new(format!("● {} passed   ", count("success"))).fg(colors.green),
                Span::new(format!("● {} failed   ", count("failed"))).fg(colors.red),
                Span::new(format!("{} total", details.jobs.len())).fg(colors.muted),
            ],
            colors.base(),
        )
        .into(),
    );
    content.push(blank());
    let running: Vec<_> = details.jobs.iter().filter(|j| j.running()).collect();
    if running.is_empty() {
        content.push(
            line(
                "No running jobs. Open Jobs to inspect completed traces.",
                Style::new().fg(colors.muted),
            )
            .into(),
        );
    } else {
        content.push(
            line(
                "LIVE TRACES  ·  following every running job",
                Style::new().fg(colors.accent).bold(),
            )
            .into(),
        );
        let columns = if ctx.viewport().w >= 105 { 2 } else { 1 };
        let groups = running.len().div_ceil(columns);
        // Share the available height: concurrent logs should remain visible even at 80×24.
        let lines = (ctx.viewport().h.saturating_sub(14) as usize / groups)
            .saturating_sub(3)
            .clamp(1, 7);
        for chunk in running.chunks(columns) {
            let mut row = HStack::new().height(Length::Auto).gap(2);
            for job in chunk {
                row = row.child(job_panel(ctx, job, lines, true, colors));
            }
            content.push(row.into());
            content.push(blank());
        }
    }
    scroll_content(
        ctx,
        format!("pipeline-{}", item_key(&details.item.key)),
        content,
        colors,
    )
}

fn job_header(job: &Job, selected: bool, expanded: bool, colors: Colors) -> Vec<Span> {
    let mut spans = vec![
        Span::new(if selected { "› " } else { "  " }).fg(colors.accent),
        Span::new(if expanded { "− " } else { "+ " }).fg(colors.muted),
        Span::new(job.name.clone()).bold(),
        Span::new(format!("  {}  #{}  ", job.stage, job.id)).fg(colors.muted),
        status_span(&job.status, colors),
    ];
    if job.allow_failure {
        spans.push(Span::new("  ·  allowed failure").fg(colors.yellow));
    }
    spans
}

fn job_panel(
    ctx: &Context<Cronk>,
    job: &Job,
    lines: usize,
    header: bool,
    colors: Colors,
) -> Element {
    let mut panel = VStack::new()
        .height(Length::Auto)
        .style(Style::new().bg(colors.surface));
    if header {
        panel = panel.child(rich(
            job_header(job, false, true, colors),
            Style::new().bg(colors.surface),
        ));
    }
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
            panel = panel.child(line(
                if job.running() {
                    "  ● LIVE · trailing output"
                } else if trace.finished {
                    "  End of trace"
                } else {
                    "  Loading trace…"
                },
                Style::new().fg(if job.running() {
                    colors.cyan
                } else {
                    colors.muted
                }),
            ));
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

fn jobs(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Element {
    if details.jobs.is_empty() {
        return empty_state(
            "No jobs",
            "This pipeline has not created any jobs yet.",
            colors,
        );
    }
    let mut content = Vec::new();
    for (index, job) in details.jobs.iter().enumerate() {
        let selected = ctx.state.config.field == index;
        let expanded = job.running() || ctx.state.expanded.contains(&job.id);
        let id = job.id;
        let link = ctx.link().clone();
        content.push(click(
            ctx,
            format!("job-{id}"),
            rich(
                job_header(job, selected, expanded, colors),
                if selected {
                    colors.selected()
                } else {
                    colors.base()
                },
            ),
            move || {
                link.send(Msg::Select(index));
                Msg::ToggleJob(id)
            },
        ));
        if expanded {
            content.push(job_panel(
                ctx,
                job,
                if job.running() { 8 } else { 18 },
                false,
                colors,
            ));
        }
        content.push(blank());
    }
    let mut scroll = ScrollView::new()
        .virtualize(false)
        .offset(ctx.state.content_offset)
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .scrollbar(true)
        .scrollbar_config(vertical_scrollbar(colors))
        .padding(content_padding(2))
        .smooth_wheel_scroll(false)
        .on_scroll_to(ctx.link().callback(Msg::ContentScroll))
        .on_viewport_change(
            ctx.link()
                .callback(|event: ScrollViewportEvent| Msg::ContentScroll(event.offset)),
        )
        .children(content);
    if let Some(job) = details.jobs.get(ctx.state.config.field) {
        scroll = scroll.reveal_key(format!("job-{}", job.id));
    }
    Element::from(scroll).key(format!("jobs-{}", item_key(&details.item.key)))
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

fn discussions(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Element {
    if details.discussions.is_empty() {
        return empty_state(
            "No discussions",
            "Add a comment to start the review conversation.",
            colors,
        );
    }
    let selected = ctx.state.config.field.min(details.discussions.len() - 1);
    let mut summaries = Vec::new();
    for (index, discussion) in details.discussions.iter().enumerate() {
        let style = if selected == index {
            colors.selected()
        } else {
            colors.base()
        };
        let author = discussion
            .notes
            .first()
            .map_or("unknown", |n| n.author.username.as_str());
        let preview = discussion
            .notes
            .first()
            .map_or("Empty discussion", |n| n.body.lines().next().unwrap_or(""));
        summaries.push(click(
            ctx,
            format!("discussion-{}", discussion.id),
            VStack::new()
                .height(Length::Px(3))
                .style(style)
                .child(rich(
                    vec![
                        Span::new(format!(
                            " {} @{}  ",
                            if selected == index { "›" } else { " " },
                            author
                        ))
                        .fg(colors.cyan),
                        status_span(discussion_state(discussion), colors),
                        Span::new(format!("  ·  {} notes", discussion.notes.len()))
                            .fg(colors.muted),
                    ],
                    style,
                ))
                .child(line(format!("   {preview}"), style))
                .child(blank()),
            move || Msg::Select(index),
        ));
    }
    let notes: Vec<_> = details.discussions[selected]
        .notes
        .iter()
        .map(|note| note_view(note, colors))
        .collect();
    let content = scroll_content(
        ctx,
        format!("discussion-notes-{}", details.discussions[selected].id),
        notes,
        colors,
    );
    let summary = ScrollView::new()
        .scroll_keys(ScrollKeymap::NONE)
        .focusable(false)
        .tab_stop(false)
        .padding(content_padding(1))
        .scrollbar(true)
        .scrollbar_config(vertical_scrollbar(colors))
        .reveal_key(format!("discussion-{}", details.discussions[selected].id))
        .children(summaries);
    if ctx.viewport().w >= 100 {
        HStack::new()
            .child(summary.width(Length::Px(42)))
            .child(content)
            .into()
    } else {
        VStack::new()
            .child(summary.height(Length::Px(6)))
            .child(content)
            .into()
    }
}

fn changes(ctx: &Context<Cronk>, details: &Details, colors: Colors) -> Element {
    if details.diffs.is_empty() {
        return empty_state(
            "No changes available",
            "GitLab returned no file diffs for this merge request.",
            colors,
        );
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
        content.push(
            Element::from(line(
                format!("{path}  ·  {kind}"),
                Style::new().fg(colors.accent).bg(colors.surface).bold(),
            ))
            .key(format!("diff-file-{index}")),
        );
        if diff.too_large || diff.collapsed {
            content.push(line(if diff.too_large { "○ GitLab omitted this diff because it is too large." }
                else { "○ GitLab returned a collapsed diff; open the merge request URL for the full patch." }, Style::new().fg(colors.yellow)).into());
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
            content.push(
                Element::from(
                    Text::new(text.to_owned())
                        .width(Length::Flex(1))
                        .height(Length::Auto)
                        .overflow(Overflow::Wrap)
                        .style(Style::new().fg(foreground)),
                )
                .key(format!("diff-{index}-line-{number}")),
            );
        }
        content.push(blank());
    }
    scroll_content(
        ctx,
        format!("changes-{}", item_key(&details.item.key)),
        content,
        colors,
    )
}

fn footer(state: &State, colors: Colors) -> Element {
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
    if busy {
        status.push(
            Span::new(if state.tick.is_multiple_of(2) {
                "● Syncing  "
            } else {
                "○ Syncing  "
            })
            .fg(colors.cyan),
        );
    }
    if let Some(error) = &state.error {
        status.push(Span::new(format!("Error: {error}")).fg(colors.red));
    } else {
        status.push(Span::new(state.status.clone()).fg(colors.muted));
    }
    let shortcuts = match state.scope {
        Scope::List => "↑ ↓ move   Enter open   / filter   : commands   r refresh   ? help",
        Scope::Details => {
            " ↑ ↓ section   Enter explore   Esc back   : commands   r refresh   ? help"
        }
        Scope::Section => {
            " ↑ ↓ navigate   Enter act   Esc overview   : commands   r refresh   ? help"
        }
    };
    VStack::new()
        .height(Length::Px(2))
        .style(Style::new().bg(colors.surface))
        .child(rich(status, Style::new().bg(colors.surface)))
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
        let interceptor = ctx
            .link()
            .key_handler(move |key| dialog_key(key, index, selected, count, multiline, commands));
        let normal = Style::new().fg(colors.foreground).bg(colors.background);
        let focus = Style::new().fg(colors.foreground).bg(colors.selection);
        let editor: Element = if field.multiline {
            TextArea::bound(&field.editor)
                .height(Length::Px((ctx.viewport().h / 5).clamp(3, 8)))
                .border(false)
                .padding(content_padding(1))
                .scrollbar(true)
                .scrollbar_config(vertical_scrollbar(colors))
                .line_numbers(false)
                .wrap(true)
                .style(normal)
                .focus_style(focus)
                .focus_content_style(focus)
                .selection_style(colors.selected())
                .caret_color(colors.accent)
                .key_interceptor(interceptor)
                .insert_tab(false)
                .on_click(ctx.link().callback(move |_| Msg::DialogField(index)))
                .on_change(ctx.link().callback(move |event| Msg::Editor(index, event)))
                .into()
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
                .on_click(ctx.link().callback(move |_| Msg::DialogField(index)))
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
                    .child(editor.key(format!("dialog-field-{index}"))),
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
            Text::new(" Confirm ")
                .style(Style::new().fg(colors.accent).bg(colors.selection).bold()),
            || Msg::Submit,
        ));
    }
    actions = actions.child(click(
        ctx,
        "dialog-close",
        Text::new(" Close ").style(Style::new().fg(colors.muted)),
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
        .scrollbar_config(vertical_scrollbar(colors))
        .smooth_wheel_scroll(false)
        .on_scroll(ctx.link().callback(|_: ScrollEvent| Msg::DialogScrolled));
    if dialog.reveal_selection {
        if commands || matches!(dialog.kind, DialogKind::Themes) {
            scroll = scroll.reveal_key(format!("dialog-option-{}", dialog.selected));
        } else if !dialog.fields.is_empty() {
            scroll = scroll.reveal_key(format!("dialog-field-{}", dialog.selected));
        }
    }
    let body = Element::from(scroll)
        .max_height(Length::Px(ctx.viewport().h.saturating_sub(6).max(1)))
        .key("dialog-scroll");
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

fn dialog_option(
    ctx: &Context<Cronk>,
    index: usize,
    label: &str,
    selected: bool,
    colors: Colors,
) -> Element {
    let link = ctx.link().clone();
    click(
        ctx,
        format!("dialog-option-{index}"),
        line(
            format!(" {} {label}", if selected { "›" } else { " " }),
            if selected {
                colors.selected().fg(colors.accent).bold()
            } else {
                Style::new().fg(colors.foreground)
            },
        ),
        move || {
            link.send(Msg::DialogSelect(index));
            Msg::Submit
        },
    )
}

const HELP: &str = "\
## Navigation
- **Shift+D / P / I / M** open Dashboard, Projects, Issues, or Merge Requests.
- **1–9 / 0** open saved views 1–10. Tab shortcuts do not run inside dialogs.
- Lists are active immediately. Arrow keys never switch tabs.
- Overflowing panes show a right-edge scrollbar. Click its track or drag its thumb to scroll.
- **Enter** opens the selected item, section, field, or job.
- **Up / Down** move within the current scope. Document sections scroll.
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
- Running jobs expand automatically. Pipeline shows all running traces.
- Job logs follow their trailing output; finished jobs expand on demand.
- **DEMO** uses fictional offline data, never a live GitLab workspace.
";

#[cfg(test)]
mod tests {
    use super::*;

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
