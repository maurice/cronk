//! Text tokens keep their display values; only explicitly chosen identities become API values.
use super::*;
use std::ops::Range;

pub struct Completion {
    pub kind: LookupKind,
    pub project: u64,
    pub multiple: bool,
    pub workspace_only: bool,
    pub current_iteration: Option<CurrentIteration>,
    pub resolved: HashMap<String, String>,
    pub swatches: HashMap<String, Label>,
    pub options: Vec<LookupOption>,
    pub selected: usize,
    pub open: bool,
    pub pending: bool,
    pub error: Option<String>,
    pub query: Option<String>,
    pub epoch: u64,
    pub due: Duration,
    pub cache: HashMap<String, Vec<LookupOption>>,
}

impl Completion {
    pub fn new(kind: LookupKind, project: u64, multiple: bool) -> Self {
        Self {
            kind,
            project,
            multiple,
            workspace_only: false,
            current_iteration: None,
            resolved: HashMap::new(),
            swatches: HashMap::new(),
            options: vec![],
            selected: 0,
            open: false,
            pending: false,
            error: None,
            query: None,
            epoch: 0,
            due: Duration::ZERO,
            cache: HashMap::new(),
        }
    }

    pub fn seed(&mut self, options: impl IntoIterator<Item = LookupOption>) {
        for option in options {
            if option.id > 0 && !option.value.trim().is_empty() {
                self.resolved
                    .insert(option.value.trim().to_owned(), option.api_value.clone());
            }
            if !option.color.is_empty() {
                self.swatches.insert(
                    option.value.trim().to_owned(),
                    Label {
                        name: option.value.trim().to_owned(),
                        color: option.color.clone(),
                        text_color: option.text_color.clone(),
                    },
                );
            }
        }
    }

    pub fn api_value(&self, text: &str) -> Result<String, String> {
        let tokens: Vec<_> = if self.multiple {
            text.split(',').collect()
        } else {
            vec![text]
        };
        let mut values = Vec::new();
        for token in tokens.into_iter().map(str::trim).filter(|s| !s.is_empty()) {
            if self.kind == LookupKind::Iterations
                && is_current_iteration_value(token)
                && self.current_iteration.is_none()
            {
                return Err("Current iteration is unavailable for this project right now.".into());
            }
            let value = self
                .resolved
                .get(token)
                .cloned()
                .or_else(|| {
                    (self.kind == LookupKind::Iterations)
                        .then(|| {
                            self.current_iteration
                                .as_ref()
                                .filter(|_| is_current_iteration_value(token))
                                .map(|iteration| iteration.id.to_string())
                        })
                        .flatten()
                })
                .or_else(|| {
                    (self.kind == LookupKind::Labels && self.swatches.contains_key(token))
                        .then(|| token.to_owned())
                })
                .or_else(|| {
                    (self.kind != LookupKind::Labels)
                        .then(|| token.parse::<u64>().ok().filter(|id| *id > 0))
                        .flatten()
                        .map(|id| id.to_string())
                })
                .ok_or_else(|| {
                    if self.kind == LookupKind::Labels {
                        "Choose a known label suggestion for each value before submitting."
                            .to_owned()
                    } else {
                        "Choose a lookup suggestion for each value before submitting (or enter a numeric ID).".to_owned()
                    }
                })?;
            if !values.contains(&value) {
                values.push(value);
            }
        }
        Ok(values.join(","))
    }

    pub fn accept_text(
        &mut self,
        text: &str,
        cursor: usize,
        index: usize,
    ) -> Option<(String, usize)> {
        let option = self.options.get(index).cloned().filter(|option| {
            self.open && !self.pending && option.id > 0 && !option.value.trim().is_empty()
        })?;
        let range = token_range(text, cursor, self.multiple);
        let mut next = text.to_owned();
        let start = range.start;
        next.replace_range(range, &option.value);
        self.resolved
            .insert(option.value.trim().to_owned(), option.api_value.clone());
        if !option.color.is_empty() {
            self.swatches.insert(
                option.value.trim().to_owned(),
                Label {
                    name: option.value.trim().to_owned(),
                    color: option.color.clone(),
                    text_color: option.text_color.clone(),
                },
            );
        }
        self.dismiss();
        Some((next, start + option.value.len()))
    }

    pub fn accept(&mut self, input: &mut TextInput, index: usize) -> bool {
        let Some((text, cursor)) = self.accept_text(input.text(), input.cursor(), index) else {
            return false;
        };
        *input = TextInput::new(text);
        input.set_cursor(cursor);
        true
    }

    pub fn dismiss(&mut self) {
        self.open = false;
        self.pending = false;
        self.options.clear();
        self.query = None;
        self.error = None;
    }

    /// `raw` plus the symbolic Current iteration first, replacing GitLab's own entry for it.
    fn with_current(&self, raw: &[LookupOption], query: &str) -> Vec<LookupOption> {
        let mut options = raw.to_vec();
        if let Some(current) = self.current_option().filter(|option| {
            option.label.to_lowercase().contains(query)
                || option.value.to_lowercase().contains(query)
                || option.description.to_lowercase().contains(query)
        }) {
            options.retain(|option| option.id != current.id);
            options.insert(0, current);
        }
        options.truncate(20);
        options
    }

    fn current_option(&self) -> Option<LookupOption> {
        self.current_iteration
            .as_ref()
            .map(LookupOption::current_iteration)
    }
}

/// The comma-delimited token containing the caret, with surrounding whitespace excluded.
/// Single-valued titles can contain commas. UTF-8 indices always come from the input editor.
pub fn token_range(text: &str, cursor: usize, multiple: bool) -> Range<usize> {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let start = if multiple {
        text[..cursor].rfind(',').map_or(0, |i| i + 1)
    } else {
        0
    };
    let end = if multiple {
        text[cursor..].find(',').map_or(text.len(), |i| cursor + i)
    } else {
        text.len()
    };
    let raw = &text[start..end];
    let trimmed = raw.trim();
    let left = start + raw.len() - raw.trim_start().len();
    left..left + trimmed.len()
}

impl Cronk {
    pub(super) fn schedule_lookup(&self, ctx: &mut Context<Self>, index: usize) -> Update {
        let now = ctx.elapsed();
        let Some(dialog) = &mut ctx.state.dialog else {
            return Update::none();
        };
        if dialog.selected != index {
            return Update::none();
        }
        let Some(field) = dialog.fields.get_mut(index) else {
            return Update::none();
        };
        let text = field.value().to_owned();
        let cursor = field.cursor();
        let Some(c) = &mut field.completion else {
            return Update::none();
        };
        let range = token_range(&text, cursor, c.multiple);
        let query = text[range].to_owned();
        if c.open && c.error.is_none() && c.query.as_ref() == Some(&query) {
            return Update::full();
        }
        ctx.state.lookup_epoch = ctx.state.lookup_epoch.wrapping_add(1);
        c.epoch = ctx.state.lookup_epoch;
        c.query = Some(query);
        c.open = true;
        c.pending = true;
        c.error = None;
        c.options.clear();
        c.selected = 0;
        c.due = now + Duration::from_millis(300);
        let epoch = c.epoch;
        Update::with_command(Command::after(
            Duration::from_millis(300),
            move |link: tui_lipan::CommandLink<Msg>| link.send(Msg::LookupStart(epoch, index)),
        ))
    }

    pub(super) fn start_lookup(&self, ctx: &mut Context<Self>, epoch: u64, index: usize) -> Update {
        let Some(c) = active_completion(&ctx.state, index)
            .filter(|c| c.open && c.pending && c.epoch == epoch)
        else {
            return Update::none();
        };
        if ctx.elapsed() < c.due || ctx.state.lookup_inflight.is_some() {
            return Update::none();
        }
        let query = c.query.clone().unwrap_or_default();
        if let Some(options) = c.cache.get(&query) {
            ctx.link()
                .send(Msg::LookupLoaded(epoch, index, Ok(options.clone())));
            return Update::none();
        }
        if c.workspace_only || ctx.state.demo {
            let options = local_options(&ctx.state, c)
                .into_iter()
                .filter(|option| {
                    let query = query.to_lowercase();
                    option.label.to_lowercase().contains(&query)
                        || option.value.to_lowercase().contains(&query)
                        || option.description.to_lowercase().contains(&query)
                })
                .take(20)
                .collect();
            ctx.link()
                .send(Msg::LookupLoaded(epoch, index, Ok(options)));
            return Update::none();
        }
        if ctx.elapsed() < ctx.state.blocked_until {
            ctx.link().send(Msg::LookupLoaded(
                epoch,
                index,
                Err("GitLab backoff is active; edit the query to retry later.".into()),
            ));
            return Update::none();
        }
        let Some(api) = self.api.clone() else {
            return Update::none();
        };
        let (project, kind) = (c.project, c.kind);
        // Serialize lookups, coalescing intermediate queries while a slow request is in flight.
        ctx.state.lookup_inflight = Some(epoch);
        Update::with_command(ctx.link().command(move |link| {
            link.send(Msg::LookupLoaded(
                epoch,
                index,
                api.lookup(project, kind, &query).map_err(|e| e.to_string()),
            ));
        }))
    }

    pub(super) fn lookup_loaded(
        &self,
        ctx: &mut Context<Self>,
        epoch: u64,
        index: usize,
        result: Result<Vec<LookupOption>, String>,
    ) -> Update {
        if ctx.state.lookup_inflight == Some(epoch) {
            ctx.state.lookup_inflight = None;
            if let Err(error) = &result
                && (error.contains("HTTP 429")
                    || error.contains("HTTP 5")
                    || error.contains("Retry-After:"))
            {
                self.network_error(ctx, error.clone());
            }
        }
        if let Some(c) =
            active_completion_mut(&mut ctx.state, index).filter(|c| c.open && c.epoch == epoch)
        {
            c.pending = false;
            match result {
                Ok(options) => {
                    let query = c.query.clone().unwrap_or_default().to_lowercase();
                    // Cache what GitLab returned; the synthetic Current entry is added per
                    // display, otherwise every cache hit would stack another copy.
                    let raw: Vec<_> = options
                        .into_iter()
                        .filter(|option| option.id > 0 && !option.value.trim().is_empty())
                        .collect();
                    c.options = c.with_current(&raw, &query);
                    for option in &c.options {
                        if !option.color.is_empty() {
                            c.swatches.insert(
                                option.value.trim().to_owned(),
                                Label {
                                    name: option.value.trim().to_owned(),
                                    color: option.color.clone(),
                                    text_color: option.text_color.clone(),
                                },
                            );
                        }
                    }
                    c.selected = 0;
                    if c.cache.len() >= 32 {
                        c.cache.clear();
                    }
                    c.cache.insert(c.query.clone().unwrap_or_default(), raw);
                }
                Err(error) => {
                    c.error = Some(error);
                }
            }
        }
        if let Some(dialog) = &ctx.state.dialog
            && let Some(c) = active_completion(&ctx.state, dialog.selected)
            && c.open
            && c.pending
        {
            ctx.link().send(Msg::LookupStart(c.epoch, dialog.selected));
        }
        Update::full()
    }
}

pub(super) fn active_completion(state: &State, index: usize) -> Option<&Completion> {
    let dialog = state.dialog.as_ref().filter(|d| d.selected == index)?;
    dialog.fields.get(index)?.completion.as_ref()
}
pub(super) fn active_completion_mut(state: &mut State, index: usize) -> Option<&mut Completion> {
    let dialog = state.dialog.as_mut().filter(|d| d.selected == index)?;
    dialog.fields.get_mut(index)?.completion.as_mut()
}

fn local_options(state: &State, c: &Completion) -> Vec<LookupOption> {
    if c.kind == LookupKind::Projects {
        return state
            .config
            .projects
            .iter()
            .map(LookupOption::project)
            .collect();
    }
    let mut options = Vec::new();
    let mut seen = HashSet::new();
    for item in state
        .items
        .iter()
        .chain(state.details.iter().map(|d| &d.item))
        .filter(|i| i.key.project == c.project)
    {
        let candidates: Vec<LookupOption> = match c.kind {
            LookupKind::Users => item
                .assignees
                .iter()
                .chain(&item.reviewers)
                .chain(std::iter::once(&item.author))
                .map(LookupOption::user)
                .collect(),
            LookupKind::Milestones | LookupKind::Iterations | LookupKind::Epics => {
                let (id, label) = match c.kind {
                    LookupKind::Milestones => (item.milestone_id, &item.milestone),
                    LookupKind::Iterations => (item.iteration_id, &item.iteration),
                    LookupKind::Epics => (item.epic_id, &item.epic),
                    _ => unreachable!(),
                };
                id.map(|id| LookupOption {
                    id,
                    label: label.clone(),
                    value: label.clone(),
                    api_value: id.to_string(),
                    description: "Demo".into(),
                    color: String::new(),
                    text_color: String::new(),
                })
                .into_iter()
                .collect()
            }
            LookupKind::Labels => item.labels.iter().map(LookupOption::label).collect(),
            LookupKind::Projects => unreachable!(),
        };
        for option in candidates {
            if seen.insert(option.id) {
                options.push(option);
            }
        }
    }
    options
}

impl FormField {
    pub fn lookup(label: &str, value: &str, completion: Completion) -> Self {
        Self {
            completion: Some(completion),
            ..Self::new(label, value, false)
        }
    }
    pub fn api_value(&self) -> Result<String, String> {
        self.completion.as_ref().map_or_else(
            || Ok(self.value().to_owned()),
            |c| c.api_value(self.value()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comma_tokens_trim_spaces_and_preserve_unicode_and_neighbors() {
        let text = "@one,  Élise  , @three";
        let range = token_range(text, "@one,  Éli".len(), true);
        assert_eq!(&text[range.clone()], "Élise");
        let mut completion = Completion::new(LookupKind::Users, 1, true);
        completion.options = vec![LookupOption {
            id: 2,
            label: "Élise".into(),
            value: "@elise".into(),
            api_value: "2".into(),
            description: String::new(),
            color: String::new(),
            text_color: String::new(),
        }];
        completion.open = true;
        let mut input = TextInput::new(text);
        input.set_cursor("@one,  Éli".len());
        assert!(completion.accept(&mut input, 0));
        assert_eq!(input.text(), "@one,  @elise  , @three");
        assert_eq!(input.cursor(), "@one,  @elise".len());
        assert_eq!(&"a,   "[token_range("a,   ", 5, true)], "");
        assert_eq!(
            &"Release, two"[token_range("Release, two", 8, false)],
            "Release, two"
        );
    }

    #[test]
    fn resolve_only_chosen_names_or_ids_trim_deduplicate_and_clear() {
        let mut c = Completion::new(LookupKind::Users, 1, true);
        c.resolved.insert("@one".into(), "42".into());
        assert_eq!(c.api_value(" @one, 7, @one, ").unwrap(), "42,7");
        assert_eq!(c.api_value("  , ").unwrap(), "");
        assert!(c.api_value("@one, missing").is_err());
        assert!(c.api_value("0").is_err());
        c.multiple = false;
        c.resolved.insert("Release, two".into(), "12".into());
        assert_eq!(c.api_value("Release, two").unwrap(), "12");
        c.resolved.insert("2026".into(), "50".into());
        assert_eq!(c.api_value("2026").unwrap(), "50");
    }

    #[test]
    fn current_iteration_is_listed_once_however_often_results_are_shown() {
        let option = |id, label: &str| LookupOption {
            id,
            label: label.into(),
            value: label.into(),
            api_value: id.to_string(),
            description: String::new(),
            color: String::new(),
            text_color: String::new(),
        };
        let mut c = Completion::new(LookupKind::Iterations, 1, false);
        c.current_iteration = Some(CurrentIteration {
            id: 2,
            title: "28 Sep 2026 - 11 Oct 2026 (Current)".into(),
            description: "Current iteration · acme".into(),
        });
        let raw = vec![
            option(1, "14 Sep 2026 - 27 Sep 2026"),
            option(2, "28 Sep 2026 - 11 Oct 2026 (Current)"),
        ];
        // Each cache hit re-decorates the same cached list.
        for _ in 0..3 {
            let shown = c.with_current(&raw, "current");
            assert_eq!(shown.iter().filter(|o| o.id == 2).count(), 1);
            assert_eq!(shown[0].id, 2);
            assert_eq!(shown[0].description, "Current iteration · acme");
        }
        assert_eq!(c.with_current(&raw, "").len(), 2);
    }

    #[test]
    fn current_iteration_resolves_case_insensitively_and_reports_missing_current() {
        let mut c = Completion::new(LookupKind::Iterations, 1, false);
        c.current_iteration = Some(CurrentIteration {
            id: 1200,
            title: "Sprint 12".into(),
            description: "Sprint 12 · Demo · ID 1200".into(),
        });
        assert_eq!(c.api_value("Current").unwrap(), "1200");
        assert_eq!(c.api_value("current").unwrap(), "1200");
        c.current_iteration = None;
        assert!(
            c.api_value("Current")
                .unwrap_err()
                .contains("Current iteration is unavailable")
        );
    }
}
