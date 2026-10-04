//! Text tokens keep their display values; only explicitly chosen identities become API IDs.
use super::*;
use std::ops::Range;

pub struct Completion {
    pub kind: LookupKind,
    pub project: u64,
    pub multiple: bool,
    pub workspace_only: bool,
    pub resolved: HashMap<String, u64>,
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
            resolved: HashMap::new(),
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
                    .insert(option.value.trim().to_owned(), option.id);
            }
        }
    }

    pub fn api_value(&self, text: &str) -> Result<String, String> {
        let tokens: Vec<_> = if self.multiple {
            text.split(',').collect()
        } else {
            vec![text]
        };
        let mut ids = Vec::new();
        for token in tokens.into_iter().map(str::trim).filter(|s| !s.is_empty()) {
            let id = self.resolved.get(token).copied()
                .or_else(|| token.parse::<u64>().ok().filter(|id| *id > 0))
                .ok_or_else(|| "Choose a lookup suggestion for each value before submitting (or enter a numeric ID).".to_owned())?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids
            .into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(","))
    }

    pub fn accept(&mut self, input: &mut TextInput, index: usize) -> bool {
        let Some(option) = self.options.get(index).cloned().filter(|option| {
            self.open && !self.pending && option.id > 0 && !option.value.trim().is_empty()
        }) else {
            return false;
        };
        let range = token_range(input.text(), input.cursor(), self.multiple);
        let mut text = input.text().to_owned();
        let start = range.start;
        text.replace_range(range, &option.value);
        *input = TextInput::new(text);
        input.set_cursor(start + option.value.len());
        self.resolved
            .insert(option.value.trim().to_owned(), option.id);
        self.dismiss();
        true
    }

    pub fn dismiss(&mut self) {
        self.open = false;
        self.pending = false;
        self.options.clear();
        self.query = None;
        self.error = None;
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
        let Some(c) = &mut field.completion else {
            return Update::none();
        };
        let range = token_range(field.input.text(), field.input.cursor(), c.multiple);
        let query = field.input.text()[range].to_owned();
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
                    c.options = options
                        .into_iter()
                        .filter(|option| option.id > 0 && !option.value.trim().is_empty())
                        .take(20)
                        .collect();
                    c.selected = 0;
                    if c.cache.len() >= 32 {
                        c.cache.clear();
                    }
                    c.cache
                        .insert(c.query.clone().unwrap_or_default(), c.options.clone());
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
            LookupKind::Milestones | LookupKind::Iterations => {
                let (id, label) = if c.kind == LookupKind::Milestones {
                    (item.milestone_id, &item.milestone)
                } else {
                    (item.iteration_id, &item.iteration)
                };
                id.map(|id| LookupOption {
                    id,
                    label: label.clone(),
                    value: label.clone(),
                    description: format!("Demo · ID {id}"),
                })
                .into_iter()
                .collect()
            }
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
            description: String::new(),
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
        c.resolved.insert("@one".into(), 42);
        assert_eq!(c.api_value(" @one, 7, @one, ").unwrap(), "42,7");
        assert_eq!(c.api_value("  , ").unwrap(), "");
        assert!(c.api_value("@one, missing").is_err());
        assert!(c.api_value("0").is_err());
        c.multiple = false;
        c.resolved.insert("Release, two".into(), 12);
        assert_eq!(c.api_value("Release, two").unwrap(), "12");
        c.resolved.insert("2026".into(), 50);
        assert_eq!(c.api_value("2026").unwrap(), "50");
    }
}
