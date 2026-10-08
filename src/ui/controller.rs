use super::*;
use crate::gitlab::Pipelines;
use crate::{
    config::{SavedView, TabState},
    demo,
};
use tui_lipan::CommandLink;

impl Component for Cronk {
    type Message = Msg;
    type Properties = ();
    type State = State;

    fn create_state(&self, _: &()) -> State {
        let mut config = self.config.clone();
        let mut navigation_writer = None;
        let mut navigation_snapshot = None;
        let mut navigation_error = None;
        let mut navigation_selections = config
            .selections
            .iter()
            .map(|(key, index)| (key.clone(), crate::navigation::Selection::Legacy(*index)))
            .collect();
        if let Some(path) = &self.path {
            match crate::navigation::Writer::open(path, &config, self.demo) {
                Ok((writer, snapshot, warning)) => {
                    if let Some(snapshot) = &snapshot {
                        navigation_selections = snapshot.restore(&mut config);
                    }
                    navigation_writer = Some(writer);
                    navigation_snapshot = snapshot;
                    navigation_error = warning;
                }
                Err(error) => {
                    navigation_error = Some(format!(
                        "Local navigation unavailable (using memory; legacy TOML retained): {error:#}"
                    ))
                }
            }
        }
        let navigation_restoring: HashSet<String> = config
            .tab_states
            .keys()
            .cloned()
            .chain(config.selections.keys().cloned())
            .collect();
        let user_formatter = UserFormatter::from_config(&config)
            .expect("configuration is validated before creating UI state");
        config.active_tab = config.active_tab.min(3 + config.views.len());
        if config.route.as_ref().is_some_and(|key| {
            !config
                .projects
                .iter()
                .any(|p| p.id == key.project && p.kind_visible(key.kind))
        }) {
            config.route = None;
            config.section = None;
        }
        if config
            .project_route
            .is_some_and(|id| !config.projects.iter().any(|p| p.id == id))
        {
            config.project_route = None;
            if config.active_tab == 1 {
                config.section = None;
            }
        }
        prune_tab_routes(&mut config);
        let saved = config
            .tab_states
            .get(&config.active_tab.to_string())
            .cloned();
        if let Some(tab) = &saved {
            config.route = tab.route.clone();
            config.project_route = tab.project_route;
            config.section = tab.section;
            config.field = tab.field;
        }
        let is_demo = self.demo;
        let open_details =
            config.route.is_some() || (config.active_tab == 1 && config.project_route.is_some());
        let scope = if open_details {
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
        let selected = config
            .selections
            .get(&config.active_tab.to_string())
            .copied()
            .unwrap_or(0);
        let section_last = if config.active_tab == 1 && config.project_route.is_some() {
            2
        } else if config
            .route
            .as_ref()
            .is_some_and(|k| k.kind == ItemKind::MergeRequest)
        {
            4
        } else {
            2
        };
        let section_cursor = saved
            .as_ref()
            .map_or(config.section.unwrap_or(0), |tab| tab.section_cursor)
            .min(section_last);
        let mut state = State {
            saved_config: Box::new(config.clone()),
            navigation_writer,
            navigation_snapshot,
            navigation_selections,
            navigation_hydrated: HashSet::new(),
            navigation_routes_restoring: navigation_restoring.clone(),
            navigation_restoring,
            navigation_reported_error: navigation_error.clone(),
            navigation_exit_failed: false,
            navigation_error: navigation_error.clone(),
            config,
            feedback: Default::default(),
            demo: is_demo,
            scope,
            items,
            starred_stubs: Vec::new(),
            user: if is_demo {
                demo::user()
            } else {
                User::default()
            },
            user_formatter,
            details,
            shared_details: HashMap::new(),
            detail_requests: HashMap::new(),
            detail_schema_errors: HashMap::new(),
            detail_retry_requested: HashSet::new(),
            detail_sequence: 0,
            sync_progress: BTreeMap::new(),
            full_resync: false,
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
            project_pipelines: HashMap::new(),
            latest_pipeline: HashMap::new(),
            pipeline_expanded: HashSet::new(),
            pipeline_collapsed: HashSet::new(),
            drilled: None,
            pipeline_epoch: 0,
            pipelines_pending: HashSet::new(),
            pipeline_detail_pending: HashSet::new(),
            probe_pending: HashSet::new(),
            pipelines_unavailable: HashMap::new(),
            next_pipelines: Duration::ZERO,
            next_probe: Duration::ZERO,
            log_focus: None,
            log_zoom: false,
            log_zoom_from_focus: false,
            log_search: None,
            dialog: None,
            onboarding: Default::default(),
            current_iterations: HashMap::new(),
            status: if is_demo {
                "Demo workspace · no requests or remote writes".into()
            } else {
                "Connecting to GitLab…".into()
            },
            error: navigation_error,
            list_pending: HashSet::new(),
            detail_pending: None,
            trace_pending: HashSet::new(),
            mutation_pending: false,
            user_pending: false,
            user_schema_error: false,
            user_retry_requested: false,
            user_epoch: 0,
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
        };
        state.reset_current_iterations();
        state.refresh_starred_stubs();
        state.remember_selection();
        if is_demo || state.config.active_tab == 1 {
            state.restore_selection(true);
        }
        state
    }

    fn unmount(&mut self, ctx: &mut Context<Self>) {
        if let Err(error) = ctx.state.flush_navigation() {
            eprintln!("{error:#}");
        }
    }

    fn init(&mut self, ctx: &mut Context<Self>) -> Option<Command> {
        self.enable_cache(ctx);
        // Capture resolved legacy identities without changing portable TOML.
        self.persist(ctx);
        ctx.link().send(Msg::Tick);
        ctx.link().send(Msg::Refresh);
        if ctx.state.config.onboarding {
            self.show_onboarding(ctx);
            return Some(self.schedule_setup(ctx));
        }
        None
    }

    fn view(&self, ctx: &Context<Self>) -> Element {
        view::view(ctx)
    }

    fn update(&mut self, msg: Msg, ctx: &mut Context<Self>) -> Update {
        if ctx
            .state
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Onboarding))
            && matches!(
                &msg,
                Msg::Refresh | Msg::LoadUser | Msg::LoadDetails | Msg::LoadTraces
            )
        {
            return Update::none();
        }
        if ctx.state.scope == Scope::List
            && ctx.state.dialog.is_none()
            && matches!(
                &msg,
                Msg::Move(_) | Msg::Select(_) | Msg::Activate(_) | Msg::ListScroll(_)
            )
        {
            // Section/field movement and dialog input do not own the list's
            // pending identity. Explicit list navigation does.
            let key = ctx.state.tab_key();
            ctx.state.navigation_restoring.remove(&key);
            ctx.state.navigation_routes_restoring.remove(&key);
        }
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
                let error = ctx
                    .state
                    .navigation_writer
                    .as_ref()
                    .and_then(|writer| writer.error())
                    .or_else(|| ctx.state.navigation_error.clone());
                if ctx.state.error == ctx.state.navigation_reported_error || error.is_some() {
                    ctx.state.error = error.clone();
                }
                ctx.state.navigation_reported_error = error;
                ctx.state.tick += 1;
                let now = ctx.elapsed();
                if now >= ctx.state.blocked_until
                    && !ctx
                        .state
                        .dialog
                        .as_ref()
                        .is_some_and(|d| matches!(d.kind, DialogKind::Onboarding))
                {
                    if ctx.state.user.id == 0
                        && !ctx.state.user_pending
                        && !ctx.state.user_schema_error
                    {
                        ctx.link().send(Msg::LoadUser);
                    }
                    if now >= ctx.state.next_lists {
                        self.queue_lists(ctx);
                    }
                    if ctx.state.config.route.is_some() && now >= ctx.state.next_details {
                        ctx.link().send(Msg::LoadDetails);
                    }
                    if ctx.state.job_source().is_some() && now >= ctx.state.next_traces {
                        ctx.link().send(Msg::LoadTraces);
                    }
                    if ctx.state.pipelines_section_present() && now >= ctx.state.next_pipelines {
                        ctx.link().send(Msg::LoadPipelines);
                    }
                    if now >= ctx.state.next_probe {
                        ctx.link().send(Msg::LoadProbes);
                    }
                }
                return Update::with_command(Command::after(
                    Duration::from_secs(1),
                    |link: CommandLink<Msg>| link.send(Msg::Tick),
                ));
            }
            Msg::FullResync => {
                if !ctx.state.list_pending.is_empty() {
                    ctx.state.status =
                        "A sync is running; request full resync after it finishes".into();
                    return Update::full();
                }
                ctx.state.full_resync = true;
                self.retry_detail_schemas(ctx, None);
                return self.update(Msg::Refresh, ctx);
            }
            Msg::ClearCache => {
                // Do not let an in-flight read immediately repopulate a cleared cache.
                if !ctx.state.list_pending.is_empty()
                    || !ctx.state.detail_requests.is_empty()
                    || ctx.state.mutation_pending
                {
                    ctx.state.error = Some(
                        "Wait for current requests to finish before clearing the cache".into(),
                    );
                    return Update::full();
                }
                if let Some(api) = &self.api {
                    match api.clear_content_cache() {
                        Ok(()) => {
                            ctx.state.shared_details.clear();
                            ctx.state.detail_schema_errors.clear();
                            ctx.state.detail_retry_requested.clear();
                            ctx.state.tab_cache.clear();
                            // Content eviction is not a navigation reset. Retain
                            // routes, cursor identities, offsets and job choices.
                            ctx.state.remember_selection();
                            ctx.state
                                .navigation_restoring
                                .extend(ctx.state.navigation_selections.keys().cloned());
                            ctx.state
                                .navigation_routes_restoring
                                .extend(ctx.state.config.tab_states.keys().cloned());
                            ctx.state.navigation_hydrated.clear();
                            ctx.state.items.clear();
                            self.clear_detail_content(ctx);
                            ctx.state.next_lists = Duration::MAX;
                            ctx.state.next_details = Duration::MAX;
                            ctx.state.next_traces = Duration::MAX;
                            ctx.state.current_iterations.clear();
                            ctx.state.status =
                                "Local content cache cleared; refresh to import again".into();
                        }
                        Err(error) => {
                            ctx.state.error = Some(format!("Cache not cleared: {error:#}"))
                        }
                    }
                }
            }
            Msg::CacheWarning(key, request, error) => {
                if ctx.state.detail_requests.get(&key) == Some(&request) {
                    ctx.state.error = Some(error);
                }
            }
            Msg::ProjectCached(id, epoch, result) => {
                if epoch != ctx.state.list_epoch
                    || !ctx
                        .state
                        .project(id)
                        .is_some_and(|p| p.effectively_visible())
                {
                    return Update::none();
                }
                match result {
                    Ok(items) => {
                        self.merge_items(ctx, items);
                        ctx.state.status =
                            "Showing cached content · refreshing GitLab changes…".into();
                    }
                    Err(error) => ctx.state.error = Some(format!("Cache read failed: {error}")),
                }
            }
            Msg::ProjectProgress(id, epoch, progress, items) => {
                if epoch != ctx.state.list_epoch
                    || !ctx
                        .state
                        .project(id)
                        .is_some_and(|p| p.kind_visible(progress.kind))
                {
                    return Update::none();
                }
                ctx.state.sync_progress.insert(id, progress);
                self.merge_items(ctx, items);
            }
            Msg::DetailCached(key, request, details) => {
                if ctx.state.detail_requests.get(&key) != Some(&request) {
                    return Update::none();
                }
                ctx.state
                    .shared_details
                    .insert(key.clone(), ((*details).clone(), Duration::ZERO));
                if ctx.state.config.route.as_ref() == Some(&key) {
                    // Historical warnings are visible, but aren't fresh rate-limit
                    // responses and must not restart backoff on startup.
                    ctx.state.details = Some(*details);
                    ctx.state.status = "Showing cached detail · refreshing GitLab…".into();
                }
            }
            Msg::DetailProgress(key, request, part, details) => {
                if ctx.state.detail_requests.get(&key) != Some(&request) {
                    return Update::none();
                }
                ctx.state
                    .shared_details
                    .insert(key.clone(), ((*details).clone(), Duration::ZERO));
                if ctx.state.config.route.as_ref() == Some(&key) {
                    let epoch = ctx.state.detail_epoch;
                    self.update(Msg::DetailsLoaded(key.clone(), epoch, Ok(details)), ctx);
                    ctx.state.detail_pending = Some((key, epoch));
                    ctx.state.status =
                        format!("Detail: {part:?} loaded · other sections refreshing…");
                }
            }
            Msg::DetailFinished(key, request, result) => {
                if ctx.state.detail_requests.get(&key) != Some(&request) {
                    return Update::none();
                }
                ctx.state.detail_requests.remove(&key);
                // Refresh during an in-flight load queues one replacement, not
                // parallel reads or suppression from the pre-refresh response.
                if ctx.state.detail_retry_requested.remove(&key) {
                    // A refresh makes schema failures obsolete, not server-wide
                    // rate limits or transport failures. Keep the retry due, but
                    // let the normal loader wait for network backoff to expire.
                    if let Err(error) = result
                        && !error.contains("invalid JSON/schema")
                    {
                        self.network_error(ctx, error);
                    }
                    if ctx.state.config.route.as_ref() == Some(&key) {
                        ctx.state.detail_pending = None;
                        ctx.state.next_details = Duration::ZERO;
                        ctx.link().send(Msg::LoadDetails);
                    }
                    return Update::full();
                }
                if let Err(error) = &result
                    && error.contains("invalid JSON/schema")
                {
                    ctx.state
                        .detail_schema_errors
                        .insert(key.clone(), error.clone());
                }
                if let Ok(details) = &result {
                    ctx.state.detail_schema_errors.remove(&key);
                    ctx.state.shared_details.insert(
                        key.clone(),
                        (
                            (**details).clone(),
                            ctx.elapsed()
                                + Duration::from_secs(ctx.state.config.detail_refresh_secs),
                        ),
                    );
                }
                if ctx.state.config.route.as_ref() == Some(&key) {
                    ctx.state.next_details = if ctx.state.detail_schema_errors.contains_key(&key) {
                        Duration::MAX
                    } else {
                        ctx.elapsed() + Duration::from_secs(ctx.state.config.detail_refresh_secs)
                    };
                    return self
                        .update(Msg::DetailsLoaded(key, ctx.state.detail_epoch, result), ctx);
                } else if let Err(error) = result {
                    self.network_error(ctx, error);
                }
            }
            Msg::Refresh => {
                if ctx.elapsed() < ctx.state.blocked_until {
                    ctx.state.status =
                        "GitLab backoff is active; refresh will resume automatically".into();
                } else {
                    ctx.state.error = ctx.state.navigation_error.clone();
                    ctx.state.user_schema_error = false;
                    ctx.state.user_retry_requested |= ctx.state.user_pending;
                    if ctx.state.user.id == 0 {
                        ctx.link().send(Msg::LoadUser);
                    }
                    let key = ctx.state.config.route.clone();
                    self.retry_detail_schemas(ctx, key.as_ref());
                    self.queue_lists(ctx);
                    ctx.link().send(Msg::LoadDetails);
                    ctx.link().send(Msg::LoadTraces);
                    // Explicit refresh is the way back in after a per-project 403/404.
                    ctx.state.pipelines_unavailable.clear();
                    ctx.state.next_pipelines = Duration::ZERO;
                    ctx.state.next_probe = Duration::ZERO;
                    if ctx.state.pipelines_section_present() {
                        ctx.link().send(Msg::LoadPipelines);
                    }
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
                let Some(project) = ctx
                    .state
                    .project(id)
                    .filter(|p| p.effectively_visible())
                    .cloned()
                else {
                    ctx.state.list_pending.remove(&id);
                    return Update::none();
                };
                let Some(api) = self.api.clone() else {
                    return Update::none();
                };
                let hydrate = !ctx.state.items.iter().any(|item| item.key.project == id);
                let full = ctx.state.full_resync;
                return Update::with_command(ctx.link().command(move |link| {
                    if hydrate {
                        link.send(Msg::ProjectCached(
                            id,
                            epoch,
                            api.cached_items(id).map_err(|e| e.to_string()),
                        ));
                    }
                    let result = api
                        .sync_project(&project, full, |progress, items| {
                            link.send(Msg::ProjectProgress(id, epoch, progress, items));
                        })
                        .map(|items| {
                            let iteration = api.current_iteration_for_sync(&project).ok().flatten();
                            (items, iteration)
                        })
                        .map_err(|e| e.to_string());
                    link.send(Msg::ProjectLoaded(id, epoch, result));
                }));
            }
            Msg::LoadUser => {
                if ctx.state.user_pending
                    || ctx.state.user_schema_error
                    || ctx.elapsed() < ctx.state.blocked_until
                {
                    return Update::none();
                }
                let Some(api) = self.api.clone() else {
                    return Update::none();
                };
                ctx.state.user_pending = true;
                let epoch = ctx.state.user_epoch;
                return Update::with_command(ctx.link().command(move |link| {
                    link.send(Msg::UserLoaded(
                        epoch,
                        api.current_user().map_err(|e| e.to_string()),
                    ));
                }));
            }
            Msg::UserLoaded(epoch, result) => {
                if epoch == ctx.state.user_epoch {
                    self.apply_user(ctx, result);
                }
            }
            Msg::ProjectLoaded(id, epoch, result) => {
                if epoch != ctx.state.list_epoch {
                    return Update::none();
                }
                ctx.state.list_pending.remove(&id);
                ctx.state.sync_progress.remove(&id);
                if ctx.state.list_pending.is_empty() {
                    ctx.state.full_resync = false;
                }
                if !ctx
                    .state
                    .project(id)
                    .is_some_and(|p| p.effectively_visible())
                {
                    return Update::none();
                }
                let selected = ctx
                    .state
                    .visible_item_at(ctx.state.scroll.selected)
                    .map(|i| i.key.clone());
                match result {
                    Ok((items, current_iteration)) => {
                        ctx.state.navigation_hydrated.insert(id);
                        let replaced: HashSet<_> = items.iter().map(|i| i.key.clone()).collect();
                        let project = ctx
                            .state
                            .project(id)
                            .expect("project checked above")
                            .clone();
                        ctx.state.items.retain(|i| {
                            i.key.project != id
                                || (!project.kind_visible(i.key.kind) && !replaced.contains(&i.key))
                        });
                        ctx.state.items.extend(items);
                        ctx.state.current_iterations.insert(id, current_iteration);
                        if let Some(key) = selected
                            && let Some(index) = ctx.state.visible_position(&key)
                        {
                            ctx.state.scroll.selected = index;
                        }
                        ctx.state
                            .restore_selection(ctx.state.navigation_hydration_complete());
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
                if let Some(error) = ctx.state.detail_schema_errors.get(&key) {
                    ctx.state.error = Some(format!("{error} · Refresh to retry this item"));
                    ctx.state.next_details = Duration::MAX;
                    return Update::full();
                }
                if ctx.state.detail_requests.contains_key(&key) {
                    ctx.state.detail_pending = Some((key, ctx.state.detail_epoch));
                    return Update::full();
                }
                if let Some((details, due)) = ctx.state.shared_details.get(&key).cloned() {
                    ctx.state.details = Some(details);
                    if ctx.state.detail_requests.contains_key(&key) {
                        ctx.state.detail_pending = Some((key, ctx.state.detail_epoch));
                        return Update::full();
                    }
                    if ctx.elapsed() < due {
                        ctx.state.next_details = due;
                        return Update::full();
                    }
                }
                ctx.state.next_details =
                    ctx.elapsed() + Duration::from_secs(ctx.state.config.detail_refresh_secs);
                let Some(api) = self.api.clone() else {
                    if ctx.state.details.is_none() {
                        ctx.state.details = ctx.state.demo.then(|| demo::details(&key));
                    }
                    ctx.link().send(Msg::LoadTraces);
                    return Update::full();
                };
                if ctx.state.details.as_ref().is_none_or(|d| d.item.key != key) {
                    ctx.state.details = Some(Details {
                        item: ctx
                            .state
                            .items
                            .iter()
                            .find(|i| i.key == key)
                            .cloned()
                            .unwrap_or_else(|| WorkItem {
                                key: key.clone(),
                                title: "Loading item…".into(),
                                ..Default::default()
                            }),
                        ..Default::default()
                    });
                }
                let epoch = ctx.state.detail_epoch;
                ctx.state.detail_pending = Some((key.clone(), epoch));
                ctx.state.detail_sequence += 1;
                let request = ctx.state.detail_sequence;
                ctx.state.detail_requests.insert(key.clone(), request);
                let previous = ctx.state.details.clone().filter(|d| d.item.key == key);
                return Update::with_command(ctx.link().command(move |link| {
                    let previous = match api.cached_details(&key) {
                        Ok(cached) => match previous {
                            Some(memory)
                                if memory.loaded.iter().any(|part| *part != DetailPart::Core) =>
                            {
                                Some(memory)
                            }
                            memory => cached.or(memory),
                        },
                        Err(error) => {
                            link.send(Msg::CacheWarning(
                                key.clone(),
                                request,
                                format!(
                                    "Detail cache read failed; fetching GitLab instead: {error:#}"
                                ),
                            ));
                            previous
                        }
                    };
                    if let Some(cached) = &previous {
                        link.send(Msg::DetailCached(
                            key.clone(),
                            request,
                            Box::new(cached.clone()),
                        ));
                    }
                    let result = api
                        .details_progress(&key, previous, |part, details| {
                            link.send(Msg::DetailProgress(
                                key.clone(),
                                request,
                                part,
                                Box::new(details),
                            ));
                        })
                        .map(Box::new)
                        .map_err(|e| e.to_string());
                    link.send(Msg::DetailFinished(key, request, result));
                }));
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
                        let previous_warnings = ctx
                            .state
                            .details
                            .as_ref()
                            .map_or_else(Vec::new, |d| d.warnings.clone());
                        let warnings = details.warnings.clone();
                        let due = if ctx.state.detail_requests.contains_key(&key) {
                            Duration::ZERO
                        } else {
                            ctx.state.next_details
                        };
                        ctx.state
                            .shared_details
                            .insert(key.clone(), ((*details).clone(), due));
                        ctx.state.details = Some(*details);
                        if ctx.state.section_name() == "Pipeline" {
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
                            if previous_warnings.contains(&warning) {
                                continue;
                            }
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
                        ctx.state.remember_selection();
                        ctx.state.status = "Detail updated".into();
                        ctx.link().send(Msg::LoadTraces);
                    }
                    Err(error) => {
                        if error.contains("invalid JSON/schema") {
                            ctx.state.detail_schema_errors.insert(key, error.clone());
                            ctx.state.next_details = Duration::MAX;
                            self.network_error(
                                ctx,
                                format!("{error} · Refresh to retry this item"),
                            );
                        } else {
                            self.network_error(ctx, error);
                        }
                    }
                }
            }
            Msg::LoadTraces => return self.load_traces(ctx),
            Msg::TraceLoaded(source, epoch, id, finished, result) => {
                ctx.state.trace_pending.remove(&(epoch, id));
                if ctx.state.job_source().as_ref() != Some(&source)
                    || ctx.state.detail_epoch != epoch
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
            Msg::LoadPipelines => return self.load_pipelines(ctx, false),
            Msg::LoadOlderPipelines => return self.load_pipelines(ctx, true),
            Msg::PipelinesLoaded(project, epoch, page, result) => {
                return self.pipelines_loaded(ctx, project, epoch, page, result);
            }
            Msg::PipelineLoaded(project, pipeline, epoch, result) => {
                return self.pipeline_loaded(ctx, project, pipeline, epoch, result);
            }
            Msg::TogglePipeline(id) => return self.toggle_pipeline(ctx, id),
            Msg::ClickPipeline(id) => return self.click_pipeline(ctx, id),
            Msg::DrillPipeline(id) => return self.drill_pipeline(ctx, id),
            Msg::ProbeLoaded(project, epoch, result) => {
                return self.probe_loaded(ctx, project, epoch, result);
            }
            Msg::LoadProbes => return self.load_probes(ctx),
            Msg::SetupValidate(epoch) => return self.validate_setup(ctx, epoch),
            Msg::SetupValidated(epoch, validation) => {
                if epoch == ctx.state.onboarding.epoch
                    && ctx
                        .state
                        .dialog
                        .as_ref()
                        .is_some_and(|d| matches!(d.kind, DialogKind::Onboarding))
                {
                    ctx.state.onboarding.feedback = validation.feedback.clone();
                    ctx.state.onboarding.validated = Some(*validation);
                }
            }
            Msg::ProjectResolved(result) => {
                ctx.state.mutation_pending = false;
                match result {
                    Ok(mut project) => {
                        if ctx.state.config.projects.iter().any(|p| p.id == project.id) {
                            self.dialog_error(ctx, "This project is already in the workspace");
                        } else {
                            let custom_alias = ctx
                                .state
                                .dialog
                                .as_ref()
                                .and_then(|d| d.fields.get(1))
                                .map_or("", FormField::value)
                                .trim();
                            if !custom_alias.is_empty() {
                                project.alias = custom_alias.to_owned();
                            }
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
                        ctx.state.sync_progress.clear();
                        ctx.state.shared_details.clear();
                        ctx.state.detail_requests.clear();
                        ctx.state.detail_schema_errors.clear();
                        ctx.state.detail_retry_requested.clear();
                        ctx.state.tab_cache.clear();
                        ctx.state.detail_epoch += 1;
                        ctx.state.detail_pending = None;
                        ctx.link().send(Msg::Refresh);
                    }
                    Err(error) => {
                        // The dialog keeps the full explanation (request and GitLab's response)
                        // until the user dismisses it; the status line gets the summary only.
                        self.dialog_error(ctx, &error);
                        let summary = error.lines().next().unwrap_or_default().to_owned();
                        self.network_error(ctx, summary);
                    }
                }
            }
            Msg::Tab(index) => {
                if index >= ctx.state.config.tab_count() || ctx.state.dialog.is_some() {
                    return Update::none();
                }
                if index == ctx.state.config.active_tab {
                    // Re-selecting the current tab pops all the way back to its list.
                    if ctx.state.config.route.is_none() && !ctx.state.project_details() {
                        return Update::none();
                    }
                    self.reset_detail(ctx);
                } else {
                    self.switch_tab(ctx, index);
                }
                self.persist(ctx);
            }
            Msg::ToggleStar(key) => {
                let title = ctx
                    .state
                    .star_target()
                    .filter(|(target, _)| *target == key)
                    .map(|(_, title)| title)
                    .or_else(|| {
                        ctx.state
                            .items
                            .iter()
                            .chain(&ctx.state.starred_stubs)
                            .find(|i| i.key == key)
                            .map(|i| i.title.clone())
                    })
                    .unwrap_or_default();
                self.toggle_star(ctx, &key, &title);
            }
            Msg::MoveStar(delta) => {
                if ctx.state.dialog.is_none()
                    && ctx.state.scope == Scope::List
                    && ctx.state.starred_active()
                {
                    // Swap with the neighbour actually shown, so filters stay consistent.
                    let at = ctx.state.scroll.selected;
                    let keys = at.checked_add_signed(delta).and_then(|to| {
                        Some((
                            ctx.state.visible_item_at(at)?.key.clone(),
                            ctx.state.visible_item_at(to)?.key.clone(),
                        ))
                    });
                    if let Some((a, b)) = keys
                        && ctx.state.config.swap_stars(&a, &b)
                    {
                        self.move_selection(ctx, delta);
                        self.persist(ctx);
                    }
                }
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
                                self.open_project(ctx, project.id);
                            }
                        } else {
                            let key = ctx
                                .state
                                .visible_item_at(ctx.state.scroll.selected)
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
                        "Fields" if ctx.state.project_details() => {
                            return self.activate_project_field(ctx);
                        }
                        "Forget this project" => return self.action(ctx, Action::RemoveProject),
                        "Pipelines" => {
                            if let Some(job) = ctx.state.selected_job() {
                                let id = job.id;
                                return self.update(Msg::FocusLog(id), ctx);
                            }
                            if ctx.state.drilled.is_some() {
                                // Jobs not loaded yet (or none): nothing to drill further into.
                                return Update::none();
                            }
                            let Some(window) = ctx.state.open_project_pipelines() else {
                                return Update::none();
                            };
                            if let Some(pipeline) = window.pipelines.get(ctx.state.config.field) {
                                let id = pipeline.id;
                                return self.update(Msg::DrillPipeline(id), ctx);
                            }
                            if window.can_load_more() {
                                return self.update(Msg::LoadOlderPipelines, ctx);
                            }
                        }
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
                        "Pipeline" => {
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
                } else if ctx.state.scope == Scope::Section
                    && ctx.state.section_name() == "Pipelines"
                    && let Some(id) = ctx.state.drilled.take()
                {
                    // Back to the pipeline row, which stays expanded.
                    ctx.state.config.field = ctx
                        .state
                        .open_project_pipelines()
                        .and_then(|w| w.position(id))
                        .unwrap_or(0);
                    ctx.state.reveal_content = true;
                } else {
                    match ctx.state.scope {
                        Scope::Section => {
                            ctx.state.scope = Scope::Details;
                            ctx.state.config.section = None;
                            ctx.state.reveal_content = false;
                        }
                        Scope::Details => {
                            ctx.state.remember_selection();
                            let tab = ctx.state.tab_key();
                            ctx.state.navigation_routes_restoring.remove(&tab);
                            ctx.state.scope = Scope::List;
                            ctx.state.config.route = None;
                            ctx.state.config.project_route = None;
                            ctx.state.config.section = None;
                            ctx.state.details = None;
                            ctx.state.detail_epoch += 1;
                            ctx.state.detail_pending = None;
                            ctx.state.traces.clear();
                            ctx.state.expanded.clear();
                            ctx.state.collapsed.clear();
                            ctx.state.log_views.clear();
                            ctx.state.drilled = None;
                            ctx.state.content_offset = 0;
                            ctx.state.restore_selection(false);
                            self.normalize(ctx);
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
                if index < ctx.state.sections().len()
                    && (ctx.state.config.route.is_some() || ctx.state.project_details())
                {
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
                if ctx.state.project_details() {
                    return self.activate_project_field(ctx);
                } else {
                    self.edit_field(ctx);
                }
            }
            Msg::ProjectDetailViewport(id, epoch, target, event) => {
                if !ctx.state.project_details()
                    || ctx.state.config.project_route != Some(id)
                    || ctx.state.detail_epoch != epoch
                {
                    return Update::none();
                }
                ctx.state.content_max_offset = event.metrics.max_offset;
                // A viewport measured for a superseded reveal (rapid navigation) must not
                // cancel the pending one; pipelines make the document long enough to matter.
                if ctx.state.reveal_content
                    && target
                        .as_ref()
                        .is_some_and(|key| *key != ctx.state.detail_target_key())
                {
                    return Update::none();
                }
                ctx.state.content_offset = event.offset;
                ctx.state.reveal_content = false;
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
                        ctx.state.visible_item_count()
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
                if ctx.state.config.active_tab == 1 {
                    let index = ctx
                        .state
                        .config
                        .project_route
                        .and_then(|id| ctx.state.config.projects.iter().position(|p| p.id == id))
                        .unwrap_or(ctx.state.scroll.selected);
                    if let Some(project) = ctx.state.config.projects.get_mut(index) {
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
                        self.reclamp_after_visibility_change(ctx);
                        self.restart_pipeline_sync(ctx);
                        self.persist(ctx);
                        self.normalize(ctx);
                        self.restart_project_sync(ctx);
                    }
                }
            }
            Msg::ToggleProjectKind(kind) => {
                if ctx.state.project_details()
                    && let Some(id) = ctx.state.config.project_route
                    && let Some(project) = ctx.state.config.projects.iter_mut().find(|p| p.id == id)
                {
                    match kind {
                        ItemKind::Issue => project.issues_visible = !project.issues_visible,
                        ItemKind::MergeRequest => {
                            project.merge_requests_visible = !project.merge_requests_visible
                        }
                    }
                    self.reclamp_after_visibility_change(ctx);
                    if kind == ItemKind::MergeRequest {
                        self.restart_pipeline_sync(ctx);
                    }
                    self.persist(ctx);
                    self.normalize(ctx);
                    self.restart_project_sync(ctx);
                }
            }
            Msg::ToggleJob(id) => {
                let Some(job) = ctx
                    .state
                    .current_jobs()
                    .and_then(|(_, jobs)| jobs.iter().find(|j| j.id == id))
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
                    .current_jobs()
                    .and_then(|(_, jobs)| jobs.iter().position(|j| j.id == id))
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
                if ctx
                    .state
                    .dialog
                    .as_ref()
                    .is_some_and(|d| matches!(d.kind, DialogKind::Onboarding))
                {
                    return Update::with_command(self.schedule_setup(ctx));
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
                if ctx
                    .state
                    .dialog
                    .as_ref()
                    .is_some_and(|d| matches!(d.kind, DialogKind::Onboarding))
                {
                    return Update::with_command(self.schedule_setup(ctx));
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
                if ctx
                    .state
                    .dialog
                    .as_ref()
                    .is_some_and(|d| matches!(d.kind, DialogKind::Onboarding))
                {
                    return Update::with_command(self.schedule_setup(ctx));
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
            // Number keys address saved views only; Starred has its own Shift+S.
            (index < 4 + ctx.state.config.views.len()).then_some(Msg::Tab(index))
        } else if matches!(
            (key.code, key.mods.shift),
            (KeyCode::Char('S'), _) | (KeyCode::Char('s'), true)
        ) {
            ctx.state.config.starred_tab().map(Msg::Tab)
        } else if matches!(key.code, KeyCode::Up | KeyCode::Down)
            && key.mods == KeyMods::SHIFT
            && ctx.state.scope == Scope::List
            && ctx.state.starred_active()
        {
            Some(Msg::MoveStar(if key.code == KeyCode::Up { -1 } else { 1 }))
        } else {
            match key.code {
                KeyCode::Left if key.mods == KeyMods::NONE => {
                    ctx.state.config.active_tab.checked_sub(1).map(Msg::Tab)
                }
                KeyCode::Right if key.mods == KeyMods::NONE => {
                    let next = ctx.state.config.active_tab + 1;
                    (next < ctx.state.config.tab_count()).then_some(Msg::Tab(next))
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
                KeyCode::Char(' ')
                    if ctx.state.project_details() && ctx.state.scope == Scope::Section =>
                {
                    match (ctx.state.section_name(), ctx.state.config.field) {
                        ("Fields", 1) => Some(Msg::ToggleProject),
                        ("Fields", 2) => Some(Msg::ToggleProjectKind(ItemKind::Issue)),
                        ("Fields", 3) => Some(Msg::ToggleProjectKind(ItemKind::MergeRequest)),
                        ("Pipelines", row) if ctx.state.drilled.is_none() => ctx
                            .state
                            .open_project_pipelines()
                            .and_then(|w| w.pipelines.get(row))
                            .map(|p| Msg::TogglePipeline(p.id)),
                        _ => None,
                    }
                }
                // Space toggles visibility only from the project list; inside details it
                // acts solely on a focused control (fields, pipelines, jobs above).
                KeyCode::Char(' ') if ctx.state.scope == Scope::List => Some(Msg::ToggleProject),
                KeyCode::Char(' ') => None,
                KeyCode::Char('/') => Some(Msg::Action(Action::Filter)),
                KeyCode::Char('s') => Some(Msg::Action(Action::ToggleStar)),
                KeyCode::Char('n') => Some(Msg::Action(
                    if ctx.state.kind() == Some(ItemKind::MergeRequest) {
                        Action::NewMergeRequest
                    } else {
                        Action::NewIssue
                    },
                )),
                KeyCode::Char('c') => Some(Msg::Action(Action::Comment)),
                KeyCode::Char('r') => Some(Msg::Action(if ctx.state.in_jobs() {
                    Action::RetryJob
                } else {
                    Action::Refresh
                })),
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
    pub(super) fn persist(&self, ctx: &mut Context<Self>) -> bool {
        if ctx.state.config.projects != ctx.state.saved_config.projects
            && let Some(api) = &self.api
        {
            for project in &ctx.state.config.projects {
                api.set_project_visibility(project);
            }
            for old in &ctx.state.saved_config.projects {
                if !ctx.state.config.projects.iter().any(|p| p.id == old.id) {
                    let mut hidden = old.clone();
                    hidden.visible = false;
                    api.set_project_visibility(&hidden);
                }
            }
        }
        ctx.state.refresh_starred_stubs();
        if ctx.state.config.active_tab >= ctx.state.config.tab_count() {
            // The Starred tab vanished (for example its project was hidden).
            let gone = ctx.state.config.active_tab;
            self.switch_tab(ctx, 0);
            self.forget_tab(ctx, gone);
        }
        let key = ctx.state.tab_key();
        ctx.state
            .config
            .selections
            .insert(key.clone(), ctx.state.scroll.selected);
        let tab = navigation(&ctx.state);
        ctx.state.config.tab_states.insert(key, tab);
        prune_tab_routes(&mut ctx.state.config);
        // Per-tab leftovers of a vanished tab (for example Starred) must not be
        // inherited by whichever saved view later takes its index.
        let tab_count = ctx.state.config.tab_count();
        let live = |key: &String| key.parse::<usize>().map_or(true, |index| index < tab_count);
        ctx.state.config.filters.retain(|key, _| live(key));
        ctx.state.config.selections.retain(|key, _| live(key));
        ctx.state.navigation_selections.retain(|key, _| live(key));
        ctx.state.navigation_restoring.retain(live);
        ctx.state.navigation_routes_restoring.retain(live);
        ctx.state.tab_cache.retain(|key, _| live(key));
        ctx.state.invalidate_hidden_navigation();
        if ctx.state.config.route.as_ref().is_some_and(|route| {
            !ctx.state
                .config
                .projects
                .iter()
                .any(|p| p.id == route.project && p.kind_visible(route.kind))
        }) {
            self.reset_detail(ctx);
        }
        let config = &ctx.state.config;
        let visible: HashSet<_> = config
            .projects
            .iter()
            .filter(|p| p.effectively_visible())
            .map(|p| p.id)
            .collect();
        let item_visible = |key: &ItemKey| {
            config
                .projects
                .iter()
                .any(|p| p.id == key.project && p.kind_visible(key.kind))
        };
        ctx.state.shared_details.retain(|key, _| item_visible(key));
        ctx.state.detail_requests.retain(|key, _| item_visible(key));
        ctx.state
            .detail_schema_errors
            .retain(|key, _| item_visible(key));
        ctx.state
            .detail_retry_requested
            .retain(|key| item_visible(key));
        ctx.state.sync_progress.retain(|id, _| visible.contains(id));
        ctx.state.tab_cache.retain(|key, _| {
            config
                .tab_states
                .get(key)
                .is_some_and(|tab| tab.route.is_some())
        });
        if !ctx.state.config.portable_eq(&ctx.state.saved_config) {
            let installation_changed = ctx.state.config.gitlab_url.trim_end_matches('/')
                != ctx.state.saved_config.gitlab_url.trim_end_matches('/');
            if let Some(path) = &self.path
                && let Err(error) = ctx.state.config.save(path)
            {
                ctx.state.error = Some(format!("Workspace not saved: {error:#}"));
                return false;
            }
            *ctx.state.saved_config = ctx.state.config.clone();
            if installation_changed {
                // Only discard old installation UI state after config save
                // succeeds. The same background writer opens the new namespace.
                self.reset_detail(ctx);
                ctx.state.config.clear_navigation();
                ctx.state.navigation_selections.clear();
                ctx.state.navigation_restoring.clear();
                ctx.state.navigation_routes_restoring.clear();
                ctx.state.navigation_hydrated.clear();
                ctx.state.items.clear();
                ctx.state
                    .config
                    .tab_states
                    .insert("0".into(), navigation(&ctx.state));
                ctx.state.config.selections.insert("0".into(), 0);
                ctx.state.scroll = BoundaryScroll::default();
            }
        }
        ctx.state.remember_selection();
        if let Some(writer) = &ctx.state.navigation_writer {
            let snapshot = crate::navigation::Snapshot::capture(
                &ctx.state.config,
                &ctx.state.navigation_selections,
                ctx.state.demo,
            );
            if ctx.state.navigation_snapshot.as_ref() != Some(&snapshot) {
                writer.submit(snapshot.clone());
                ctx.state.navigation_snapshot = Some(snapshot);
            }
        }
        true
    }

    pub(super) fn enable_cache(&mut self, ctx: &mut Context<Self>) {
        if !ctx.state.demo
            && let (Some(api), Some(path)) = (&mut self.api, &self.path)
            && let Err(error) = api.enable_persistence(path)
        {
            ctx.state.error = Some(format!(
                "Persistent cache unavailable (using memory): {error:#}"
            ));
        }
    }

    fn merge_items(&self, ctx: &mut Context<Self>, items: Vec<WorkItem>) {
        if items.is_empty() {
            return;
        }
        let selected = ctx
            .state
            .visible_item_at(ctx.state.scroll.selected)
            .map(|i| i.key.clone());
        let mut merged: HashMap<_, _> = std::mem::take(&mut ctx.state.items)
            .into_iter()
            .map(|i| (i.key.clone(), i))
            .collect();
        for item in items {
            merged.insert(item.key.clone(), item);
        }
        ctx.state.items = merged.into_values().collect();
        if let Some(key) = selected
            && let Some(index) = ctx.state.visible_position(&key)
        {
            ctx.state.scroll.selected = index;
        }
        ctx.state.restore_selection(false);
        self.normalize(ctx);
    }

    fn normalize(&self, ctx: &mut Context<Self>) {
        if !ctx.state.demo
            && ctx.state.items.is_empty()
            && ctx.state.config.active_tab != 1
            && ctx
                .state
                .navigation_restoring
                .contains(&ctx.state.tab_key())
        {
            return;
        }
        let len = if ctx.state.config.active_tab == 1 {
            ctx.state.config.projects.len()
        } else {
            ctx.state.visible_item_count()
        };
        let height = list_height_for_tab(ctx.viewport().h, ctx.state.config.active_tab);
        ctx.state.scroll.normalize(len, height);
    }

    fn restart_project_sync(&self, ctx: &mut Context<Self>) {
        // A worker owns a snapshot of collection preferences. Never apply its
        // partial/skipped collections using the newly selected preferences.
        ctx.state.list_epoch += 1;
        ctx.state.list_pending.clear();
        ctx.state.sync_progress.clear();
        ctx.state.next_lists = Duration::ZERO;
        // Per-project API locks serialize replacements with any old import.
        ctx.link().send(Msg::Refresh);
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
            .filter(|p| p.effectively_visible())
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

    fn retry_detail_schemas(&self, ctx: &mut Context<Self>, key: Option<&ItemKey>) {
        if let Some(api) = &self.api {
            api.retry_schema_errors(key);
        }
        let matches = |candidate: &ItemKey| key.is_none_or(|key| key == candidate);
        ctx.state
            .detail_schema_errors
            .retain(|candidate, _| !matches(candidate));
        ctx.state.detail_retry_requested.extend(
            ctx.state
                .detail_requests
                .keys()
                .filter(|candidate| matches(candidate))
                .cloned(),
        );
        for (candidate, (_, due)) in &mut ctx.state.shared_details {
            if matches(candidate) {
                *due = Duration::ZERO;
            }
        }
        for cache in ctx.state.tab_cache.values_mut() {
            if cache
                .details
                .as_ref()
                .is_none_or(|details| matches(&details.item.key))
            {
                cache.next_details = Duration::ZERO;
            }
        }
        if ctx.state.config.route.as_ref().is_some_and(matches) {
            ctx.state.next_details = Duration::ZERO;
        }
    }

    fn apply_user(&self, ctx: &mut Context<Self>, result: Result<User, String>) {
        ctx.state.user_pending = false;
        if std::mem::take(&mut ctx.state.user_retry_requested) {
            ctx.state.user_schema_error = false;
            if let Err(error) = result
                && !error.contains("invalid JSON/schema")
            {
                self.network_error(ctx, error);
            }
            ctx.link().send(Msg::LoadUser);
            return;
        }
        match result {
            Ok(user) => {
                ctx.state.user_schema_error = false;
                ctx.state.user = user;
                ctx.state
                    .restore_selection(ctx.state.navigation_hydration_complete());
                self.persist(ctx);
            }
            Err(error) => {
                ctx.state.user_schema_error = error.contains("invalid JSON/schema");
                self.network_error(ctx, error);
            }
        }
    }

    /// Back off like `network_error` without replacing the visible error banner.
    fn quiet_network_error(&self, ctx: &mut Context<Self>, error: &str) {
        let visible = ctx.state.error.take();
        self.network_error(ctx, error.to_owned());
        ctx.state.error = visible;
    }

    pub(super) fn network_error(&self, ctx: &mut Context<Self>, error: String) {
        // A bad resource schema is not a server outage or a rate limit. Blocking
        // unrelated list/detail/trace requests cannot make that response valid.
        if error.contains("invalid JSON/schema") {
            ctx.state.error = Some(error);
            return;
        }
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
        let Some(source) = ctx.state.job_source() else {
            return Update::none();
        };
        let Some((project, jobs)) = ctx.state.current_jobs() else {
            return Update::none();
        };
        if ctx.elapsed() < ctx.state.blocked_until {
            return Update::none();
        }
        let key = source;
        let epoch = ctx.state.detail_epoch;
        let requests: Vec<_> = jobs
            .iter()
            .filter(|job| {
                ctx.state.trace_wanted(job)
                    && !ctx.state.trace_pending.iter().any(|(_, id)| *id == job.id)
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

    /// Kept out of `update` so its stack frame stays small in debug builds.
    #[inline(never)]
    fn pipelines_loaded(
        &mut self,
        ctx: &mut Context<Self>,
        project: u64,
        epoch: u64,
        page: usize,
        result: Result<Pipelines<Vec<Pipeline>>, String>,
    ) -> Update {
        // A superseded reply must not evict the entry of a request re-issued after the
        // epoch bump (restart_pipeline_sync clears the pending sets itself).
        if epoch != ctx.state.pipeline_epoch {
            return Update::none();
        }
        ctx.state.pipelines_pending.remove(&(project, page));
        // Keep the selected row identity (a pipeline, or the footer) while the window shifts.
        let on_rows = ctx.state.config.project_route == Some(project)
            && ctx.state.scope == Scope::Section
            && ctx.state.section_name() == "Pipelines"
            && ctx.state.drilled.is_none();
        let selected = on_rows
            .then(|| {
                ctx.state
                    .open_project_pipelines()
                    .and_then(|w| w.pipelines.get(ctx.state.config.field))
                    .map(|p| p.id)
            })
            .flatten();
        let on_footer = on_rows
            && ctx
                .state
                .open_project_pipelines()
                .is_some_and(|w| w.can_load_more() && ctx.state.config.field == w.pipelines.len());
        let keep: HashSet<u64> = ctx
            .state
            .pipeline_expanded
            .iter()
            .chain(ctx.state.drilled.iter())
            .copied()
            .collect();
        let window = ctx.state.project_pipelines.entry(project).or_default();
        match result {
            Ok(Pipelines::Available(pipelines)) => {
                if page == 1 {
                    match pipelines.first() {
                        Some(newest) => {
                            ctx.state.latest_pipeline.insert(project, newest.clone());
                        }
                        None => {
                            ctx.state.latest_pipeline.remove(&project);
                        }
                    }
                }
                window.merge(page, pipelines, &keep);
                if let Some(id) = selected
                    && let Some(index) = window.position(id)
                {
                    ctx.state.config.field = index;
                } else if on_footer {
                    ctx.state.config.field = window.row_count().saturating_sub(1);
                } else if on_rows {
                    ctx.state.config.field = ctx
                        .state
                        .config
                        .field
                        .min(window.row_count().saturating_sub(1));
                }
                if let Some(id) = ctx.state.drilled
                    && ctx.state.config.project_route == Some(project)
                    && window.get(id).is_none()
                {
                    // The drilled pipeline left the window: back to the pipeline rows.
                    ctx.state.drilled = None;
                    ctx.state.config.field = 0;
                    self.close_log_mode(ctx);
                }
                if ctx.state.config.project_route == Some(project) {
                    ctx.state.status = "Pipelines updated".into();
                }
                return Update::with_command(self.load_pipeline_details(ctx, project));
            }
            Ok(Pipelines::Unavailable(reason)) => {
                // A per-project fact (CI/CD disabled, no permission): stop asking until an
                // explicit refresh or visibility change, and leave the global backoff alone.
                ctx.state.pipelines_unavailable.insert(project, reason);
                ctx.state.latest_pipeline.remove(&project);
            }
            Err(error) => {
                window.error = Some(error.clone());
                self.network_error(ctx, error);
            }
        }
        Update::full()
    }

    /// Kept out of `update` so its stack frame stays small in debug builds.
    #[inline(never)]
    fn pipeline_loaded(
        &mut self,
        ctx: &mut Context<Self>,
        project: u64,
        pipeline: u64,
        epoch: u64,
        result: Result<Box<LoadedPipeline>, String>,
    ) -> Update {
        if epoch != ctx.state.pipeline_epoch {
            return Update::none();
        }
        ctx.state.pipeline_detail_pending.remove(&pipeline);
        if !ctx.state.project_pipelines.contains_key(&project) {
            return Update::none();
        }
        match result {
            Ok(loaded) => {
                let (full, jobs, warnings) = *loaded;
                let drilled_here = ctx.state.config.project_route == Some(project)
                    && ctx.state.drilled == Some(pipeline);
                let job_id = ctx
                    .state
                    .selected_job()
                    .filter(|_| drilled_here)
                    .map(|j| j.id);
                let window = ctx.state.project_pipelines.get_mut(&project).unwrap();
                let first_jobs = !window.jobs.contains_key(&pipeline);
                let stamp = full.updated_at.clone();
                if let Some(existing) = window.pipelines.iter_mut().find(|p| p.id == pipeline) {
                    *existing = full;
                }
                window.jobs.insert(pipeline, PipelineJobs { jobs, stamp });
                window.error = warnings.into_iter().next();
                let jobs = &window.jobs[&pipeline].jobs;
                if drilled_here {
                    if let Some(index) = jobs.iter().position(|j| Some(j.id) == job_id) {
                        ctx.state.config.field = index;
                    } else {
                        ctx.state.config.field =
                            ctx.state.config.field.min(jobs.len().saturating_sub(1));
                    }
                    let focus_lost = ctx
                        .state
                        .log_focus
                        .is_some_and(|id| !jobs.iter().any(|j| j.id == id));
                    if focus_lost {
                        self.close_log_mode(ctx);
                    }
                    if first_jobs {
                        // The first render after drilling targeted the pipeline row; now
                        // that its jobs exist, bring the selected job into view.
                        ctx.state.reveal_content = true;
                    }
                    ctx.link().send(Msg::LoadTraces);
                }
            }
            Err(error) => {
                if let Some(window) = ctx.state.project_pipelines.get_mut(&project) {
                    // Retry only when the row changes or on explicit refresh: a deleted
                    // pipeline that stays expanded must not drive the backoff every cycle.
                    let stamp = window.get(pipeline).and_then(|p| p.updated_at.clone());
                    window.jobs.entry(pipeline).or_insert_with(|| PipelineJobs {
                        jobs: Vec::new(),
                        stamp,
                    });
                    window.error = Some(error.clone());
                }
                self.network_error(ctx, error);
            }
        }
        Update::full()
    }

    /// Kept out of `update` so its stack frame stays small in debug builds.
    #[inline(never)]
    fn toggle_pipeline(&mut self, ctx: &mut Context<Self>, id: u64) -> Update {
        let Some(project) = ctx.state.config.project_route else {
            return Update::none();
        };
        if ctx.state.pipeline_expanded(project, id) {
            ctx.state.pipeline_expanded.remove(&id);
            ctx.state.pipeline_collapsed.insert(id);
            if ctx.state.drilled == Some(id) {
                ctx.state.drilled = None;
                ctx.state.config.field = ctx
                    .state
                    .open_project_pipelines()
                    .and_then(|w| w.position(id))
                    .unwrap_or(0);
                self.close_log_mode(ctx);
            }
            Update::full()
        } else {
            ctx.state.pipeline_collapsed.remove(&id);
            ctx.state.pipeline_expanded.insert(id);
            Update::with_command(self.load_pipeline_details(ctx, project))
        }
    }

    /// Kept out of `update` so its stack frame stays small in debug builds.
    #[inline(never)]
    fn click_pipeline(&mut self, ctx: &mut Context<Self>, id: u64) -> Update {
        self.close_log_mode(ctx);
        let Some(index) = ctx
            .state
            .open_project_pipelines()
            .and_then(|w| w.position(id))
        else {
            return Update::none();
        };
        if let Some(section) = ctx.state.section_index("Pipelines") {
            ctx.state.section_cursor = section;
            ctx.state.config.section = Some(section);
        }
        ctx.state.scope = Scope::Section;
        ctx.state.drilled = None;
        ctx.state.config.field = index;
        ctx.state.reveal_content = false;
        self.persist(ctx);
        self.update(Msg::TogglePipeline(id), ctx)
    }

    /// Kept out of `update` so its stack frame stays small in debug builds.
    #[inline(never)]
    fn drill_pipeline(&mut self, ctx: &mut Context<Self>, id: u64) -> Update {
        let Some(project) = ctx.state.config.project_route else {
            return Update::none();
        };
        if ctx
            .state
            .open_project_pipelines()
            .and_then(|w| w.get(id))
            .is_none()
        {
            return Update::none();
        }
        if let Some(section) = ctx.state.section_index("Pipelines") {
            ctx.state.section_cursor = section;
            ctx.state.config.section = Some(section);
        }
        ctx.state.scope = Scope::Section;
        ctx.state.pipeline_collapsed.remove(&id);
        ctx.state.pipeline_expanded.insert(id);
        ctx.state.drilled = Some(id);
        ctx.state.config.field = 0;
        ctx.state.reveal_content = true;
        ctx.link().send(Msg::LoadTraces);
        self.persist(ctx);
        Update::with_command(self.load_pipeline_details(ctx, project))
    }

    /// Kept out of `update` so its stack frame stays small in debug builds.
    #[inline(never)]
    fn probe_loaded(
        &mut self,
        ctx: &mut Context<Self>,
        project: u64,
        epoch: u64,
        result: Result<Pipelines<Option<Box<Pipeline>>>, String>,
    ) -> Update {
        if epoch != ctx.state.pipeline_epoch {
            return Update::none();
        }
        ctx.state.probe_pending.remove(&project);
        match result {
            Ok(Pipelines::Available(Some(pipeline))) => {
                ctx.state.latest_pipeline.insert(project, *pipeline);
            }
            Ok(Pipelines::Available(None)) => {
                ctx.state.latest_pipeline.remove(&project);
            }
            Ok(Pipelines::Unavailable(reason)) => {
                ctx.state.pipelines_unavailable.insert(project, reason);
                ctx.state.latest_pipeline.remove(&project);
            }
            // A flaky CI endpoint must not nag on every tab: back off quietly.
            Err(error) => self.quiet_network_error(ctx, &error),
        }
        Update::full()
    }

    /// The Pipelines section follows merge request visibility: keep the section
    /// cursor valid and stop any drilled job polling when it disappears.
    fn reclamp_after_visibility_change(&self, ctx: &mut Context<Self>) {
        if !ctx.state.project_details() {
            return;
        }
        let last = ctx.state.sections().len() - 1;
        if !ctx.state.pipelines_section_present() {
            // The Pipelines heading sat at index 1; do not let the cursor slide onto
            // the destructive "Forget this project" section that now occupies it.
            if ctx.state.section_cursor == 1 && ctx.state.section_cursor <= last {
                ctx.state.section_cursor = 0;
                ctx.state.config.section = ctx.state.config.section.map(|_| 0);
                ctx.state.config.field = 0;
            }
            ctx.state.drilled = None;
            self.close_log_mode(ctx);
        }
        ctx.state.section_cursor = ctx.state.section_cursor.min(last);
        ctx.state.config.section = ctx.state.config.section.map(|s| s.min(last));
    }

    /// Visibility or setup changed: forget per-project unavailability, invalidate
    /// in-flight pipeline replies and re-probe soon.
    pub(super) fn restart_pipeline_sync(&self, ctx: &mut Context<Self>) {
        ctx.state.pipeline_epoch += 1;
        ctx.state.pipelines_pending.clear();
        ctx.state.pipeline_detail_pending.clear();
        ctx.state.probe_pending.clear();
        ctx.state.pipelines_unavailable.clear();
        ctx.state.next_pipelines = Duration::ZERO;
        ctx.state.next_probe = Duration::ZERO;
        if ctx.state.pipelines_section_present() {
            ctx.link().send(Msg::LoadPipelines);
        }
    }

    /// Page 1 (or the next older page) of the open project's pipelines, then the
    /// detail + jobs of expanded pipelines. Memory-only; nothing is cached on disk.
    fn load_pipelines(&self, ctx: &mut Context<Self>, older: bool) -> Update {
        if !older {
            ctx.state.next_pipelines =
                ctx.elapsed() + Duration::from_secs(ctx.state.config.detail_refresh_secs);
        }
        if !ctx.state.pipelines_section_present() {
            return Update::none();
        }
        let Some(project) = ctx.state.config.project_route else {
            return Update::none();
        };
        if ctx.state.pipelines_unavailable.contains_key(&project) {
            return Update::none();
        }
        let page = if older {
            let Some(window) = ctx.state.project_pipelines.get(&project) else {
                return Update::none();
            };
            if !window.can_load_more() {
                return Update::none();
            }
            window.pages_loaded + 1
        } else {
            1
        };
        if ctx.state.pipelines_pending.contains(&(project, page))
            || ctx.elapsed() < ctx.state.blocked_until
        {
            return Update::none();
        }
        let epoch = ctx.state.pipeline_epoch;
        let Some(api) = self.api.clone() else {
            if ctx.state.demo {
                let fresh = demo::project_pipelines(project, page);
                ctx.state.pipelines_pending.insert((project, page));
                ctx.link().send(Msg::PipelinesLoaded(
                    project,
                    epoch,
                    page,
                    Ok(Pipelines::Available(fresh)),
                ));
            }
            return Update::full();
        };
        ctx.state.pipelines_pending.insert((project, page));
        ctx.state.project_pipelines.entry(project).or_default();
        Update::with_command(ctx.link().command(move |link| {
            let result = api
                .project_pipelines(project, page)
                .map_err(|e| e.to_string());
            link.send(Msg::PipelinesLoaded(project, epoch, page, result));
        }))
    }

    /// Detail + jobs for expanded/drilled pipelines (bounded, independent deliveries).
    fn load_pipeline_details(&self, ctx: &mut Context<Self>, project: u64) -> Option<Command> {
        if ctx.elapsed() < ctx.state.blocked_until {
            return None;
        }
        let wanted: Vec<u64> = ctx
            .state
            .wanted_pipelines(project)
            .into_iter()
            .filter(|id| !ctx.state.pipeline_detail_pending.contains(id))
            .take(4)
            .collect();
        if wanted.is_empty() {
            return None;
        }
        let epoch = ctx.state.pipeline_epoch;
        let Some(api) = self.api.clone() else {
            if ctx.state.demo {
                for id in wanted {
                    let result = demo::pipeline_with_jobs(project, id)
                        .map(|(pipeline, jobs)| Box::new((pipeline, jobs, Vec::new())));
                    ctx.link()
                        .send(Msg::PipelineLoaded(project, id, epoch, result));
                }
            }
            return None;
        };
        ctx.state
            .pipeline_detail_pending
            .extend(wanted.iter().copied());
        Some(ctx.link().command(move |link| {
            std::thread::scope(|scope| {
                for &id in &wanted {
                    let api = api.clone();
                    let link = link.clone();
                    scope.spawn(move || {
                        let result = api
                            .pipeline_with_jobs(project, id)
                            .map(Box::new)
                            .map_err(|e| e.to_string());
                        link.send(Msg::PipelineLoaded(project, id, epoch, result));
                    });
                }
            });
        }))
    }

    /// Newest pipeline for every MR-visible project except the open one, whose page 1
    /// already covers it. Slower than list refresh: this only feeds the row indicator.
    fn load_probes(&self, ctx: &mut Context<Self>) -> Update {
        ctx.state.next_probe =
            ctx.elapsed() + Duration::from_secs(ctx.state.config.pipeline_refresh_secs());
        if ctx.elapsed() < ctx.state.blocked_until {
            return Update::none();
        }
        let open = ctx
            .state
            .config
            .project_route
            .filter(|_| ctx.state.pipelines_section_present());
        let ids: Vec<u64> = ctx
            .state
            .config
            .projects
            .iter()
            .filter(|p| p.kind_visible(ItemKind::MergeRequest) && Some(p.id) != open)
            .map(|p| p.id)
            .filter(|id| {
                !ctx.state.probe_pending.contains(id)
                    && !ctx.state.pipelines_unavailable.contains_key(id)
            })
            .collect();
        if ids.is_empty() {
            return Update::none();
        }
        let epoch = ctx.state.pipeline_epoch;
        let Some(api) = self.api.clone() else {
            if ctx.state.demo {
                for id in ids {
                    let latest = demo::project_pipelines(id, 1).into_iter().next();
                    ctx.link().send(Msg::ProbeLoaded(
                        id,
                        epoch,
                        Ok(Pipelines::Available(latest.map(Box::new))),
                    ));
                }
            }
            return Update::full();
        };
        ctx.state.probe_pending.extend(ids.iter().copied());
        Update::with_command(ctx.link().command(move |link| {
            for batch in ids.chunks(4) {
                std::thread::scope(|scope| {
                    for &id in batch {
                        let api = api.clone();
                        let link = link.clone();
                        scope.spawn(move || {
                            let result = api
                                .latest_pipeline(id)
                                .map(|outcome| match outcome {
                                    Pipelines::Available(p) => {
                                        Pipelines::Available(p.map(Box::new))
                                    }
                                    Pipelines::Unavailable(reason) => {
                                        Pipelines::Unavailable(reason)
                                    }
                                })
                                .map_err(|e| e.to_string());
                            link.send(Msg::ProbeLoaded(id, epoch, result));
                        });
                    }
                });
            }
        }))
    }

    /// Drop all remembered state for a tab index that is vanishing or being reused.
    fn forget_tab(&self, ctx: &mut Context<Self>, index: usize) {
        let key = index.to_string();
        let state = &mut ctx.state;
        state.config.filters.remove(&key);
        state.config.selections.remove(&key);
        state.config.tab_states.remove(&key);
        state.navigation_selections.remove(&key);
        state.navigation_restoring.remove(&key);
        state.navigation_routes_restoring.remove(&key);
        state.tab_cache.remove(&key);
    }

    /// Star or unstar an item, adding or removing the Starred tab as needed.
    fn toggle_star(&self, ctx: &mut Context<Self>, key: &ItemKey, title: &str) {
        let before = ctx.state.config.starred_tab();
        ctx.state.config.toggle_star(key, title);
        ctx.state.refresh_starred_stubs();
        // The last star is gone: leave and forget the vanished tab.
        if let Some(tab) = before
            && ctx.state.config.starred_tab().is_none()
        {
            if ctx.state.config.active_tab == tab {
                self.switch_tab(ctx, 0);
            }
            self.forget_tab(ctx, tab);
        }
        self.normalize(ctx);
        self.persist(ctx);
    }

    fn switch_tab(&self, ctx: &mut Context<Self>, index: usize) {
        if index == ctx.state.config.active_tab {
            return;
        }
        ctx.state.feedback.clear();
        let key = ctx.state.tab_key();
        ctx.state.remember_selection();
        let tab = navigation(&ctx.state);
        ctx.state.config.tab_states.insert(key.clone(), tab);
        ctx.state
            .config
            .selections
            .insert(key.clone(), ctx.state.scroll.selected);
        if let Some(details) = &ctx.state.details {
            ctx.state.shared_details.insert(
                details.item.key.clone(),
                (details.clone(), ctx.state.next_details),
            );
        }
        let cache = TabCache {
            details: ctx.state.details.take(),
            drilled: ctx.state.drilled.take(),
            traces: std::mem::take(&mut ctx.state.traces),
            log_views: std::mem::take(&mut ctx.state.log_views),
            next_details: ctx.state.next_details,
            next_traces: ctx.state.next_traces,
        };
        ctx.state.tab_cache.insert(key, cache);
        ctx.state.config.active_tab = index;
        let key = ctx.state.tab_key();
        if ctx.state.navigation_selections.contains_key(&key) {
            ctx.state.navigation_restoring.insert(key.clone());
        }
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
        ctx.state.config.project_route = tab.project_route;
        ctx.state.config.section = tab.section;
        ctx.state.config.field = tab.field;
        let open_details = ctx.state.config.route.is_some()
            || (ctx.state.config.active_tab == 1 && ctx.state.config.project_route.is_some());
        ctx.state.scope = if !open_details {
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
        ctx.state.drilled = cache.drilled;
        ctx.state.traces = cache.traces;
        ctx.state.log_views = cache.log_views;
        self.close_log_mode(ctx);
        ctx.state.next_details = cache.next_details;
        ctx.state.next_traces = cache.next_traces;
        if let Some(key) = ctx.state.config.route.clone()
            && let Some((details, due)) = ctx.state.shared_details.get(&key)
        {
            ctx.state.details = Some(details.clone());
            ctx.state.next_details = *due;
        }
        if let Some(key) = ctx.state.config.route.clone()
            && ctx.state.detail_requests.contains_key(&key)
        {
            ctx.state.detail_pending = Some((key, ctx.state.detail_epoch));
        }
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
            ctx.state.restore_selection(
                self.demo || index == 1 || ctx.state.navigation_hydration_complete(),
            );
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

    fn open_project(&self, ctx: &mut Context<Self>, id: u64) {
        let tab = ctx.state.tab_key();
        ctx.state.navigation_restoring.remove(&tab);
        ctx.state.navigation_routes_restoring.remove(&tab);
        ctx.state.detail_viewport = None;
        ctx.state.detail_offset_request = None;
        ctx.state.reveal_content = true;
        ctx.state.content_max_offset = usize::MAX;
        ctx.state.config.project_route = Some(id);
        ctx.state.config.route = None;
        ctx.state.config.section = None;
        ctx.state.scope = Scope::Details;
        ctx.state.section_cursor = 0;
        ctx.state.config.field = 0;
        ctx.state.content_offset = 0;
        ctx.state.detail_epoch += 1;
        ctx.state.detail_pending = None;
        ctx.state.details = None;
        ctx.state.traces.clear();
        ctx.state.expanded.clear();
        ctx.state.collapsed.clear();
        ctx.state.log_views.clear();
        ctx.state.drilled = None;
        self.close_log_mode(ctx);
        // Older pages are dropped when a project is (re)opened; page 1 stays as a
        // snapshot that renders immediately and then refreshes.
        if let Some(window) = ctx.state.project_pipelines.get_mut(&id) {
            window.pipelines.truncate(crate::gitlab::PIPELINE_PAGE);
            window.pages_loaded = window.pages_loaded.min(1);
            window.exhausted = false;
            window.jobs.clear();
        }
        ctx.state.next_pipelines = Duration::ZERO;
        if ctx.state.pipelines_section_present() {
            ctx.link().send(Msg::LoadPipelines);
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

    /// Drop credential-specific buffers without resetting machine-local routes,
    /// positions or expansion choices (cache eviction and client replacement).
    pub(super) fn clear_detail_content(&self, ctx: &mut Context<Self>) {
        ctx.state.details = None;
        ctx.state.detail_epoch += 1;
        ctx.state.detail_pending = None;
        ctx.state.trace_pending.clear();
        ctx.state.traces.clear();
        ctx.state.log_views.clear();
        ctx.state.detail_viewport = None;
        ctx.state.detail_offset_request = Some(ctx.state.content_offset);
        self.close_log_mode(ctx);
    }

    pub(super) fn reset_detail(&self, ctx: &mut Context<Self>) {
        ctx.state.remember_selection();
        let tab = ctx.state.tab_key();
        ctx.state.navigation_routes_restoring.remove(&tab);
        ctx.state.config.route = None;
        ctx.state.config.project_route = None;
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
        ctx.state.drilled = None;
        self.close_log_mode(ctx);
        ctx.state.section_cursor = 0;
        ctx.state.content_offset = 0;
        ctx.state.restore_selection(false);
        self.normalize(ctx);
    }

    fn open_item(&self, ctx: &mut Context<Self>, key: ItemKey) {
        let tab = ctx.state.tab_key();
        ctx.state.navigation_restoring.remove(&tab);
        ctx.state.navigation_routes_restoring.remove(&tab);
        ctx.state.detail_viewport = None;
        ctx.state.detail_offset_request = None;
        ctx.state.reveal_content = true;
        ctx.state.content_max_offset = usize::MAX;
        ctx.state.config.route = Some(key.clone());
        ctx.state.config.project_route = None;
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
        ctx.state.details = ctx
            .state
            .shared_details
            .get(&key)
            .map(|(d, _)| d.clone())
            .or_else(|| {
                if ctx.state.demo {
                    Some(demo::details(&key))
                } else {
                    {
                        ctx.state
                            .items
                            .iter()
                            .find(|i| i.key == key)
                            .map(|item| Details {
                                item: item.clone(),
                                loaded: HashSet::from([DetailPart::Core]),
                                ..Default::default()
                            })
                    }
                }
            });
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
                    ctx.state.visible_item_count()
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
                    "Fields" if ctx.state.project_details() => 4,
                    "Forget this project" => 1,
                    "Fields" => ctx.state.fields().len(),
                    "Pipeline" => ctx.state.details.as_ref().map_or(0, |d| d.jobs.len()),
                    "Pipelines" => {
                        if ctx.state.drilled.is_some() {
                            ctx.state.current_jobs().map_or(0, |(_, jobs)| jobs.len())
                        } else {
                            ctx.state
                                .open_project_pipelines()
                                .map_or(0, ProjectPipelines::row_count)
                        }
                    }
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
                    ctx.state.visible_item_count()
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

    pub(super) fn show_dialog(
        &self,
        ctx: &mut Context<Self>,
        kind: DialogKind,
        title: &str,
        help: &str,
        fields: Vec<FormField>,
    ) {
        let (selected, original_theme) = if matches!(kind, DialogKind::Themes) {
            let original = ctx.state.config.theme.clone();
            let selected = crate::config::THEMES
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
    pub(super) fn close_dialog(&self, ctx: &mut Context<Self>) {
        if ctx.state.mutation_pending {
            ctx.state.status =
                "Waiting for GitLab to confirm the request; the draft is retained".into();
            return;
        }
        if ctx
            .state
            .dialog
            .as_ref()
            .is_some_and(|d| matches!(d.kind, DialogKind::Onboarding))
        {
            let previous = ctx.state.config.onboarding;
            ctx.state.config.onboarding = false;
            if previous && !self.persist(ctx) {
                ctx.state.config.onboarding = previous;
                self.dialog_error(ctx, "Could not save configuration; setup is retained");
                return;
            }
            ctx.state.onboarding.epoch += 1;
            ctx.state.onboarding.validated = None;
            if !ctx.state.demo && self.api.is_none() {
                ctx.state.status =
                    "Setup skipped · reopen Set up GitLab from Ctrl+P, or edit config and restart"
                        .into();
            }
        }
        if let Some(dialog) = ctx.state.dialog.take()
            && let Some(theme) = dialog.original_theme
        {
            ctx.state.config.theme = theme;
        }
        ctx.state.feedback.clear();
        ctx.blur();
    }
    pub(super) fn dialog_error(&self, ctx: &mut Context<Self>, error: &str) {
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
            crate::config::THEMES.len()
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
        if let Some(theme) = crate::config::THEMES.get(selected) {
            ctx.state.config.theme = (*theme).into();
        }
    }

    fn activate_project_field(&mut self, ctx: &mut Context<Self>) -> Update {
        match ctx.state.config.field {
            0 => {
                self.edit_project_alias(ctx);
                Update::full()
            }
            1 => self.update(Msg::ToggleProject, ctx),
            2 => self.update(Msg::ToggleProjectKind(ItemKind::Issue), ctx),
            3 => self.update(Msg::ToggleProjectKind(ItemKind::MergeRequest), ctx),
            _ => Update::none(),
        }
    }

    fn edit_project_alias(&self, ctx: &mut Context<Self>) {
        let Some(id) = ctx.state.config.project_route else {
            return;
        };
        let Some(project) = ctx.state.project(id) else {
            return;
        };
        let alias = project.alias.clone();
        self.show_dialog(
            ctx,
            DialogKind::AliasProject(id),
            "Project alias",
            "Local display name · leave empty to show the GitLab path · Enter saves · Esc cancels",
            vec![FormField::new("Alias", &alias, false)],
        );
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
            "milestone_id" | "iteration_id" | "epic_id" => {
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
            "epic_id" => Some(LookupKind::Epics),
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
                    LookupKind::Milestones | LookupKind::Iterations | LookupKind::Epics => {
                        let id = match kind {
                            LookupKind::Milestones => item.milestone_id,
                            LookupKind::Iterations => item.iteration_id,
                            LookupKind::Epics => item.epic_id,
                            _ => unreachable!(),
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
            Action::FullResync => { self.close_dialog(ctx); return self.update(Msg::FullResync, ctx); }
            Action::ClearCache => { self.close_dialog(ctx); return self.update(Msg::ClearCache, ctx); }
            Action::Onboarding => {
                self.show_onboarding(ctx);
                return Update::with_command(self.schedule_setup(ctx));
            }
            Action::Quit => {
                if self.persist(ctx) {
                    match ctx.state.flush_navigation() {
                        Ok(()) => ctx.quit(),
                        Err(error) if ctx.state.navigation_exit_failed => {
                            let error = format!("{error:#} · exiting with unsaved local navigation");
                            eprintln!("{error}");
                            ctx.state.error = Some(error);
                            ctx.quit();
                        }
                        Err(error) => {
                            ctx.state.navigation_exit_failed = true;
                            let error = format!("{error:#} · Quit again to retry, then exit even if saving fails");
                            ctx.state.navigation_reported_error = Some(error.clone());
                            ctx.state.error = Some(error);
                        }
                    }
                }
            }
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
            Action::ToggleStar => {
                if let Some((key, title)) = ctx.state.star_target() {
                    self.close_dialog(ctx);
                    self.toggle_star(ctx, &key, &title);
                } else {
                    self.dialog_error(ctx, "Select or open an issue or merge request to star");
                }
            }
            Action::SaveView => {
                if ctx.state.kind().is_none() { self.dialog_error(ctx, "Save views from Issues or Merge Requests"); }
                else { self.show_dialog(ctx, DialogKind::SaveView, "Save as a tab", "The current filter becomes a persistent view", vec![FormField::new("Tab name", "", false)]); }
            }
            Action::RenameView | Action::DeleteView => {
                if let Some(index) = ctx
                    .state
                    .config
                    .active_tab
                    .checked_sub(4)
                    .filter(|i| *i < ctx.state.config.views.len())
                {
                    if let Some(view) = ctx.state.config.views.get(index) {
                        let name = view.name.clone();
                        if matches!(action, Action::RenameView) {
                            self.show_dialog(ctx, DialogKind::RenameView, "Rename saved tab", "Enter saves · Esc cancels", vec![FormField::new("Tab name", &name, false)]);
                        } else { self.show_dialog(ctx, DialogKind::Confirm(Confirmation::DeleteView(index)), "Remove saved tab?", "Only the local view is removed, not GitLab data", vec![]); }
                    }
                } else { self.dialog_error(ctx, "The built-in and Starred tabs cannot be renamed or removed"); }
            }
            Action::AddProject => self.show_dialog(ctx, DialogKind::AddProject, "Add existing project", "Search by name · Enter chooses a suggestion, then adds · Full paths and IDs also work · Blank alias keeps GitLab's short name", vec![FormField::lookup("Project", "", Completion::new(LookupKind::Projects, 0, false)), FormField::new("Short alias (optional)", "", false)]),
            Action::RemoveProject => {
                let project = ctx.state.config.project_route.and_then(|id| ctx.state.project(id).cloned())
                    .or_else(|| {
                        (ctx.state.config.active_tab == 1)
                            .then(|| ctx.state.config.projects.get(ctx.state.scroll.selected).cloned())
                            .flatten()
                    });
                if let Some(project) = project {
                    let id = project.id;
                    let name = project.name().to_owned();
                    self.show_dialog(
                        ctx,
                        DialogKind::Confirm(Confirmation::RemoveProject(id)),
                        "Forget this project?",
                        &format!("{name} · GitLab project and issues are NOT deleted"),
                        vec![],
                    );
                } else {
                    self.dialog_error(ctx, "Open a project in the Projects tab first");
                }
            }
            Action::NewIssue | Action::NewMergeRequest => {
                let project = ctx.state.config.route.as_ref().map(|k| k.project)
                    .or_else(|| if ctx.state.config.active_tab == 1 { ctx.state.config.projects.get(ctx.state.scroll.selected).map(|p| p.id) } else { ctx.state.visible_item_at(ctx.state.scroll.selected).map(|i| i.key.project) })
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
                let job = ctx.state.selected_job().cloned();
                if let Some(job) = job.filter(Job::retryable) {
                    let project = ctx.state.current_jobs().map_or(0, |(project, _)| project);
                    self.show_dialog(ctx, DialogKind::Confirm(Confirmation::Mutate(Mutation::RetryJob { project, job: job.id })), "Retry pipeline job?", &format!("{} · this consumes GitLab runner resources", job.name), vec![]);
                } else { self.dialog_error(ctx, "Select a finished, retryable job first"); }
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
        if matches!(dialog.kind, DialogKind::Onboarding) {
            return self.save_setup(ctx);
        }
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
            DialogKind::Onboarding => unreachable!("handled above"),
            DialogKind::Commands => {
                if let Some((_, action)) = ctx.state.command_options().get(selected).copied() {
                    self.close_dialog(ctx);
                    return self.action(ctx, action);
                }
                return Update::none();
            }
            DialogKind::Themes => {
                if let Some(theme) = crate::config::THEMES.get(selected) {
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
                ctx.state.navigation_restoring.remove(&key);
                ctx.state.navigation_routes_restoring.remove(&key);
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
                    // The Starred tab sits after the saved views; this new view takes its index.
                    if let Some(starred) = ctx.state.config.starred_tab() {
                        self.forget_tab(ctx, starred);
                    }
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
                if ctx.state.config.project_route == Some(id) {
                    ctx.state.config.project_route = None;
                    if ctx.state.config.active_tab == 1 {
                        ctx.state.scope = Scope::List;
                        ctx.state.config.section = None;
                        ctx.state.section_cursor = 0;
                        ctx.state.content_offset = 0;
                    }
                }
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
                    shift_tab_map(&mut ctx.state.navigation_selections, removed);
                    ctx.state.navigation_restoring =
                        shift_tab_set(&ctx.state.navigation_restoring, removed);
                    ctx.state.navigation_routes_restoring =
                        shift_tab_set(&ctx.state.navigation_routes_restoring, removed);
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
        project_route: state.config.project_route,
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
    let tab_count = config.tab_count();
    config.tab_states.retain(|key, tab| {
        if !key.parse::<usize>().is_ok_and(|index| index < tab_count) {
            return false;
        }
        if tab.route.as_ref().is_some_and(|route| {
            !config
                .projects
                .iter()
                .any(|p| p.id == route.project && p.kind_visible(route.kind))
        }) {
            *tab = TabState {
                list_offset: tab.list_offset,
                project_route: tab
                    .project_route
                    .filter(|id| config.projects.iter().any(|p| p.id == *id)),
                ..TabState::default()
            };
        }
        if tab
            .project_route
            .is_some_and(|id| !config.projects.iter().any(|p| p.id == id))
        {
            tab.project_route = None;
            if tab.route.is_none() {
                tab.section = None;
                tab.section_cursor = 0;
                tab.field = 0;
                tab.content_offset = 0;
            }
        }
        if let Some(route) = &tab.route {
            let last = if route.kind == ItemKind::MergeRequest {
                4
            } else {
                2
            };
            tab.section = tab.section.map(|section| section.min(last));
            tab.section_cursor = tab.section_cursor.min(last);
        } else if tab.project_route.is_some() {
            tab.section = tab.section.map(|_| 0);
            tab.section_cursor = 0;
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

fn shift_tab_set(set: &HashSet<String>, removed: usize) -> HashSet<String> {
    let mut map: BTreeMap<_, _> = set.iter().map(|key| (key.clone(), ())).collect();
    shift_tab_map(&mut map, removed);
    map.into_keys().collect()
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
