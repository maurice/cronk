use super::*;
use crate::{
    config::{SavedView, TabState},
    demo,
};
use tui_lipan::{CommandLink, TaskPolicy};

impl Component for Cronk {
    type Message = Msg;
    type Properties = ();
    type State = State;

    fn create_state(&self, _: &()) -> State {
        let mut config = self.config.clone();
        let user_formatter = UserFormatter::from_config(&config)
            .expect("configuration is validated before creating UI state");
        config.active_tab = config.active_tab.min(3 + config.views.len());
        if config.route.as_ref().is_some_and(|key| {
            !config
                .projects
                .iter()
                .any(|p| p.id == key.project && p.visible)
        }) {
            config.route = None;
            config.section = None;
        }
        prune_tab_routes(&mut config);
        let saved = config
            .tab_states
            .get(&config.active_tab.to_string())
            .cloned();
        if let Some(tab) = &saved {
            config.route = tab.route.clone();
            config.section = tab.section;
            config.field = tab.field;
        }
        let is_demo = self.api.is_none();
        let scope = if config.route.is_some() {
            if config.section.is_some() {
                Scope::Section
            } else {
                Scope::Details
            }
        } else {
            Scope::List
        };
        let details = if is_demo {
            config.route.as_ref().map(demo::details)
        } else {
            None
        };
        let items = if is_demo { demo::items() } else { vec![] };
        let mut current_iterations = HashMap::new();
        if is_demo {
            for item in &items {
                if let (Some(id), false) = (item.iteration_id, item.iteration.trim().is_empty()) {
                    current_iterations
                        .entry(item.key.project)
                        .or_insert_with(|| {
                            Some(CurrentIteration {
                                id,
                                title: item.iteration.clone(),
                                description: format!("{} · Demo · ID {id}", item.iteration),
                            })
                        });
                }
            }
        }
        let selected = config
            .selections
            .get(&config.active_tab.to_string())
            .copied()
            .unwrap_or(0);
        let section_cursor = saved
            .as_ref()
            .map_or(config.section.unwrap_or(0), |tab| tab.section_cursor)
            .min(
                if config
                    .route
                    .as_ref()
                    .is_some_and(|k| k.kind == ItemKind::MergeRequest)
                {
                    5
                } else {
                    2
                },
            );
        State {
            config,
            feedback: Default::default(),
            demo: is_demo,
            scope,
            items,
            user: if is_demo {
                demo::user()
            } else {
                User::default()
            },
            user_formatter,
            details,
            scroll: BoundaryScroll {
                selected,
                offset: saved
                    .as_ref()
                    .map_or(selected.saturating_sub(2), |tab| tab.list_offset),
            },
            section_cursor,
            content_offset: saved.as_ref().map_or(0, |tab| tab.content_offset),
            content_max_offset: usize::MAX,
            reveal_content: saved.is_none(),
            detail_viewport: None,
            detail_offset_request: None,
            tab_cache: BTreeMap::new(),
            traces: HashMap::new(),
            expanded: saved
                .as_ref()
                .map_or_else(HashSet::new, |tab| tab.expanded.iter().copied().collect()),
            collapsed: saved.map_or_else(HashSet::new, |tab| tab.collapsed.into_iter().collect()),
            log_views: HashMap::new(),
            log_focus: None,
            log_zoom: false,
            log_zoom_from_focus: false,
            log_search: None,
            dialog: None,
            current_iterations,
            status: if is_demo {
                "Demo workspace · no requests or remote writes".into()
            } else {
                "Connecting to GitLab…".into()
            },
            error: None,
            list_pending: HashSet::new(),
            detail_pending: None,
            trace_pending: HashSet::new(),
            mutation_pending: false,
            user_pending: false,
            list_epoch: 0,
            detail_epoch: 0,
            next_lists: Duration::ZERO,
            next_details: Duration::ZERO,
            next_traces: Duration::ZERO,
            failures: 0,
            blocked_until: Duration::ZERO,
            tick: 0,
            lookup_epoch: 0,
            lookup_inflight: None,
        }
    }

    fn init(&mut self, ctx: &mut Context<Self>) -> Option<Command> {
        ctx.link().send(Msg::Tick);
        ctx.link().send(Msg::Refresh);
        None
    }

    fn view(&self, ctx: &Context<Self>) -> Element {
        view::view(ctx)
    }

    fn update(&mut self, msg: Msg, ctx: &mut Context<Self>) -> Update {
        if matches!(
            &msg,
            Msg::Move(_)
                | Msg::Enter
                | Msg::Back
                | Msg::Section(_)
                | Msg::DetailSection(_)
                | Msg::Field(_)
                | Msg::ScrollContent(_)
                | Msg::DetailScrolled(_)
                | Msg::Tab(_)
                | Msg::Action(_)
                | Msg::Palette
        ) {
            ctx.state.feedback.tooltip = None;
        }
        match msg {
            Msg::StatusHover(key, text, position) => {
                return if ctx.state.feedback.status_hover(key, text, position) {
                    Update::full()
                } else {
                    Update::none()
                };
            }
            Msg::Hover(key, entered) => {
                return if ctx.state.feedback.hover(key, entered) {
                    Update::full()
                } else {
                    Update::none()
                };
            }
            Msg::ClickFlash(key) => {
                if !ctx.state.config.animations {
                    return Update::none();
                }
                let generation = ctx.state.feedback.flash(key.clone());
                return Update::with_command(Command::after(
                    super::interaction::CLICK_FLASH,
                    move |link: CommandLink<Msg>| link.send(Msg::EndClickFlash(key, generation)),
                ));
            }
            Msg::EndClickFlash(key, generation) => {
                return if ctx.state.feedback.end_flash(&key, generation) {
                    Update::full()
                } else {
                    Update::none()
                };
            }
            Msg::Tick => {
                ctx.state.tick += 1;
                let now = ctx.elapsed();
                if now >= ctx.state.blocked_until {
                    if ctx.state.user.id == 0 && !ctx.state.user_pending {
                        ctx.link().send(Msg::LoadUser);
                    }
                    if ctx.state.scope == Scope::List && now >= ctx.state.next_lists {
                        self.queue_lists(ctx);
                    }
                    if ctx.state.config.route.is_some() && now >= ctx.state.next_details {
                        ctx.link().send(Msg::LoadDetails);
                    }
                    if ctx.state.config.route.is_some() && now >= ctx.state.next_traces {
                        ctx.link().send(Msg::LoadTraces);
                    }
                }
                return Update::with_command(Command::after(
                    Duration::from_secs(1),
                    |link: CommandLink<Msg>| link.send(Msg::Tick),
                ));
            }
            Msg::Refresh => {
                if ctx.elapsed() < ctx.state.blocked_until {
                    ctx.state.status =
                        "GitLab backoff is active; refresh will resume automatically".into();
                } else {
                    ctx.state.error = None;
                    self.queue_lists(ctx);
                    ctx.link().send(Msg::LoadDetails);
                    ctx.link().send(Msg::LoadTraces);
                }
            }
            Msg::LoadProject(id, epoch) => {
                if epoch != ctx.state.list_epoch {
                    return Update::none();
                }
                if ctx.elapsed() < ctx.state.blocked_until {
                    ctx.state.list_pending.remove(&id);
                    return Update::none();
                }
                let Some(project) = ctx.state.project(id).filter(|p| p.visible).cloned() else {
                    ctx.state.list_pending.remove(&id);
                    return Update::none();
                };
                let Some(api) = self.api.clone() else {
                    return Update::none();
                };
                return Update::with_command(ctx.link().command(move |link| {
                    let result = api
                        .list_project(&project)
                        .map(|items| (items, api.current_iteration(project.id).ok().flatten()))
                        .map_err(|e| e.to_string());
                    link.send(Msg::ProjectLoaded(id, epoch, result));
                }));
            }
            Msg::LoadUser => {
                if ctx.state.user_pending || ctx.elapsed() < ctx.state.blocked_until {
                    return Update::none();
                }
                let Some(api) = self.api.clone() else {
                    return Update::none();
                };
                ctx.state.user_pending = true;
                return Update::with_command(ctx.link().command(move |link| {
                    link.send(Msg::UserLoaded(
                        api.current_user().map_err(|e| e.to_string()),
                    ));
                }));
            }
            Msg::UserLoaded(result) => {
                ctx.state.user_pending = false;
                match result {
                    Ok(user) => ctx.state.user = user,
                    Err(error) => self.network_error(ctx, error),
                }
            }
            Msg::ProjectLoaded(id, epoch, result) => {
                if epoch != ctx.state.list_epoch {
                    return Update::none();
                }
                ctx.state.list_pending.remove(&id);
                if !ctx.state.project(id).is_some_and(|p| p.visible) {
                    return Update::none();
                }
                let selected = ctx
                    .state
                    .visible_items()
                    .get(ctx.state.scroll.selected)
                    .map(|i| i.key.clone());
                match result {
                    Ok((items, current_iteration)) => {
                        ctx.state.items.retain(|i| i.key.project != id);
                        ctx.state.items.extend(items);
                        ctx.state.current_iterations.insert(id, current_iteration);
                        if let Some(key) = selected
                            && let Some(index) =
                                ctx.state.visible_items().iter().position(|i| i.key == key)
                        {
                            ctx.state.scroll.selected = index;
                        }
                        self.normalize(ctx);
                        self.persist(ctx);
                        if ctx.state.list_pending.is_empty() {
                            ctx.state.status = "Visible projects are up to date".into();
                        }
                    }
                    Err(error) => self.network_error(ctx, error),
                }
            }
            Msg::LoadDetails => {
                let Some(key) = ctx.state.config.route.clone() else {
                    return Update::none();
                };
                if ctx
                    .state
                    .detail_pending
                    .as_ref()
                    .is_some_and(|(k, e)| k == &key && *e == ctx.state.detail_epoch)
                    || ctx.elapsed() < ctx.state.blocked_until
                {
                    return Update::none();
                }
                ctx.state.next_details =
                    ctx.elapsed() + Duration::from_secs(ctx.state.config.detail_refresh_secs);
                let Some(api) = self.api.clone() else {
                    if ctx.state.details.is_none() {
                        ctx.state.details = Some(demo::details(&key));
                    }
                    ctx.link().send(Msg::LoadTraces);
                    return Update::full();
                };
                let epoch = ctx.state.detail_epoch;
                ctx.state.detail_pending = Some((key.clone(), epoch));
                return Update::with_command(ctx.link().command_keyed(
                    "detail",
                    TaskPolicy::LatestOnly,
                    move |link| {
                        let result = api.details(&key).map(Box::new).map_err(|e| e.to_string());
                        link.send_if_not_cancelled(Msg::DetailsLoaded(key, epoch, result));
                    },
                ));
            }
            Msg::DetailsLoaded(key, epoch, result) => {
                if ctx.state.config.route.as_ref() != Some(&key) || epoch != ctx.state.detail_epoch
                {
                    return Update::none();
                }
                ctx.state.detail_pending = None;
                match result {
                    Ok(details) => {
                        // Preserve the selected job/discussion identity as GitLab reorders collections.
                        let job_id = ctx
                            .state
                            .details
                            .as_ref()
                            .and_then(|d| d.jobs.get(ctx.state.config.field))
                            .map(|j| j.id);
                        let discussion_id = ctx
                            .state
                            .details
                            .as_ref()
                            .and_then(|d| d.discussions.get(ctx.state.config.field))
                            .map(|d| d.id.clone());
                        if let Some(item) = ctx.state.items.iter_mut().find(|i| i.key == key) {
                            *item = details.item.clone();
                        }
                        let warnings = details.warnings.clone();
                        ctx.state.details = Some(*details);
                        if ctx.state.section_name() == "Jobs" {
                            if let Some(index) = ctx
                                .state
                                .details
                                .as_ref()
                                .and_then(|d| d.jobs.iter().position(|j| Some(j.id) == job_id))
                            {
                                ctx.state.config.field = index;
                            }
                        } else if ctx.state.section_name() == "Discussions"
                            && let Some(index) = ctx.state.details.as_ref().and_then(|d| {
                                d.discussions
                                    .iter()
                                    .position(|d| Some(&d.id) == discussion_id.as_ref())
                            })
                        {
                            ctx.state.config.field = index;
                        }
                        for warning in warnings {
                            // Missing optional features must not stall otherwise healthy live logs.
                            if warning.contains("Retry-After:")
                                || warning.contains("HTTP 429")
                                || warning.contains("HTTP 503")
                            {
                                self.network_error(ctx, warning);
                            }
                        }
                        if ctx.state.log_focus.is_some_and(|id| {
                            !ctx.state
                                .details
                                .as_ref()
                                .is_some_and(|d| d.jobs.iter().any(|j| j.id == id))
                        }) {
                            self.close_log_mode(ctx);
                        }
                        ctx.state.status = "Detail updated".into();
                        ctx.link().send(Msg::LoadTraces);
                    }
                    Err(error) => self.network_error(ctx, error),
                }
            }
            Msg::LoadTraces => return self.load_traces(ctx),
            Msg::TraceLoaded(key, epoch, id, finished, result) => {
                ctx.state.trace_pending.remove(&(epoch, id));
                if ctx.state.config.route.as_ref() != Some(&key) || ctx.state.detail_epoch != epoch
                {
                    return Update::none();
                }
                match result {
                    Ok(chunk) => {
                        // If resizing/zooming enlarged the viewport near EOF, its
                        // displayed top may have been clamped. Anchor that actual
                        // source line before adding new output, not an old offset.
                        let height = self.log_height(ctx, id);
                        let total = ctx.state.traces.get(&id).map_or(0, Trace::line_count);
                        if let Some(view) = ctx.state.log_views.get_mut(&id).filter(|v| !v.follow) {
                            view.offset = view.position(total, height);
                        }
                        let trace = ctx.state.traces.entry(id).or_default();
                        trace.append(&chunk.text, chunk.reset);
                        let view = ctx.state.log_views.entry(id).or_default();
                        if chunk.reset {
                            view.reset();
                            view.refresh_matches(trace);
                        }
                        // Search is refreshed on navigation, not on every live append.
                        // Stable source-line positions keep paused viewports anchored.
                        let no_progress = !chunk.reset && chunk.next_offset == trace.offset;
                        trace.offset = chunk.next_offset;
                        // Read completed jobs to EOF rather than treating the first bounded chunk as complete.
                        trace.finished = finished && no_progress;
                        trace.error = None;
                    }
                    Err(error) => {
                        ctx.state.traces.entry(id).or_default().error = Some(error.clone());
                        self.network_error(ctx, error);
                    }
                }
            }
            Msg::ProjectResolved(result) => {
                ctx.state.mutation_pending = false;
                match result {
                    Ok(mut project) => {
                        if ctx.state.config.projects.iter().any(|p| p.id == project.id) {
                            self.dialog_error(ctx, "This project is already in the workspace");
                        } else {
                            project.alias = ctx
                                .state
                                .dialog
                                .as_ref()
                                .and_then(|d| d.fields.get(1))
                                .map_or("", FormField::value)
                                .trim()
                                .to_owned();
                            ctx.state.config.projects.push(project);
                            self.close_dialog(ctx);
                            self.persist(ctx);
                            ctx.link().send(Msg::Refresh);
                        }
                    }
                    Err(error) => self.dialog_error(ctx, &error),
                }
            }
            Msg::MutationDone(result) => {
                ctx.state.mutation_pending = false;
                match result {
                    Ok(()) => {
                        self.close_dialog(ctx);
                        ctx.state.status = "Change saved to GitLab".into();
                        // A list GET begun before the write must not roll its result back in the UI.
                        ctx.state.list_epoch += 1;
                        ctx.state.list_pending.clear();
                        ctx.state.detail_epoch += 1;
                        ctx.state.detail_pending = None;
                        ctx.link().send(Msg::Refresh);
                    }
                    Err(error) => {
                        self.dialog_error(ctx, &error);
                        self.network_error(ctx, error);
                    }
                }
            }
            Msg::Tab(index) => {
                if index >= ctx.state.tab_names().len()
                    || index == ctx.state.config.active_tab
                    || ctx.state.dialog.is_some()
                {
                    return Update::none();
                }
                self.switch_tab(ctx, index);
                self.persist(ctx);
            }
            Msg::Move(delta) => {
                if ctx.state.dialog.is_some() {
                    self.move_dialog(ctx, delta);
                } else {
                    self.move_selection(ctx, delta);
                    self.persist(ctx);
                }
            }
            Msg::Select(index) => {
                if index != ctx.state.config.field {
                    self.close_log_mode(ctx);
                }
                self.select(ctx, index);
                self.persist(ctx);
            }
            Msg::Activate(index) => {
                self.select(ctx, index);

                return self.update(Msg::Enter, ctx);
            }
            Msg::Enter => {
                if ctx.state.dialog.is_some() {
                    return self.submit(ctx);
                }
                match ctx.state.scope {
                    Scope::List => {
                        if ctx.state.config.active_tab == 1 {
                            if let Some(project) = ctx
                                .state
                                .config
                                .projects
                                .get(ctx.state.scroll.selected)
                                .cloned()
                            {
                                self.switch_tab(ctx, 2);
                                self.reset_detail(ctx);
                                ctx.state.config.filters.insert(
                                    "2".into(),
                                    format!("project:\"{}\"", project.path.replace('"', "")),
                                );
                                ctx.state.scroll = BoundaryScroll::default();
                            }
                        } else {
                            let key = ctx
                                .state
                                .visible_items()
                                .get(ctx.state.scroll.selected)
                                .map(|i| i.key.clone());
                            if let Some(key) = key {
                                self.open_item(ctx, key);
                            }
                        }
                    }
                    Scope::Details => {
                        ctx.state.reveal_content = true;
                        ctx.state.scope = Scope::Section;
                        ctx.state.config.section = Some(ctx.state.section_cursor);
                        ctx.state.config.field = 0;
                    }
                    Scope::Section => match ctx.state.section_name() {
                        "Fields" => self.edit_field(ctx),
                        "Description" => {
                            if let Some(d) = &ctx.state.details {
                                let key = d.item.key.clone();
                                let description = d.item.description.clone();
                                self.show_dialog(
                                    ctx,
                                    DialogKind::Edit(key, "description".into()),
                                    "Edit description",
                                    "Enter saves · Ctrl+J inserts a newline · Esc cancels",
                                    vec![FormField::new("Description", &description, true)],
                                );
                            }
                        }
                        "Jobs" => {
                            if let Some(job) = ctx
                                .state
                                .details
                                .as_ref()
                                .and_then(|d| d.jobs.get(ctx.state.config.field))
                            {
                                return self.update(Msg::FocusLog(job.id), ctx);
                            }
                        }
                        "Discussions" => return self.action(ctx, Action::Reply),
                        "Activity" => return self.action(ctx, Action::Comment),
                        _ => {}
                    },
                }
                self.persist(ctx);
            }
            Msg::Back => {
                if ctx.state.dialog.is_some() {
                    self.close_dialog(ctx);
                } else if ctx.state.log_search.take().is_some() {
                    // Escape closes search before zoom/focus or section navigation.
                } else if ctx.state.log_zoom {
                    ctx.state.log_zoom = false;
                    if !ctx.state.log_zoom_from_focus {
                        ctx.state.log_focus = None;
                    }
                } else if ctx.state.log_focus.take().is_some() {
                    // Return to job navigation without changing section or viewport.
                } else {
                    match ctx.state.scope {
                        Scope::Section => {
                            ctx.state.scope = Scope::Details;
                            ctx.state.config.section = None;
                            ctx.state.reveal_content = false;
                        }
                        Scope::Details => {
                            ctx.state.scope = Scope::List;
                            ctx.state.config.route = None;
                            ctx.state.config.section = None;
                            ctx.state.details = None;
                            ctx.state.detail_epoch += 1;
                            ctx.state.detail_pending = None;
                            ctx.state.traces.clear();
                            ctx.state.expanded.clear();
                            ctx.state.collapsed.clear();
                            ctx.state.log_views.clear();
                            ctx.state.content_offset = 0;
                        }
                        Scope::List => return Update::none(),
                    }
                    self.persist(ctx);
                }
            }
            Msg::Section(index) => {
                self.close_log_mode(ctx);
                if index < ctx.state.sections().len() {
                    ctx.state.reveal_content = true;
                    ctx.state.section_cursor = index;
                    ctx.state.scope = Scope::Section;
                    ctx.state.config.section = Some(index);
                    ctx.state.config.field = 0;
                    self.persist(ctx);
                }
            }
            Msg::DetailSection(index) => {
                self.close_log_mode(ctx);
                if index < ctx.state.sections().len() && ctx.state.config.route.is_some() {
                    ctx.state.section_cursor = index;
                    ctx.state.scope = Scope::Details;
                    ctx.state.config.section = None;
                    ctx.state.reveal_content = false;
                    self.persist(ctx);
                }
            }
            Msg::DetailScrolled(offset) => {
                ctx.state.detail_offset_request = None;
                ctx.state.reveal_content = false;
                ctx.state.content_offset = offset;
                self.persist(ctx);
            }
            Msg::ScrollContent(delta) => {
                ctx.state.detail_offset_request = None;
                ctx.state.reveal_content = false;
                ctx.state.content_offset = ctx
                    .state
                    .content_offset
                    .saturating_add_signed(delta)
                    .min(ctx.state.content_max_offset);
                self.persist(ctx);
            }
            Msg::Field(index) => {
                self.close_log_mode(ctx);
                ctx.state.scope = Scope::Section;
                ctx.state.section_cursor = 0;
                ctx.state.config.section = Some(0);
                ctx.state.reveal_content = false;
                ctx.state.config.field = index;
                self.persist(ctx);
                self.edit_field(ctx);
            }
            Msg::DetailViewport(event, target, origin, route, epoch) => {
                // Ignore callbacks from a previous route or a superseded keyboard target.
                if epoch != ctx.state.detail_epoch
                    || ctx.state.config.route.as_ref() != Some(&route)
                {
                    return Update::none();
                }
                ctx.state.content_max_offset = event.metrics.max_offset;
                if target
                    .as_ref()
                    .is_some_and(|key| *key != ctx.state.detail_target_key())
                {
                    // The measured rows are still useful when rapid navigation
                    // supersedes a reveal; only its offset request is stale.
                    ctx.state.detail_viewport = Some((route, *event));
                    return Update::none();
                }
                let navigation = target.is_some() && ctx.state.reveal_content;
                let offset = if navigation {
                    target
                        .as_deref()
                        .and_then(|key| super::detail_span(&event, key))
                        .map_or(event.offset, |(top, end)| {
                            crate::scroll::reveal_span(
                                origin,
                                top,
                                end,
                                event.metrics.len,
                                event.metrics.visible,
                            )
                        })
                } else {
                    event.offset
                };
                ctx.state.detail_offset_request =
                    (navigation && offset != event.offset).then_some(offset);
                ctx.state.detail_viewport = Some((route, *event));
                if navigation {
                    // One-shot navigation: wheel/drag, refresh and Escape own the
                    // viewport afterwards rather than snapping back to the target.
                    ctx.state.reveal_content = false;
                    ctx.state.content_offset = offset;
                    self.persist(ctx);
                } else {
                    return self.update(Msg::ContentScroll(offset), ctx);
                }
            }
            Msg::ContentScroll(offset) => {
                ctx.state.detail_offset_request = None;
                if ctx.state.content_offset != offset {
                    ctx.state.content_offset = offset;
                    if !self.persist(ctx) {
                        return Update::full();
                    }
                }
                return Update::none();
            }
            Msg::ListScroll(_) | Msg::ListViewportChanged => {
                if ctx.state.scope != Scope::List {
                    return Update::none();
                }
                let previous = ctx.state.scroll.clone();
                if let Msg::ListScroll(offset) = msg {
                    let len = if ctx.state.config.active_tab == 1 {
                        ctx.state.config.projects.len()
                    } else {
                        ctx.state.visible_items().len()
                    };
                    let row_height = list_row_height(ctx.state.config.active_tab);
                    let height = list_height_for_tab(ctx.viewport().h, ctx.state.config.active_tab);
                    ctx.state.scroll.scroll_to(offset / row_height, len, height);
                } else {
                    // Layout changes preserve the cursor; only mouse scrolling owns the viewport.
                    self.normalize(ctx);
                }
                if ctx.state.scroll == previous {
                    return Update::none();
                }
                self.persist(ctx);
            }
            Msg::ToggleProject => {
                if ctx.state.config.active_tab == 1
                    && let Some(project) =
                        ctx.state.config.projects.get_mut(ctx.state.scroll.selected)
                {
                    project.visible = !project.visible;
                    ctx.state.status = format!(
                        "{} {}",
                        project.name(),
                        if project.visible {
                            "included"
                        } else {
                            "hidden"
                        }
                    );
                    self.persist(ctx);
                    ctx.link().send(Msg::Refresh);
                }
            }
            Msg::ToggleJob(id) => {
                let Some(job) = ctx
                    .state
                    .details
                    .as_ref()
                    .and_then(|d| d.jobs.iter().find(|j| j.id == id))
                else {
                    return Update::none();
                };
                if ctx.state.job_expanded(job) {
                    ctx.state.expanded.remove(&id);
                    ctx.state.collapsed.insert(id);
                    if ctx.state.log_focus == Some(id) {
                        self.close_log_mode(ctx);
                    }
                } else {
                    ctx.state.collapsed.remove(&id);
                    ctx.state.expanded.insert(id);
                }
                self.persist(ctx);
                ctx.link().send(Msg::LoadTraces);
            }
            Msg::FocusLog(id) => {
                let Some(index) = ctx
                    .state
                    .details
                    .as_ref()
                    .and_then(|d| d.jobs.iter().position(|j| j.id == id))
                else {
                    return Update::none();
                };
                ctx.state.config.field = index;
                ctx.state.log_focus = Some(id);
                ctx.state.collapsed.remove(&id);
                ctx.state.expanded.insert(id);
                ctx.state.log_views.entry(id).or_default();
                ctx.state.reveal_content = true;
                self.persist(ctx);
                ctx.link().send(Msg::LoadTraces);
            }
            Msg::ZoomLog => {
                if ctx.state.log_zoom {
                    ctx.state.log_zoom = false;
                    if !ctx.state.log_zoom_from_focus {
                        ctx.state.log_focus = None;
                    }
                    ctx.state.log_search = None;
                } else if let Some(id) = ctx.state.selected_job().map(|j| j.id) {
                    ctx.state.log_zoom_from_focus = ctx.state.log_focus.is_some();
                    ctx.state.log_zoom = true;
                    ctx.state.log_focus = Some(id);
                    ctx.state.collapsed.remove(&id);
                    ctx.state.expanded.insert(id);
                    ctx.state.log_views.entry(id).or_default();
                    self.persist(ctx);
                    ctx.link().send(Msg::LoadTraces);
                }
            }
            Msg::ScrollLog(id, delta) => {
                let height = self.log_height(ctx, id);
                let total = ctx.state.traces.get(&id).map_or(0, Trace::line_count);
                ctx.state
                    .log_views
                    .entry(id)
                    .or_default()
                    .scroll(total, height, delta);
            }
            Msg::LogWheel(id, origin, delta) => {
                if !ctx.state.log_zoom || ctx.state.log_focus != Some(id) {
                    return Update::none();
                }
                let height = self.log_height(ctx, id);
                let total = ctx.state.traces.get(&id).map_or(0, Trace::line_count);
                let view = ctx.state.log_views.entry(id).or_default();
                if view.position(total, height) != origin || delta == 0 {
                    return Update::none();
                }
                view.scroll(total, height, delta);
                view.wheel_epoch = view.wheel_epoch.wrapping_add(1);
            }
            Msg::LogBar(id, row, height, start) => {
                if !ctx.state.details.as_ref().is_some_and(|d| {
                    d.jobs
                        .iter()
                        .any(|j| j.id == id && ctx.state.job_expanded(j))
                }) {
                    return Update::none();
                }
                let total = ctx.state.traces.get(&id).map_or(0, Trace::line_count);
                let view = ctx.state.log_views.entry(id).or_default();
                let (top, thumb) =
                    super::logs::scrollbar(total, height, view.position(total, height));
                if start {
                    let on_thumb = (top..top + thumb).contains(&row);
                    view.drag_grab = Some(if on_thumb { row - top } else { thumb / 2 });
                    if on_thumb {
                        // Quantizing a huge log to a short track must not move its
                        // viewport merely because the thumb was pressed.
                        return Update::none();
                    }
                }
                let grab = view.drag_grab.unwrap_or(thumb / 2);
                let travel = height.saturating_sub(thumb);
                let max = total.saturating_sub(height);
                view.offset = if travel == 0 {
                    0
                } else {
                    (row.saturating_sub(grab).min(travel) as u128 * max as u128 / travel as u128)
                        as usize
                };
                view.follow = view.offset == max;
            }
            Msg::LogBarEnd(id) => {
                if let Some(view) = ctx.state.log_views.get_mut(&id) {
                    view.drag_grab = None;
                }
                return Update::none();
            }
            Msg::SearchLog => {
                if ctx.state.log_focus.is_some() {
                    let query = ctx
                        .state
                        .log_focus
                        .and_then(|id| ctx.state.log_views.get(&id))
                        .map_or("", |v| v.query.as_str());
                    ctx.state.log_search = Some(TextInput::new(query));
                    ctx.request_focus("log-search");
                }
            }
            Msg::LogSearchInput(event) => {
                if let Some(input) = &mut ctx.state.log_search {
                    event.apply_to(input);
                }
            }
            Msg::SubmitLogSearch => {
                if let Some(input) = ctx.state.log_search.take()
                    && let Some(id) = ctx.state.log_focus
                {
                    let view = ctx.state.log_views.entry(id).or_default();
                    view.query = input.text().to_owned();
                    view.current_match = None;
                    return self.update(Msg::LogSearchNext(false), ctx);
                }
            }
            Msg::LogSearchNext(backwards) => {
                if let Some(id) = ctx.state.log_focus {
                    let height = self.log_height(ctx, id);
                    if let Some(trace) = ctx.state.traces.get(&id) {
                        ctx.state
                            .log_views
                            .entry(id)
                            .or_default()
                            .find(trace, height, backwards);
                    }
                }
            }
            Msg::Action(action) => return self.action(ctx, action),
            Msg::Palette => self.show_dialog(
                ctx,
                DialogKind::Commands,
                "Command palette",
                "Type to filter · ↑/↓ or Tab to choose · Enter to run",
                vec![FormField::new("Command", "", false)],
            ),
            Msg::CloseDialog => self.close_dialog(ctx),
            Msg::Submit => return self.submit(ctx),
            Msg::DialogScrolled => {
                if let Some(d) = &mut ctx.state.dialog {
                    if !d.reveal_selection {
                        return Update::none();
                    }
                    d.reveal_selection = false;
                }
            }
            Msg::DialogSelect(index) => {
                let themes = ctx
                    .state
                    .dialog
                    .as_ref()
                    .is_some_and(|dialog| matches!(dialog.kind, DialogKind::Themes));
                if let Some(d) = &mut ctx.state.dialog {
                    d.selected = index;
                    d.reveal_selection = true;
                }
                if themes {
                    self.preview_theme(ctx);
                }
            }
            Msg::DialogField(index) => {
                if let Some(d) = &mut ctx.state.dialog
                    && index < d.fields.len()
                {
                    if let Some(c) = d
                        .fields
                        .get_mut(d.selected)
                        .and_then(|f| f.completion.as_mut())
                    {
                        c.dismiss();
                    }
                    d.selected = index;
                    d.reveal_selection = true;
                    ctx.request_focus(format!("dialog-field-{index}"));
                }
            }

            Msg::LookupStart(epoch, index) => return self.start_lookup(ctx, epoch, index),
            Msg::LookupLoaded(epoch, index, result) => {
                return self.lookup_loaded(ctx, epoch, index, result);
            }
            Msg::LookupMove(index, delta) => {
                if let Some(c) = completion::active_completion_mut(&mut ctx.state, index) {
                    if !c.open {
                        return self.schedule_lookup(ctx, index);
                    }
                    c.selected = c
                        .selected
                        .saturating_add_signed(delta)
                        .min(c.options.len().saturating_sub(1));
                }
                if let Some(d) = &mut ctx.state.dialog {
                    d.reveal_selection = true;
                }
            }
            Msg::LookupAccept(index, epoch, selected) => {
                if let Some(d) = &mut ctx.state.dialog
                    && d.selected == index
                    && let Some(field) = d.fields.get_mut(index)
                {
                    let text = field.value().to_owned();
                    let cursor = field.cursor();
                    if let Some(c) = &mut field.completion
                        && c.epoch == epoch
                    {
                        if let Some((text, cursor)) = c.accept_text(&text, cursor, selected) {
                            field.set_text_and_cursor(text, cursor);
                        }
                        d.reveal_selection = true;
                        d.error = None;
                        ctx.request_focus(format!("dialog-field-{index}"));
                    }
                }
            }
            Msg::LookupEnter(index) => {
                if let Some(c) = completion::active_completion(&ctx.state, index)
                    && c.open
                {
                    if c.pending {
                        return Update::none();
                    }
                    if !c.options.is_empty() {
                        return self.update(Msg::LookupAccept(index, c.epoch, c.selected), ctx);
                    }
                }
                return self.submit(ctx);
            }
            Msg::Input(index, event) => {
                let mut lookup_changed = false;
                if let Some(d) = &mut ctx.state.dialog {
                    d.reveal_selection = true;
                    if let Some(f) = d.fields.get_mut(index) {
                        let previous = f.input.text().to_owned();
                        let previous_cursor = f.input.cursor();
                        event.apply_to(&mut f.input);
                        lookup_changed = f.completion.is_some()
                            && (previous != f.input.text() || previous_cursor != f.input.cursor());
                        if matches!(d.kind, DialogKind::Commands) && previous != f.input.text() {
                            d.selected = 0;
                        }
                    }
                    d.error = None;
                }
                if lookup_changed {
                    return self.schedule_lookup(ctx, index);
                }
            }
            Msg::Editor(index, event) => {
                let mut lookup_changed = false;
                if let Some(d) = &mut ctx.state.dialog {
                    d.reveal_selection = true;
                    if let Some(f) = d.fields.get_mut(index) {
                        let previous = f.editor.text().to_owned();
                        let previous_cursor = f.editor.cursor();
                        event.apply_to(&mut f.editor);
                        lookup_changed = f.completion.is_some()
                            && (previous != f.editor.text()
                                || previous_cursor != f.editor.cursor());
                    }
                    d.error = None;
                }
                if lookup_changed {
                    return self.schedule_lookup(ctx, index);
                }
            }
            Msg::Newline(index) => {
                if let Some(f) = ctx
                    .state
                    .dialog
                    .as_mut()
                    .and_then(|d| d.fields.get_mut(index))
                {
                    f.editor.insert_char('\n');
                }
            }
        }
        Update::full()
    }

    fn on_key(&mut self, key: KeyEvent, ctx: &mut Context<Self>) -> KeyUpdate {
        if key.is_with(KeyCode::Char('c'), KeyMods::CTRL) {
            return KeyUpdate::handled(self.action(ctx, Action::Quit));
        }
        if ctx.state.dialog.is_some() {
            let msg = match key.code {
                KeyCode::Esc => Some(Msg::CloseDialog),
                KeyCode::Enter => Some(Msg::Submit),
                KeyCode::Down | KeyCode::Tab if !key.mods.shift => Some(Msg::Move(1)),
                KeyCode::Up | KeyCode::BackTab | KeyCode::Tab => Some(Msg::Move(-1)),
                _ => None,
            };
            return match msg {
                Some(msg) => KeyUpdate::handled(self.update(msg, ctx)),
                None => KeyUpdate::unhandled(Update::none()),
            };
        }
        if ctx.state.log_search.is_some() {
            return match key.code {
                KeyCode::Esc => KeyUpdate::handled(self.update(Msg::Back, ctx)),
                KeyCode::Enter => KeyUpdate::handled(self.update(Msg::SubmitLogSearch, ctx)),
                _ => KeyUpdate::unhandled(Update::none()),
            };
        }
        if let Some(id) = ctx.state.selected_job().map(|j| j.id) {
            let focused = ctx.state.log_focus == Some(id);
            let ctrl_scroll = key.mods == KeyMods::CTRL;
            let plain = !key.mods.ctrl && !key.mods.alt && !key.mods.super_key;
            let height = self.log_height(ctx, id) as isize;
            let log_message = match key.code {
                KeyCode::Up if ctrl_scroll || (plain && focused) => Some(Msg::ScrollLog(id, -1)),
                KeyCode::Down if ctrl_scroll || (plain && focused) => Some(Msg::ScrollLog(id, 1)),
                KeyCode::PageUp if ctrl_scroll || (plain && focused) => {
                    Some(Msg::ScrollLog(id, -height))
                }
                KeyCode::PageDown if ctrl_scroll || (plain && focused) => {
                    Some(Msg::ScrollLog(id, height))
                }
                KeyCode::Home if plain && focused => Some(Msg::ScrollLog(id, isize::MIN)),
                KeyCode::End if plain && focused => Some(Msg::ScrollLog(id, isize::MAX)),
                KeyCode::Char('k') if plain && focused => Some(Msg::ScrollLog(id, -1)),
                KeyCode::Char('j') if plain && focused => Some(Msg::ScrollLog(id, 1)),
                KeyCode::Char(' ') if plain => Some(Msg::ToggleJob(id)),
                KeyCode::Char('z') if plain => Some(Msg::ZoomLog),
                KeyCode::Char('/') if plain && focused => Some(Msg::SearchLog),
                KeyCode::Char('n') if plain && focused => Some(Msg::LogSearchNext(key.mods.shift)),
                KeyCode::Char('N') if plain && focused => Some(Msg::LogSearchNext(true)),
                _ => None,
            };
            if let Some(msg) = log_message {
                return KeyUpdate::handled(self.update(msg, ctx));
            }
        }
        let ctrl = key.mods.ctrl;
        let msg = if key.is_with(KeyCode::Char('p'), KeyMods::CTRL) || key.is(KeyCode::Char(':')) {
            Some(Msg::Palette)
        } else if ctrl && key.code == KeyCode::Char('r') {
            Some(Msg::Refresh)
        } else if key.mods.alt || ctrl || key.mods.super_key {
            None
        } else if let Some(index) = tab_shortcut(key) {
            Some(Msg::Tab(index))
        } else {
            match key.code {
                KeyCode::Left if key.mods == KeyMods::NONE => {
                    ctx.state.config.active_tab.checked_sub(1).map(Msg::Tab)
                }
                KeyCode::Right if key.mods == KeyMods::NONE => {
                    let next = ctx.state.config.active_tab + 1;
                    (next < ctx.state.tab_names().len()).then_some(Msg::Tab(next))
                }
                KeyCode::Down | KeyCode::Char('j') => Some(Msg::Move(1)),
                KeyCode::Up | KeyCode::Char('k') => Some(Msg::Move(-1)),
                KeyCode::Tab => Some(Msg::Move(if key.mods.shift { -1 } else { 1 })),
                KeyCode::BackTab => Some(Msg::Move(-1)),

                KeyCode::Enter => Some(Msg::Enter),
                KeyCode::Esc => Some(Msg::Back),
                KeyCode::PageDown | KeyCode::PageUp if ctx.state.scope != Scope::List => {
                    let rows = ctx.viewport().h.saturating_sub(9).max(1) as isize;
                    Some(Msg::ScrollContent(if key.code == KeyCode::PageDown {
                        rows
                    } else {
                        -rows
                    }))
                }
                KeyCode::PageDown => Some(Msg::Move(list_height_for_tab(
                    ctx.viewport().h,
                    ctx.state.config.active_tab,
                ) as isize)),
                KeyCode::PageUp => Some(Msg::Move(
                    -(list_height_for_tab(ctx.viewport().h, ctx.state.config.active_tab) as isize),
                )),
                KeyCode::Home => Some(Msg::Move(isize::MIN)),
                KeyCode::End => Some(Msg::Move(isize::MAX)),
                KeyCode::Char(' ') => Some(Msg::ToggleProject),
                KeyCode::Char('/') => Some(Msg::Action(Action::Filter)),
                KeyCode::Char('s') => Some(Msg::Action(Action::SaveView)),
                KeyCode::Char('n') => Some(Msg::Action(
                    if ctx.state.kind() == Some(ItemKind::MergeRequest) {
                        Action::NewMergeRequest
                    } else {
                        Action::NewIssue
                    },
                )),
                KeyCode::Char('c') => Some(Msg::Action(Action::Comment)),
                KeyCode::Char('r') => Some(Msg::Action(
                    if ctx.state.scope == Scope::Section && ctx.state.section_name() == "Jobs" {
                        Action::RetryJob
                    } else {
                        Action::Refresh
                    },
                )),
                KeyCode::Char('R') => Some(Msg::Action(Action::Reply)),
                KeyCode::Char('x') => Some(Msg::Action(Action::Resolve)),
                KeyCode::Char('?') => Some(Msg::Action(Action::Help)),
                KeyCode::Char('q') => Some(Msg::Action(Action::Quit)),

                _ => None,
            }
        };
        match msg {
            Some(msg) => KeyUpdate::handled(self.update(msg, ctx)),
            None => KeyUpdate::unhandled(Update::none()),
        }
    }
}

impl Cronk {
    fn persist(&self, ctx: &mut Context<Self>) -> bool {
        let key = ctx.state.tab_key();
        ctx.state
            .config
            .selections
            .insert(key.clone(), ctx.state.scroll.selected);
        let tab = navigation(&ctx.state);
        ctx.state.config.tab_states.insert(key, tab);
        prune_tab_routes(&mut ctx.state.config);
        let config = &ctx.state.config;
        ctx.state.tab_cache.retain(|key, _| {
            config
                .tab_states
                .get(key)
                .is_some_and(|tab| tab.route.is_some())
        });
        if let Some(path) = &self.path
            && let Err(error) = ctx.state.config.save(path)
        {
            ctx.state.error = Some(format!("Workspace not saved: {error:#}"));
            return false;
        }
        true
    }

    fn normalize(&self, ctx: &mut Context<Self>) {
        let len = if ctx.state.config.active_tab == 1 {
            ctx.state.config.projects.len()
        } else {
            ctx.state.visible_items().len()
        };
        let height = list_height_for_tab(ctx.viewport().h, ctx.state.config.active_tab);
        ctx.state.scroll.normalize(len, height);
    }

    fn queue_lists(&self, ctx: &mut Context<Self>) {
        if !ctx.state.list_pending.is_empty() {
            return;
        }
        ctx.state.next_lists =
            ctx.elapsed() + Duration::from_secs(ctx.state.config.list_refresh_secs);
        if self.api.is_none() {
            return;
        }
        ctx.state.list_epoch += 1;
        let epoch = ctx.state.list_epoch;
        let ids: Vec<_> = ctx
            .state
            .config
            .projects
            .iter()
            .filter(|p| p.visible)
            .map(|p| p.id)
            .collect();
        ctx.state.list_pending.extend(&ids);
        ctx.state.status = if ids.is_empty() {
            "Add projects with the command palette (Ctrl+P)".into()
        } else {
            "Refreshing visible projects…".into()
        };
        for id in ids {
            ctx.link().send(Msg::LoadProject(id, epoch));
        }
    }

    pub(super) fn network_error(&self, ctx: &mut Context<Self>, error: String) {
        ctx.state.failures = (ctx.state.failures + 1).min(6);
        let seconds = 5 * (1u64 << ctx.state.failures);
        let retry_after = retry_after(&error, std::time::SystemTime::now());
        ctx.state.blocked_until = ctx.state.blocked_until.max(
            ctx.elapsed()
                .saturating_add(Duration::from_secs(seconds.max(retry_after))),
        );
        ctx.state.error = Some(error);
    }

    fn load_traces(&self, ctx: &mut Context<Self>) -> Update {
        ctx.state.next_traces = ctx.elapsed() + Duration::from_secs(2);
        let Some(details) = &ctx.state.details else {
            return Update::none();
        };
        if ctx.elapsed() < ctx.state.blocked_until {
            return Update::none();
        }
        let key = details.item.key.clone();
        let project = details.jobs_project.unwrap_or(key.project);
        let epoch = ctx.state.detail_epoch;
        let requests: Vec<_> = details
            .jobs
            .iter()
            .filter(|job| {
                (job.running()
                    || ctx.state.expanded.contains(&job.id)
                    || ctx.state.traces.contains_key(&job.id))
                    && !ctx.state.trace_pending.iter().any(|(_, id)| *id == job.id)
                    && !ctx
                        .state
                        .traces
                        .get(&job.id)
                        .is_some_and(|t| t.finished && !job.running())
            })
            .map(|j| {
                (
                    j.id,
                    ctx.state.traces.get(&j.id).map_or(0, |t| t.offset),
                    !j.running(),
                )
            })
            .collect();
        if requests.is_empty() {
            return Update::none();
        }
        if let Some(api) = self.api.clone() {
            ctx.state
                .trace_pending
                .extend(requests.iter().map(|(id, _, _)| (epoch, *id)));
            // Each job is independently delivered. A slow trace does not suppress already returned panels.
            let command = ctx.link().command(move |link| {
                // Bound connections and worker threads even for very wide enterprise pipelines.
                for batch in requests.chunks(4) {
                    std::thread::scope(|scope| {
                        for &(id, offset, finished) in batch {
                            let api = api.clone();
                            let link = link.clone();
                            let key = key.clone();
                            scope.spawn(move || {
                                link.send(Msg::TraceLoaded(
                                    key,
                                    epoch,
                                    id,
                                    finished,
                                    api.trace(project, id, offset).map_err(|e| e.to_string()),
                                ));
                            });
                        }
                    });
                }
            });
            Update::with_command(command)
        } else {
            for (id, _, finished) in requests {
                let trace = ctx.state.traces.entry(id).or_default();
                trace.append(&demo::trace(id, ctx.state.tick), true);
                trace.finished = finished;
            }
            Update::full()
        }
    }

    fn switch_tab(&self, ctx: &mut Context<Self>, index: usize) {
        if index == ctx.state.config.active_tab {
            return;
        }
        ctx.state.feedback.clear();
        let key = ctx.state.tab_key();
        let tab = navigation(&ctx.state);
        ctx.state.config.tab_states.insert(key.clone(), tab);
        ctx.state
            .config
            .selections
            .insert(key.clone(), ctx.state.scroll.selected);
        let cache = TabCache {
            details: ctx.state.details.take(),
            traces: std::mem::take(&mut ctx.state.traces),
            log_views: std::mem::take(&mut ctx.state.log_views),
            next_details: ctx.state.next_details,
            next_traces: ctx.state.next_traces,
        };
        ctx.state.tab_cache.insert(key, cache);
        ctx.state.config.active_tab = index;
        let key = ctx.state.tab_key();
        let selected = ctx.state.config.selections.get(&key).copied().unwrap_or(0);
        let tab = ctx
            .state
            .config
            .tab_states
            .get(&key)
            .cloned()
            .unwrap_or_else(|| TabState {
                list_offset: selected.saturating_sub(2),
                ..TabState::default()
            });
        let cache = ctx.state.tab_cache.remove(&key).unwrap_or_default();
        ctx.state.config.route = tab.route;
        ctx.state.config.section = tab.section;
        ctx.state.config.field = tab.field;
        ctx.state.scope = if ctx.state.config.route.is_none() {
            Scope::List
        } else if ctx.state.config.section.is_some() {
            Scope::Section
        } else {
            Scope::Details
        };
        // A response from a previous visit must not overwrite the restored cache.
        ctx.state.detail_epoch += 1;
        ctx.state.detail_pending = None;
        ctx.state.trace_pending.clear();
        ctx.state.details = cache.details;
        ctx.state.traces = cache.traces;
        ctx.state.log_views = cache.log_views;
        self.close_log_mode(ctx);
        ctx.state.next_details = cache.next_details;
        ctx.state.next_traces = cache.next_traces;
        ctx.state.expanded = tab.expanded.into_iter().collect();
        ctx.state.collapsed = tab.collapsed.into_iter().collect();
        ctx.state.content_offset = tab.content_offset;
        ctx.state.content_max_offset = usize::MAX;
        ctx.state.detail_viewport = None;
        ctx.state.detail_offset_request = Some(tab.content_offset);
        ctx.state.section_cursor = tab.section_cursor.min(ctx.state.sections().len() - 1);
        ctx.state.reveal_content = false;
        ctx.state.scroll = BoundaryScroll {
            selected,
            offset: tab.list_offset,
        };
        // Avoid discarding a restored cursor while the first live list is still loading.
        if self.api.is_none() || !ctx.state.items.is_empty() || index == 1 {
            self.normalize(ctx);
        }
        if ctx.state.config.route.is_some() {
            if ctx.state.details.is_none() || ctx.elapsed() >= ctx.state.next_details {
                ctx.link().send(Msg::LoadDetails);
            } else if ctx.elapsed() >= ctx.state.next_traces {
                ctx.link().send(Msg::LoadTraces);
            }
        }
    }

    fn close_log_mode(&self, ctx: &mut Context<Self>) {
        ctx.state.log_focus = None;
        ctx.state.log_zoom = false;
        ctx.state.log_zoom_from_focus = false;
        ctx.state.log_search = None;
    }

    fn log_height(&self, ctx: &Context<Self>, id: u64) -> usize {
        ctx.state.log_height(ctx.viewport().h, id)
    }

    fn reset_detail(&self, ctx: &mut Context<Self>) {
        ctx.state.config.route = None;
        ctx.state.config.section = None;
        ctx.state.config.field = 0;
        ctx.state.scope = Scope::List;
        ctx.state.details = None;
        ctx.state.detail_epoch += 1;
        ctx.state.detail_pending = None;
        ctx.state.trace_pending.clear();
        ctx.state.traces.clear();
        ctx.state.expanded.clear();
        ctx.state.collapsed.clear();
        ctx.state.log_views.clear();
        self.close_log_mode(ctx);
        ctx.state.section_cursor = 0;
        ctx.state.content_offset = 0;
    }

    fn open_item(&self, ctx: &mut Context<Self>, key: ItemKey) {
        ctx.state.detail_viewport = None;
        ctx.state.detail_offset_request = None;
        ctx.state.reveal_content = true;
        ctx.state.content_max_offset = usize::MAX;
        ctx.state.config.route = Some(key.clone());
        ctx.state.config.section = None;
        ctx.state.scope = Scope::Details;
        ctx.state.section_cursor = 0;
        ctx.state.config.field = 0;
        ctx.state.content_offset = 0;
        ctx.state.detail_epoch += 1;
        ctx.state.detail_pending = None;
        ctx.state.traces.clear();
        ctx.state.expanded.clear();
        ctx.state.collapsed.clear();
        ctx.state.log_views.clear();
        self.close_log_mode(ctx);
        ctx.state.details = if self.api.is_none() {
            Some(demo::details(&key))
        } else {
            None
        };
        ctx.link().send(Msg::LoadDetails);
    }

    fn move_selection(&self, ctx: &mut Context<Self>, delta: isize) {
        self.close_log_mode(ctx);
        ctx.state.detail_offset_request = None;
        ctx.state.reveal_content = true;
        match ctx.state.scope {
            Scope::List => {
                let len = if ctx.state.config.active_tab == 1 {
                    ctx.state.config.projects.len()
                } else {
                    ctx.state.visible_items().len()
                };
                let height = list_height_for_tab(ctx.viewport().h, ctx.state.config.active_tab);
                ctx.state.scroll.move_by(delta, len, height);
            }
            Scope::Details => {
                ctx.state.section_cursor = ctx
                    .state
                    .section_cursor
                    .saturating_add_signed(delta)
                    .min(ctx.state.sections().len() - 1);
            }
            Scope::Section => {
                let len = match ctx.state.section_name() {
                    "Fields" => ctx.state.fields().len(),
                    "Jobs" => ctx.state.details.as_ref().map_or(0, |d| d.jobs.len()),
                    "Discussions" => ctx
                        .state
                        .details
                        .as_ref()
                        .map_or(0, |d| d.discussions.len()),
                    _ => {
                        ctx.state.reveal_content = false;
                        ctx.state.content_offset = ctx
                            .state
                            .content_offset
                            .saturating_add_signed(delta)
                            .min(ctx.state.content_max_offset);
                        return;
                    }
                };
                ctx.state.config.field = ctx
                    .state
                    .config
                    .field
                    .saturating_add_signed(delta)
                    .min(len.saturating_sub(1));
            }
        }
    }

    fn select(&self, ctx: &mut Context<Self>, index: usize) {
        match ctx.state.scope {
            Scope::List => {
                let len = if ctx.state.config.active_tab == 1 {
                    ctx.state.config.projects.len()
                } else {
                    ctx.state.visible_items().len()
                };
                let height = list_height_for_tab(ctx.viewport().h, ctx.state.config.active_tab);
                ctx.state.scroll.select(index, len, height);
            }
            Scope::Details => ctx.state.section_cursor = index.min(ctx.state.sections().len() - 1),
            Scope::Section => {
                ctx.state.config.field = index;
                ctx.state.reveal_content = false;
            }
        }
    }

    fn show_dialog(
        &self,
        ctx: &mut Context<Self>,
        kind: DialogKind,
        title: &str,
        help: &str,
        fields: Vec<FormField>,
    ) {
        let (selected, original_theme) = if matches!(kind, DialogKind::Themes) {
            let original = ctx.state.config.theme.clone();
            let selected = ["midnight", "dracula", "light"]
                .iter()
                .position(|theme| *theme == original)
                .unwrap_or(0);
            (selected, Some(original))
        } else {
            (0, None)
        };
        ctx.state.feedback.clear();
        ctx.state.dialog = Some(Dialog {
            kind,
            title: title.into(),
            help: help.into(),
            fields,
            original_theme,
            selected,
            reveal_selection: true,
            error: None,
        });
        ctx.request_focus("dialog-field-0");
    }
    fn close_dialog(&self, ctx: &mut Context<Self>) {
        if ctx.state.mutation_pending {
            ctx.state.status =
                "Waiting for GitLab to confirm the request; the draft is retained".into();
            return;
        }
        if let Some(dialog) = ctx.state.dialog.take()
            && let Some(theme) = dialog.original_theme
        {
            ctx.state.config.theme = theme;
        }
        ctx.state.feedback.clear();
        ctx.blur();
    }
    fn dialog_error(&self, ctx: &mut Context<Self>, error: &str) {
        if let Some(d) = &mut ctx.state.dialog {
            d.error = Some(error.into());
        } else {
            ctx.state.error = Some(error.into());
        }
    }
    fn move_dialog(&self, ctx: &mut Context<Self>, delta: isize) {
        let count = if ctx
            .state
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Commands))
        {
            ctx.state.command_options().len()
        } else if ctx
            .state
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Themes))
        {
            3
        } else {
            ctx.state.dialog.as_ref().map_or(0, |d| d.fields.len())
        };
        let themes = ctx
            .state
            .dialog
            .as_ref()
            .is_some_and(|dialog| matches!(dialog.kind, DialogKind::Themes));
        if let Some(d) = &mut ctx.state.dialog {
            d.reveal_selection = true;
            d.selected = d
                .selected
                .saturating_add_signed(delta)
                .min(count.saturating_sub(1));
            if !matches!(d.kind, DialogKind::Commands | DialogKind::Themes) && count > 0 {
                let focus_key = format!("dialog-field-{}", d.selected);
                ctx.request_focus(focus_key);
            }
        }
        if themes {
            self.preview_theme(ctx);
        }
    }

    fn preview_theme(&self, ctx: &mut Context<Self>) {
        let Some(selected) = ctx.state.dialog.as_ref().map(|dialog| dialog.selected) else {
            return;
        };
        if let Some(theme) = ["midnight", "dracula", "light"].get(selected) {
            ctx.state.config.theme = (*theme).into();
        }
    }

    fn edit_field(&self, ctx: &mut Context<Self>) {
        let Some(key) = ctx.state.config.route.clone() else {
            return;
        };
        let fields = ctx.state.fields();
        let Some((label, name, value)) = fields.get(ctx.state.config.field) else {
            return;
        };
        let help = match *name {
            "state_event" => "Enter close or reopen. Enter commits · Esc cancels",
            "milestone_id" | "iteration_id" => {
                "Type a name · ↑/↓ suggestions · Enter chooses, then saves · Empty clears · Esc cancels"
            }
            "assignee_ids" | "reviewer_ids" => {
                "Type names separated by commas · ↑/↓ suggestions · Enter chooses, then saves · Empty clears"
            }
            "labels" => {
                "Type known labels separated by commas · ↑/↓ suggestions · Enter chooses, then saves · Empty clears"
            }
            _ => "Enter commits · Esc cancels",
        };
        let initial = if *name == "state_event" {
            if value == "opened" { "close" } else { "reopen" }
        } else {
            value
        };
        let mut field = FormField::new(label, initial, false);
        let lookup_kind = match *name {
            "labels" => Some(LookupKind::Labels),
            "assignee_ids" | "reviewer_ids" => Some(LookupKind::Users),
            "milestone_id" => Some(LookupKind::Milestones),
            "iteration_id" => Some(LookupKind::Iterations),
            _ => None,
        };
        if let Some(kind) = lookup_kind {
            let mut c = Completion::new(
                kind,
                key.project,
                matches!(kind, LookupKind::Users | LookupKind::Labels),
            );
            if kind == LookupKind::Iterations {
                c.current_iteration = ctx.state.current_iteration(key.project).cloned();
            }
            if let Some(details) = &ctx.state.details {
                let item = &details.item;
                match kind {
                    LookupKind::Labels => c.seed(item.labels.iter().map(LookupOption::label)),
                    LookupKind::Users => c.seed(
                        item.assignees
                            .iter()
                            .chain(&item.reviewers)
                            .map(LookupOption::user),
                    ),
                    LookupKind::Milestones | LookupKind::Iterations => {
                        let id = if kind == LookupKind::Milestones {
                            item.milestone_id
                        } else {
                            item.iteration_id
                        };
                        if let Some(id) = id {
                            c.resolved.insert(initial.trim().to_owned(), id.to_string());
                        }
                    }
                    LookupKind::Projects => {}
                }
            }
            field.completion = Some(c);
        }
        if *name == "labels" {
            field.editor.set_cursor(initial.len());
        }
        self.show_dialog(
            ctx,
            DialogKind::Edit(key, (*name).into()),
            &format!("Edit {label}"),
            help,
            vec![field],
        );
    }

    fn action(&mut self, ctx: &mut Context<Self>, action: Action) -> Update {
        if ctx.state.mutation_pending {
            return Update::none();
        }
        match action {
            Action::Refresh => { self.close_dialog(ctx); return self.update(Msg::Refresh, ctx); }
            Action::Quit => { if self.persist(ctx) { ctx.quit(); } }
            Action::Help => self.show_dialog(ctx, DialogKind::Help, "Keyboard guide", "Esc returns to your workspace", vec![]),
            Action::Themes => self.show_dialog(ctx, DialogKind::Themes, "Choose theme", "↑/↓ or Tab previews · Enter applies · Esc reverts · config colors remain overrides", vec![]),
            Action::Filter => {
                if ctx.state.scope != Scope::List || ctx.state.config.active_tab == 1 {
                    self.dialog_error(ctx, "Choose Dashboard, Issues, Merge Requests, or a saved list first");
                } else {
                    let query = ctx.state.query_text().to_owned();
                    self.show_dialog(ctx, DialogKind::Filter, "Filter view", "Space-separated terms are ANDed, including repeated fields: label:foo label:bar (both labels). No OR or grouping with brackets/parentheses. Quote values with spaces: label:\"needs review\" · prefix - to exclude: -label:blocked.", vec![FormField::new("Query", &query, false)]);
                }
            }
            Action::SaveView => {
                if ctx.state.kind().is_none() { self.dialog_error(ctx, "Save views from Issues or Merge Requests"); }
                else { self.show_dialog(ctx, DialogKind::SaveView, "Save as a tab", "The current filter becomes a persistent view", vec![FormField::new("Tab name", "", false)]); }
            }
            Action::RenameView | Action::DeleteView => {
                if let Some(index) = ctx.state.config.active_tab.checked_sub(4) {
                    if let Some(view) = ctx.state.config.views.get(index) {
                        let name = view.name.clone();
                        if matches!(action, Action::RenameView) {
                            self.show_dialog(ctx, DialogKind::RenameView, "Rename saved tab", "Enter saves · Esc cancels", vec![FormField::new("Tab name", &name, false)]);
                        } else { self.show_dialog(ctx, DialogKind::Confirm(Confirmation::DeleteView(index)), "Remove saved tab?", "Only the local view is removed, not GitLab data", vec![]); }
                    }
                } else { self.dialog_error(ctx, "The four built-in tabs cannot be renamed or removed"); }
            }
            Action::AddProject => self.show_dialog(ctx, DialogKind::AddProject, "Add existing project", "Search by name · Enter chooses a suggestion, then adds · Full paths and IDs also work", vec![FormField::lookup("Project", "", Completion::new(LookupKind::Projects, 0, false)), FormField::new("Short alias (optional)", "", false)]),
            Action::AliasProject | Action::RemoveProject => {
                if ctx.state.config.active_tab != 1 { self.dialog_error(ctx, "Select a project in the Projects tab first"); }
                else if let Some(project) = ctx.state.config.projects.get(ctx.state.scroll.selected) {
                    let id = project.id;
                    let name = project.name().to_owned();
                    if matches!(action, Action::AliasProject) { self.show_dialog(ctx, DialogKind::AliasProject(id), "Project alias", "Short local name; GitLab path is unchanged", vec![FormField::new("Alias", &name, false)]); }
                    else { self.show_dialog(ctx, DialogKind::Confirm(Confirmation::RemoveProject(id)), "Remove project from workspace?", &format!("{name} · GitLab project and issues are NOT deleted"), vec![]); }
                }
            }
            Action::NewIssue | Action::NewMergeRequest => {
                let project = ctx.state.config.route.as_ref().map(|k| k.project)
                    .or_else(|| if ctx.state.config.active_tab == 1 { ctx.state.config.projects.get(ctx.state.scroll.selected).map(|p| p.id) } else { ctx.state.visible_items().get(ctx.state.scroll.selected).map(|i| i.key.project) })
                    .or_else(|| ctx.state.config.projects.iter().find(|p| p.visible).map(|p| p.id));
                let Some(project) = project else { self.dialog_error(ctx, "Add a project first"); return Update::full(); };
                let project = ctx.state.project(project).map(|p| p.path.clone()).unwrap_or_else(|| project.to_string());
                let mut completion = Completion::new(LookupKind::Projects, 0, false);
                completion.workspace_only = true;
                completion.seed(ctx.state.config.projects.iter().map(LookupOption::project));
                let mut fields = vec![FormField::lookup("Project (workspace)", &project, completion), FormField::new("Title", "", false)];
                if matches!(action, Action::NewMergeRequest) {
                    fields.push(FormField::new("Source branch (must already exist)", "", false));
                    fields.push(FormField::new("Target branch", "main", false));
                }
                fields.push(FormField::new("Description", "", true));
                self.show_dialog(ctx, if matches!(action, Action::NewIssue) { DialogKind::NewIssue } else { DialogKind::NewMergeRequest }, if matches!(action, Action::NewIssue) { "Create issue" } else { "Create merge request" }, "Tab changes field · Ctrl+J newline · Enter creates on GitLab · Esc cancels", fields);
            }
            Action::Comment | Action::Reply | Action::Resolve => {
                let Some(key) = ctx.state.config.route.clone() else { self.dialog_error(ctx, "Open an issue or merge request first"); return Update::full(); };
                if matches!(action, Action::Comment) {
                    self.show_dialog(ctx, DialogKind::Comment(key), "Add comment", "Enter posts to GitLab · Ctrl+J newline · Esc cancels", vec![FormField::new("Comment", "", true)]);
                } else {
                    let discussion = if ctx.state.scope == Scope::Section && ctx.state.section_name() == "Discussions" {
                        ctx.state.details.as_ref().and_then(|d| d.discussions.get(ctx.state.config.field)).cloned()
                    } else { None };
                    if let Some(d) = discussion {
                        if matches!(action, Action::Reply) {
                            self.show_dialog(ctx, DialogKind::Reply(key, d.id), "Reply to discussion", "Enter posts · Ctrl+J newline · Esc cancels", vec![FormField::new("Reply", "", true)]);
                        } else if d.notes.iter().any(|n| n.resolvable) {
                            let resolved = d.notes.iter().any(|n| n.resolvable && !n.resolved);
                            self.show_dialog(ctx, DialogKind::Confirm(Confirmation::Mutate(Mutation::Resolve { key, discussion: d.id, resolved })), if resolved { "Resolve discussion?" } else { "Reopen discussion?" }, "This updates the discussion for everyone", vec![]);
                        } else { self.dialog_error(ctx, "This discussion cannot be resolved"); }
                    } else { self.dialog_error(ctx, "Enter Discussions and select a thread first"); }
                }
            }
            Action::RetryJob => {
                let job = if ctx.state.scope == Scope::Section && ctx.state.section_name() == "Jobs" {
                    ctx.state.details.as_ref().and_then(|d| d.jobs.get(ctx.state.config.field)).cloned()
                } else { None };
                if let Some(job) = job.filter(Job::retryable) {
                    let project = ctx.state.details.as_ref().and_then(|d| d.jobs_project)
                        .unwrap_or_else(|| ctx.state.config.route.as_ref().map_or(0, |k| k.project));
                    self.show_dialog(ctx, DialogKind::Confirm(Confirmation::Mutate(Mutation::RetryJob { project, job: job.id })), "Retry pipeline job?", &format!("{} · this consumes GitLab runner resources", job.name), vec![]);
                } else { self.dialog_error(ctx, "Select a finished, retryable job in the Jobs section"); }
            }
        }
        Update::full()
    }

    fn submit(&mut self, ctx: &mut Context<Self>) -> Update {
        if ctx.state.mutation_pending {
            return Update::none();
        }
        let Some(dialog) = &ctx.state.dialog else {
            return Update::none();
        };
        let kind = dialog.kind.clone();
        let values = dialog
            .fields
            .iter()
            .map(|f| {
                if matches!(kind, DialogKind::AddProject) {
                    Ok(f.value().to_owned())
                } else {
                    f.api_value()
                }
            })
            .collect::<Result<Vec<_>, _>>();
        let values = match values {
            Ok(values) => values,
            Err(error) => {
                self.dialog_error(ctx, &error);
                return Update::full();
            }
        };
        let selected = dialog.selected;
        let first = values.first().map_or("", String::as_str);
        let mutation = match kind {
            DialogKind::Commands => {
                if let Some((_, action)) = ctx.state.command_options().get(selected).copied() {
                    self.close_dialog(ctx);
                    return self.action(ctx, action);
                }
                return Update::none();
            }
            DialogKind::Themes => {
                if let Some(theme) = ["midnight", "dracula", "light"].get(selected) {
                    ctx.state.config.theme = (*theme).into();
                }
                if let Some(dialog) = &mut ctx.state.dialog {
                    dialog.original_theme = None;
                }
                self.close_dialog(ctx);
                self.persist(ctx);
                return Update::full();
            }
            DialogKind::Help => {
                self.close_dialog(ctx);
                return Update::full();
            }
            DialogKind::Filter => {
                if let Err(error) = Query::parse(first) {
                    self.dialog_error(ctx, &error);
                    return Update::full();
                }
                let key = ctx.state.tab_key();
                ctx.state.config.filters.insert(key, first.into());
                ctx.state.scroll = BoundaryScroll::default();
                self.close_dialog(ctx);
                self.persist(ctx);
                return Update::full();
            }
            DialogKind::SaveView | DialogKind::RenameView => {
                let name = first.trim();
                let rename_index = if matches!(kind, DialogKind::RenameView) {
                    ctx.state.config.active_tab.checked_sub(4)
                } else {
                    None
                };
                if name.is_empty()
                    || ctx
                        .state
                        .config
                        .views
                        .iter()
                        .enumerate()
                        .any(|(i, v)| Some(i) != rename_index && v.name == name)
                {
                    self.dialog_error(ctx, "Choose a nonempty, unique tab name");
                    return Update::full();
                }
                if let Some(index) = rename_index {
                    ctx.state.config.views[index].name = name.into();
                } else if let Some(kind) = ctx.state.kind() {
                    let query = ctx.state.query_text().to_owned();
                    ctx.state.config.views.push(SavedView {
                        name: name.into(),
                        kind,
                        query,
                    });
                    self.switch_tab(ctx, ctx.state.config.views.len() + 3);
                }
                self.close_dialog(ctx);
                self.persist(ctx);
                return Update::full();
            }
            DialogKind::AddProject => {
                if first.trim().is_empty() {
                    self.dialog_error(ctx, "Enter a project path or ID");
                    return Update::full();
                }
                let Some(api) = self.api.clone() else {
                    self.dialog_error(
                        ctx,
                        "Demo mode uses fictional projects; connect GitLab to add a real project",
                    );
                    return Update::full();
                };
                let path = first.trim().to_owned();
                ctx.state.mutation_pending = true;
                return Update::with_command(ctx.link().command(move |link| {
                    link.send(Msg::ProjectResolved(
                        api.resolve_project(&path).map_err(|e| e.to_string()),
                    ))
                }));
            }
            DialogKind::AliasProject(id) => {
                if let Some(project) = ctx.state.config.projects.iter_mut().find(|p| p.id == id) {
                    project.alias = first.trim().into();
                }
                self.close_dialog(ctx);
                self.persist(ctx);
                return Update::full();
            }
            DialogKind::Confirm(Confirmation::RemoveProject(id)) => {
                ctx.state.config.projects.retain(|p| p.id != id);
                ctx.state.items.retain(|i| i.key.project != id);
                self.normalize(ctx);
                self.close_dialog(ctx);
                self.persist(ctx);
                return Update::full();
            }
            DialogKind::Confirm(Confirmation::DeleteView(index)) => {
                if index < ctx.state.config.views.len() {
                    self.switch_tab(ctx, 0);
                    ctx.state.config.views.remove(index);
                    let removed = index + 4;
                    shift_tab_map(&mut ctx.state.config.filters, removed);
                    shift_tab_map(&mut ctx.state.config.selections, removed);
                    shift_tab_map(&mut ctx.state.config.tab_states, removed);
                    shift_tab_map(&mut ctx.state.tab_cache, removed);
                }
                self.close_dialog(ctx);
                self.persist(ctx);
                return Update::full();
            }
            DialogKind::Confirm(Confirmation::Mutate(mutation)) => mutation,
            DialogKind::Edit(key, field) => {
                if field == "title" && first.trim().is_empty() {
                    self.dialog_error(ctx, "Title cannot be empty");
                    return Update::full();
                }
                Mutation::Edit {
                    key,
                    field,
                    value: first.to_owned(),
                }
            }
            DialogKind::Comment(ref key) | DialogKind::Reply(ref key, _) => {
                let key = key.clone();
                if first.trim().is_empty() {
                    self.dialog_error(ctx, "Comment cannot be empty");
                    return Update::full();
                }
                if let DialogKind::Reply(_, discussion) = kind {
                    Mutation::Reply {
                        key,
                        discussion,
                        body: first.into(),
                    }
                } else {
                    Mutation::Comment {
                        key,
                        body: first.into(),
                    }
                }
            }
            DialogKind::NewIssue | DialogKind::NewMergeRequest => {
                let Some(project) = first
                    .parse::<u64>()
                    .ok()
                    .filter(|id| ctx.state.project(*id).is_some())
                else {
                    self.dialog_error(ctx, "Choose a project from your workspace suggestions");
                    return Update::full();
                };
                if values[1].trim().is_empty() {
                    self.dialog_error(ctx, "Title cannot be empty");
                    return Update::full();
                }
                if matches!(kind, DialogKind::NewIssue) {
                    Mutation::CreateIssue {
                        project,
                        title: values[1].clone(),
                        description: values[2].clone(),
                    }
                } else {
                    if values[2].trim().is_empty() || values[3].trim().is_empty() {
                        self.dialog_error(ctx, "Source and target branches are required");
                        return Update::full();
                    }
                    Mutation::CreateMergeRequest {
                        project,
                        title: values[1].clone(),
                        source: values[2].clone(),
                        target: values[3].clone(),
                        description: values[4].clone(),
                    }
                }
            }
        };
        let Some(api) = self.api.clone() else {
            self.dialog_error(ctx, "Demo is read-only: no request was sent. Local filters, views, visibility, and themes are editable.");
            return Update::full();
        };
        if ctx.elapsed() < ctx.state.blocked_until {
            self.dialog_error(
                ctx,
                "GitLab backoff is active. Your draft is retained; retry later.",
            );
            return Update::full();
        }
        ctx.state.mutation_pending = true;
        ctx.state.status = "Saving to GitLab…".into();
        Update::with_command(ctx.link().command(move |link| {
            link.send(Msg::MutationDone(
                api.mutate(mutation).map_err(|e| e.to_string()),
            ))
        }))
    }
}

fn navigation(state: &State) -> TabState {
    TabState {
        route: state.config.route.clone(),
        section: state.config.section,
        field: state.config.field,
        section_cursor: state.section_cursor,
        list_offset: state.scroll.offset,
        content_offset: state.content_offset,
        expanded: state.expanded.iter().copied().collect(),
        collapsed: state.collapsed.iter().copied().collect(),
    }
}

fn prune_tab_routes(config: &mut Config) {
    config.tab_states.retain(|key, tab| {
        if !key
            .parse::<usize>()
            .is_ok_and(|index| index < 4 + config.views.len())
        {
            return false;
        }
        if tab.route.as_ref().is_some_and(|route| {
            !config
                .projects
                .iter()
                .any(|p| p.id == route.project && p.visible)
        }) {
            *tab = TabState {
                list_offset: tab.list_offset,
                ..TabState::default()
            };
        }
        if let Some(route) = &tab.route {
            let last = if route.kind == ItemKind::MergeRequest {
                5
            } else {
                2
            };
            tab.section = tab.section.map(|section| section.min(last));
            tab.section_cursor = tab.section_cursor.min(last);
        }
        true
    });
}

fn tab_shortcut(key: KeyEvent) -> Option<usize> {
    // Traditional terminals send uppercase; enhanced protocols may send Shift+lowercase.
    match (key.code, key.mods.shift) {
        (KeyCode::Char('D'), _) | (KeyCode::Char('d'), true) => Some(0),
        (KeyCode::Char('P'), _) | (KeyCode::Char('p'), true) => Some(1),
        (KeyCode::Char('I'), _) | (KeyCode::Char('i'), true) => Some(2),
        (KeyCode::Char('M'), _) | (KeyCode::Char('m'), true) => Some(3),
        (KeyCode::Char(c @ '1'..='9'), false) => Some(4 + c as usize - '1' as usize),
        (KeyCode::Char('0'), false) => Some(13),
        _ => None,
    }
}

fn retry_after(error: &str, now: std::time::SystemTime) -> u64 {
    let Some(value) = error.split("Retry-After: ").nth(1) else {
        return 0;
    };
    let value = value.split(" (retry later").next().unwrap_or(value).trim();
    value
        .parse()
        .ok()
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()
                .map(|date| date.duration_since(now).unwrap_or_default().as_secs())
        })
        .unwrap_or(0)
}

fn shift_tab_map<T>(map: &mut std::collections::BTreeMap<String, T>, removed: usize) {
    *map = std::mem::take(map)
        .into_iter()
        .filter_map(|(key, value)| {
            let index = key.parse::<usize>().ok()?;
            if index == removed {
                None
            } else {
                Some((
                    (if index > removed { index - 1 } else { index }).to_string(),
                    value,
                ))
            }
        })
        .collect();
}
