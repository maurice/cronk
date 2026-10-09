use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    #[default]
    Issue,
    MergeRequest,
}

impl ItemKind {
    pub fn segment(self) -> &'static str {
        match self {
            Self::Issue => "issues",
            Self::MergeRequest => "merge_requests",
        }
    }
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Issue => "#",
            Self::MergeRequest => "!",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub id: u64,
    pub path: String,
    /// Local display override. Empty falls back to the GitLab path.
    /// New projects may start with GitLab's short name when the add dialog leaves this blank.
    pub alias: String,
    pub visible: bool,
    /// Local collection preferences, preserved when the master switch is off.
    pub issues_visible: bool,
    pub merge_requests_visible: bool,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: 0,
            path: String::new(),
            alias: String::new(),
            visible: false,
            issues_visible: true,
            merge_requests_visible: true,
        }
    }
}

impl Project {
    pub fn kind_visible(&self, kind: ItemKind) -> bool {
        self.visible
            && match kind {
                ItemKind::Issue => self.issues_visible,
                ItemKind::MergeRequest => self.merge_requests_visible,
            }
    }

    pub fn effectively_visible(&self) -> bool {
        self.visible && (self.issues_visible || self.merge_requests_visible)
    }

    pub fn visibility_status(&self) -> &'static str {
        if !self.effectively_visible() {
            "hidden"
        } else if self.issues_visible && self.merge_requests_visible {
            "visible"
        } else {
            "partially_visible"
        }
    }

    pub fn name(&self) -> &str {
        if self.alias.is_empty() {
            &self.path
        } else {
            &self.alias
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemKey {
    pub project: u64,
    pub iid: u64,
    pub kind: ItemKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Label {
    pub name: String,
    pub color: String,
    pub text_color: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct User {
    pub id: u64,
    pub username: String,
    pub name: String,
    /// GitLab exposes this only when the user has chosen to make it public.
    pub public_email: Option<String>,
    pub bot: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LookupKind {
    Users,
    Milestones,
    Iterations,
    Epics,
    Projects,
    Labels,
}

pub const CURRENT_ITERATION_NAME: &str = "Current";

pub fn is_current_iteration_value(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case(CURRENT_ITERATION_NAME)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CurrentIteration {
    pub id: u64,
    pub title: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookupOption {
    pub id: u64,
    pub label: String,
    pub value: String,
    pub api_value: String,
    pub description: String,
    pub color: String,
    pub text_color: String,
}

impl LookupOption {
    pub fn user(user: &User) -> Self {
        Self {
            id: user.id,
            label: user.name.clone(),
            value: format!("@{}", user.username),
            api_value: user.id.to_string(),
            description: format!("@{} · ID {}", user.username, user.id),
            color: String::new(),
            text_color: String::new(),
        }
    }
    pub fn project(project: &Project) -> Self {
        Self {
            id: project.id,
            label: project.name().to_owned(),
            value: project.path.clone(),
            api_value: project.id.to_string(),
            description: format!("{} · ID {}", project.path, project.id),
            color: String::new(),
            text_color: String::new(),
        }
    }
    pub fn label(label: &Label) -> Self {
        Self {
            id: lookup_id(&label.name),
            label: label.name.clone(),
            value: label.name.clone(),
            api_value: label.name.clone(),
            description: "Known label".into(),
            color: label.color.clone(),
            text_color: label.text_color.clone(),
        }
    }

    pub fn current_iteration(iteration: &CurrentIteration) -> Self {
        Self {
            id: iteration.id,
            label: iteration.title.clone(),
            value: iteration.title.clone(),
            api_value: iteration.id.to_string(),
            description: iteration.description.clone(),
            color: String::new(),
            text_color: String::new(),
        }
    }
}

fn lookup_id(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish().max(1)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pipeline {
    pub id: u64,
    pub status: String,
    pub web_url: String,
    /// push, schedule, merge_request_event, web, api, trigger, pipeline, parent_pipeline, …
    pub source: Option<String>,
    /// Branch/tag, or `refs/merge-requests/<iid>/head` for MR pipelines.
    #[serde(rename = "ref")]
    pub ref_name: Option<String>,
    pub sha: Option<String>,
    /// `workflow:name`; GitLab sends `null` when unset.
    pub name: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    // Only on the single-pipeline endpoint and on MR `head_pipeline`.
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// Run time in seconds; null while running.
    pub duration: Option<u64>,
    pub queued_duration: Option<u64>,
    pub user: Option<User>,
}

impl Pipeline {
    pub fn active(&self) -> bool {
        matches!(
            self.status.as_str(),
            "created" | "waiting_for_resource" | "preparing" | "pending" | "running"
        )
    }

    /// IID for `refs/merge-requests/<iid>/head|merge` refs.
    pub fn merge_request_iid(&self) -> Option<u64> {
        self.ref_name
            .as_deref()?
            .strip_prefix("refs/merge-requests/")?
            .split('/')
            .next()?
            .parse()
            .ok()
    }

    /// Short ref for display: `!<iid>` for MR pipelines, otherwise the branch/tag.
    pub fn ref_label(&self) -> String {
        match self.merge_request_iid() {
            Some(iid) => format!("!{iid}"),
            None => self.ref_name.clone().unwrap_or_default(),
        }
    }

    /// Humanised trigger source. Unknown values pass through.
    pub fn source_label(&self) -> String {
        match self.source.as_deref().unwrap_or("") {
            "merge_request_event" => "merge request".into(),
            "parent_pipeline" => "parent pipeline".into(),
            "external_pull_request_event" => "external PR".into(),
            "ondemand_dast_scan" | "ondemand_dast_validation" => "on-demand scan".into(),
            "security_orchestration_policy" => "security policy".into(),
            other => other.replace('_', " "),
        }
    }

    /// Elapsed run time in seconds: wall time since start (or creation) while active,
    /// otherwise GitLab's fixed `duration`. Missing timestamps fall back to `duration`.
    pub fn elapsed_secs(&self, now_unix: i64) -> Option<u64> {
        if self.active()
            && let Some(start) = self
                .started_at
                .as_deref()
                .and_then(crate::dates::parse_unix)
                .or_else(|| {
                    self.created_at
                        .as_deref()
                        .and_then(crate::dates::parse_unix)
                })
        {
            return Some(now_unix.saturating_sub(start).max(0) as u64);
        }
        self.duration
    }

    /// Shared list/detail timing: `3m 12s · 2 min ago`, omitting unavailable facts.
    pub fn timing_label(&self, now_unix: i64) -> String {
        let mut times = Vec::new();
        if let Some(secs) = self.elapsed_secs(now_unix) {
            times.push(format_duration(secs));
        }
        if let Some(updated) = self.updated_at.as_deref().filter(|u| !u.is_empty()) {
            times.push(crate::dates::format_relative(updated, now_unix));
        }
        times.join(" · ")
    }
}

/// `3m 12s`, `1h 04m`, `45s`.
pub fn format_duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkItem {
    pub key: ItemKey,
    pub title: String,
    pub description: String,
    pub state: String,
    pub labels: Vec<Label>,
    pub author: User,
    pub assignees: Vec<User>,
    pub reviewers: Vec<User>,
    pub milestone: String,
    pub milestone_id: Option<u64>,
    pub iteration: String,
    pub iteration_id: Option<u64>,
    /// GitLab REST issue parent (epic); absent on installations without epics.
    pub epic: String,
    pub epic_id: Option<u64>,
    pub source_branch: String,
    pub target_branch: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
    pub web_url: String,
    pub draft: bool,
    pub pipeline: Option<Pipeline>,
    /// A detail request has checked this MR's pipeline, including a confirmed absence.
    /// Persisted so terminal MRs do not trigger the same request on every visit.
    pub pipeline_metrics_fetched: bool,
    // None means GitLab has not supplied this information, not zero discussions.
    pub unresolved: Option<usize>,
}

impl WorkItem {
    /// Pipeline run time plus the most useful age for the merge-request list.
    /// For terminal MRs, report when the MR merged/closed rather than when its
    /// pipeline last changed; open MRs keep the pipeline's last-update age.
    pub fn list_pipeline_timing(&self, now_unix: i64) -> String {
        let terminal = if self.key.kind == ItemKind::MergeRequest {
            match self.state.as_str() {
                "merged" => self.merged_at.as_deref().map(|date| ("merged", date)),
                "closed" => self.closed_at.as_deref().map(|date| ("closed", date)),
                _ => None,
            }
        } else {
            None
        };
        if let Some((event, date)) = terminal.filter(|(_, date)| !date.is_empty()) {
            let mut times = Vec::new();
            if let Some(secs) = self
                .pipeline
                .as_ref()
                .and_then(|p| p.elapsed_secs(now_unix))
            {
                times.push(format_duration(secs));
            }
            times.push(format!(
                "{event} {}",
                crate::dates::format_relative(date, now_unix)
            ));
            return times.join(" · ");
        }
        self.pipeline
            .as_ref()
            .map_or_else(String::new, |pipeline| pipeline.timing_label(now_unix))
    }

    /// Retain immutable terminal-MR metrics when a list refresh omits the
    /// detail-only fields that were fetched previously.
    pub fn preserve_terminal_pipeline_metrics(&mut self, previous: &Self) {
        if self.key.kind != ItemKind::MergeRequest
            || !matches!(self.state.as_str(), "merged" | "closed")
            || self.state != previous.state
            || !matches!(previous.state.as_str(), "merged" | "closed")
            || !(previous.pipeline_metrics_fetched || previous.pipeline.is_some())
        {
            return;
        }
        if self.pipeline.is_none() {
            self.pipeline = previous.pipeline.clone();
        }
        if self.merged_at.is_none() {
            self.merged_at.clone_from(&previous.merged_at);
        }
        if self.closed_at.is_none() {
            self.closed_at.clone_from(&previous.closed_at);
        }
        self.pipeline_metrics_fetched = true;
    }

    pub fn dashboard_roles(&self, current_user: u64) -> Vec<&'static str> {
        if current_user == 0 {
            return Vec::new();
        }
        let mut roles = Vec::new();
        if self.author.id == current_user {
            roles.push("author");
        }
        if self.assignees.iter().any(|user| user.id == current_user) {
            roles.push("assignee");
        }
        if self.key.kind == ItemKind::MergeRequest
            && self.reviewers.iter().any(|user| user.id == current_user)
        {
            roles.push("reviewer");
        }
        roles
    }

    pub fn on_dashboard_for(&self, current_user: u64) -> bool {
        current_user != 0
            && (self.author.id == current_user
                || self.assignees.iter().any(|user| user.id == current_user)
                || (self.key.kind == ItemKind::MergeRequest
                    && self.reviewers.iter().any(|user| user.id == current_user)))
    }

    pub fn attention_rank(&self, current_user: u64) -> u8 {
        if self.state != "opened" {
            return 9;
        }
        if self.pipeline.as_ref().is_some_and(|p| p.status == "failed") {
            return 0;
        }
        if self.unresolved.is_some_and(|n| n > 0) {
            return 1;
        }
        if self.key.kind == ItemKind::MergeRequest
            && self
                .reviewers
                .iter()
                .any(|user| user.id == current_user && current_user != 0)
        {
            return 2;
        }
        if self.needs_reviewers(current_user) {
            return 3;
        }
        if self.is_merge_request()
            && !self.draft
            && self
                .pipeline
                .as_ref()
                .is_some_and(|p| p.status == "success")
        {
            return 4;
        }
        if self.is_merge_request()
            && self.pipeline.as_ref().is_some_and(|p| {
                matches!(
                    p.status.as_str(),
                    "created" | "pending" | "preparing" | "running" | "waiting_for_resource"
                )
            })
        {
            return 5;
        }
        if self.is_merge_request() && self.draft {
            return 6;
        }
        if self.is_merge_request() { 7 } else { 8 }
    }

    pub fn attention_reasons(&self, current_user: u64) -> Vec<String> {
        let mut reasons = Vec::new();
        if self.pipeline.as_ref().is_some_and(|p| p.status == "failed") {
            reasons.push("Pipeline failed".into());
        }
        if let Some(count) = self.unresolved.filter(|count| *count > 0) {
            reasons.push(format!("{count} unresolved threads"));
        }
        if self.key.kind == ItemKind::MergeRequest
            && self
                .reviewers
                .iter()
                .any(|user| user.id == current_user && current_user != 0)
        {
            reasons.push("Review assigned".into());
        }
        if self.needs_reviewers(current_user) {
            reasons.push("Reviewers needed".into());
        }
        if self.is_merge_request()
            && !self.draft
            && self
                .pipeline
                .as_ref()
                .is_some_and(|p| p.status == "success")
        {
            reasons.push("Checks passed".into());
        }
        if self.is_merge_request()
            && self.pipeline.as_ref().is_some_and(|p| {
                matches!(
                    p.status.as_str(),
                    "created" | "pending" | "preparing" | "running" | "waiting_for_resource"
                )
            })
        {
            reasons.push("Checks in progress".into());
        }
        if self.is_merge_request() && self.draft {
            reasons.push("Draft".into());
        }
        if reasons.is_empty() {
            reasons.push(if self.is_merge_request() {
                "Open merge request".into()
            } else {
                "Open issue".into()
            });
        }
        reasons
    }

    fn is_merge_request(&self) -> bool {
        self.key.kind == ItemKind::MergeRequest
    }

    fn needs_reviewers(&self, current_user: u64) -> bool {
        self.is_merge_request()
            && !self.draft
            && self.reviewers.is_empty()
            && current_user != 0
            && (self.author.id == current_user
                || self.assignees.iter().any(|user| user.id == current_user))
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;

    #[test]
    fn legacy_cached_pipelines_and_null_fields_decode() {
        let legacy: Pipeline =
            serde_json::from_str(r#"{"id":7,"status":"success","web_url":"u"}"#).unwrap();
        assert_eq!(legacy.source, None);
        assert_eq!(legacy.ref_label(), "");
        let nulls: Pipeline = serde_json::from_str(
            r#"{"id":8,"status":"running","web_url":"u","name":null,"ref":null,"user":null,"duration":null}"#,
        )
        .unwrap();
        assert_eq!(nulls.name, None);
        assert!(nulls.active());
    }

    #[test]
    fn pipeline_timing_labels_track_elapsed_and_relative_time() {
        let now = crate::dates::parse_unix("2026-09-28T12:00:00Z").unwrap();
        let mut pipeline = Pipeline {
            status: "running".into(),
            started_at: Some("2026-09-28T11:56:48Z".into()),
            updated_at: Some("2026-09-28T11:59:20Z".into()),
            duration: Some(90),
            ..Pipeline::default()
        };
        assert_eq!(pipeline.timing_label(now), "3m 12s · just now");
        assert_eq!(pipeline.timing_label(now + 10), "3m 22s · 1 min ago");
        pipeline.status = "success".into();
        assert_eq!(pipeline.timing_label(now), "1m 30s · just now");
        assert_eq!(pipeline.timing_label(now + 100), "1m 30s · 2 min ago");
    }

    #[test]
    fn pipeline_timing_labels_omit_missing_facts_and_fall_back_to_available_data() {
        let now = crate::dates::parse_unix("2026-09-28T12:00:00Z").unwrap();
        let mut pipeline = Pipeline::default();
        assert_eq!(pipeline.timing_label(now), "");
        pipeline.updated_at = Some("2026-09-28T11:58:00Z".into());
        assert_eq!(pipeline.timing_label(now), "2 min ago");
        pipeline.updated_at = Some(String::new());
        pipeline.duration = Some(45);
        assert_eq!(pipeline.timing_label(now), "45s");
        pipeline.status = "running".into();
        assert_eq!(pipeline.elapsed_secs(now), Some(45));
        pipeline.started_at = Some("invalid".into());
        pipeline.created_at = Some("2026-09-28T11:57:00Z".into());
        assert_eq!(pipeline.timing_label(now), "3m 00s");
        pipeline.started_at = Some("2026-09-28T12:01:00Z".into());
        assert_eq!(pipeline.timing_label(now), "0s");
    }

    #[test]
    fn terminal_merge_request_timing_uses_the_merge_or_close_timestamp() {
        let now = crate::dates::parse_unix("2026-09-28T12:00:00Z").unwrap();
        let mut merged = WorkItem {
            key: ItemKey {
                kind: ItemKind::MergeRequest,
                ..ItemKey::default()
            },
            state: "merged".into(),
            merged_at: Some("2026-09-26T12:00:00Z".into()),
            pipeline: Some(Pipeline {
                status: "success".into(),
                duration: Some(192),
                updated_at: Some("2026-09-26T11:58:00Z".into()),
                ..Pipeline::default()
            }),
            ..WorkItem::default()
        };
        assert_eq!(
            merged.list_pipeline_timing(now),
            "3m 12s · merged 2 days ago"
        );
        merged.state = "closed".into();
        merged.merged_at = None;
        merged.closed_at = Some("2026-09-27T12:00:00Z".into());
        merged.pipeline = None;
        assert_eq!(merged.list_pipeline_timing(now), "closed yesterday");
        let issue = WorkItem {
            key: ItemKey {
                kind: ItemKind::Issue,
                ..ItemKey::default()
            },
            state: "closed".into(),
            closed_at: Some("2026-09-27T12:00:00Z".into()),
            ..WorkItem::default()
        };
        assert_eq!(issue.list_pipeline_timing(now), "");
    }

    #[test]
    fn merge_request_refs_sources_and_elapsed_time() {
        let mut pipeline = Pipeline {
            ref_name: Some("refs/merge-requests/104/merge".into()),
            source: Some("merge_request_event".into()),
            status: "running".into(),
            created_at: Some("2026-09-28T11:50:00Z".into()),
            ..Pipeline::default()
        };
        let now = crate::dates::parse_unix("2026-09-28T12:00:00Z").unwrap();
        assert_eq!(pipeline.merge_request_iid(), Some(104));
        assert_eq!(pipeline.ref_label(), "!104");
        assert_eq!(pipeline.source_label(), "merge request");
        assert_eq!(
            pipeline.elapsed_secs(now),
            Some(600),
            "wall time since creation"
        );
        pipeline.started_at = Some("2026-09-28T11:55:00Z".into());
        assert_eq!(
            pipeline.elapsed_secs(now),
            Some(300),
            "start beats creation"
        );
        pipeline.status = "success".into();
        assert_eq!(
            pipeline.elapsed_secs(now),
            None,
            "finished without duration: unknown"
        );
        pipeline.duration = Some(3725);
        assert_eq!(
            format_duration(pipeline.elapsed_secs(now).unwrap()),
            "1h 02m"
        );
        assert_eq!(format_duration(192), "3m 12s");
        assert_eq!(format_duration(45), "45s");
        pipeline.ref_name = Some("main".into());
        pipeline.source = Some("parent_pipeline".into());
        assert_eq!(pipeline.ref_label(), "main");
        assert_eq!(pipeline.source_label(), "parent pipeline");
    }
}

#[cfg(test)]
mod dashboard_tests {
    use super::*;

    const ME: u64 = 1;

    #[test]
    fn user_profiles_deserialize_optional_public_email_and_bot_flag() {
        let profile: User = serde_json::from_value(serde_json::json!({
            "id": 7,
            "username": "opaque-bot-123",
            "name": "Deploy token",
            "public_email": "deploy@example.org",
            "bot": true
        }))
        .unwrap();
        assert_eq!(profile.public_email.as_deref(), Some("deploy@example.org"));
        assert!(profile.bot);

        let sparse: User = serde_json::from_value(serde_json::json!({
            "id": 8,
            "username": "human"
        }))
        .unwrap();
        assert_eq!(sparse.public_email, None);
        assert!(!sparse.bot);
    }

    fn mr() -> WorkItem {
        WorkItem {
            key: ItemKey {
                project: 1,
                iid: 1,
                kind: ItemKind::MergeRequest,
            },
            state: "opened".into(),
            author: User {
                id: ME,
                ..User::default()
            },
            ..WorkItem::default()
        }
    }

    #[test]
    fn dashboard_reasons_and_roles_are_distinct() {
        let mut item = mr();
        item.assignees.push(User {
            id: ME,
            ..User::default()
        });
        item.reviewers.push(User {
            id: ME,
            ..User::default()
        });
        item.unresolved = Some(2);
        item.pipeline = Some(Pipeline {
            status: "failed".into(),
            ..Pipeline::default()
        });

        assert_eq!(item.dashboard_roles(ME), ["author", "assignee", "reviewer"]);
        assert_eq!(
            item.attention_reasons(ME),
            ["Pipeline failed", "2 unresolved threads", "Review assigned"]
        );
    }

    #[test]
    fn urgency_puts_actionable_signals_before_waiting_and_drafts() {
        let mut failed = mr();
        failed.pipeline = Some(Pipeline {
            status: "failed".into(),
            ..Pipeline::default()
        });
        let mut discussions = mr();
        discussions.unresolved = Some(2);
        let mut review = mr();
        review.reviewers.push(User {
            id: ME,
            ..User::default()
        });
        let needs_reviewers = mr();
        let mut running = mr();
        running.reviewers.push(User {
            id: 2,
            ..User::default()
        });
        running.pipeline = Some(Pipeline {
            status: "running".into(),
            ..Pipeline::default()
        });
        let mut draft = mr();
        draft.draft = true;
        let issue = WorkItem {
            key: ItemKey {
                kind: ItemKind::Issue,
                ..mr().key
            },
            ..mr()
        };

        assert!(failed.attention_rank(ME) < discussions.attention_rank(ME));
        assert!(discussions.attention_rank(ME) < review.attention_rank(ME));
        assert!(review.attention_rank(ME) < needs_reviewers.attention_rank(ME));
        assert!(needs_reviewers.attention_rank(ME) < running.attention_rank(ME));
        assert!(running.attention_rank(ME) < draft.attention_rank(ME));
        assert!(draft.attention_rank(ME) < issue.attention_rank(ME));
        assert_eq!(needs_reviewers.attention_reasons(ME), ["Reviewers needed"]);
        assert_eq!(running.attention_reasons(ME), ["Checks in progress"]);
        assert_eq!(draft.attention_reasons(ME), ["Draft"]);
    }

    #[test]
    fn missing_discussion_count_is_unknown_and_missing_identity_does_not_match() {
        let mut item = mr();
        item.unresolved = None;
        item.author.id = 2;
        assert!(!item.dashboard_roles(0).contains(&"author"));
        assert!(!item.on_dashboard_for(0));
        assert_eq!(item.attention_reasons(ME), ["Open merge request"]);
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: u64,
    pub author: User,
    pub body: String,
    pub created_at: String,
    pub system: bool,
    pub resolvable: bool,
    pub resolved: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub status: String,
    pub allow_failure: bool,
    pub web_url: String,
    pub started_at: Option<String>,
}

impl Job {
    pub fn running(&self) -> bool {
        self.status == "running"
    }
    pub fn retryable(&self) -> bool {
        matches!(self.status.as_str(), "failed" | "canceled" | "success")
    }
    /// Failed in a way that fails the pipeline (not an allowed failure).
    pub fn failed_hard(&self) -> bool {
        self.status == "failed" && !self.allow_failure
    }
    /// Status used for colouring: allowed failures are a warning, not an error.
    pub fn display_status(&self) -> &str {
        if self.status == "failed" && self.allow_failure {
            STATUS_FAILED_ALLOWED
        } else {
            &self.status
        }
    }
}

/// Orders jobs by attention: hard failures, active work, upcoming automatic work,
/// canceled, success/allowed failure, skipped, then unstarted manual work.
/// Hard failures use earliest start first (missing times last); active, canceled
/// and completed jobs use newest start first. Other groups use stage order
/// (earliest-created stage first), then name. Unknown states stay with upcoming
/// work rather than being hidden among completed or manual jobs.
pub fn sort_jobs(jobs: &mut [Job]) {
    let mut stage_rank: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for job in jobs.iter() {
        let rank = stage_rank.entry(job.stage.clone()).or_insert(job.id);
        *rank = (*rank).min(job.id);
    }
    let priority = |job: &Job| match job.status.as_str() {
        "failed" if !job.allow_failure => 0,
        "running" | "preparing" | "canceling" | "waiting_for_callback" => 1,
        "created" | "pending" | "waiting_for_resource" | "scheduled" => 2,
        "canceled" => 3,
        "success" | "failed" => 4,
        "skipped" => 5,
        "manual" => 6,
        _ => 2,
    };
    let stage_order = |a: &Job, b: &Job| {
        stage_rank[&a.stage]
            .cmp(&stage_rank[&b.stage])
            .then_with(|| a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    };
    jobs.sort_by(|a, b| {
        let group = priority(a);
        group.cmp(&priority(b)).then_with(|| match group {
            0 => a
                .started_at
                .is_none()
                .cmp(&b.started_at.is_none())
                .then_with(|| a.started_at.cmp(&b.started_at))
                .then(a.id.cmp(&b.id)),
            1 | 3 | 4 => match (&a.started_at, &b.started_at) {
                (Some(x), Some(y)) => y.cmp(x).then(b.id.cmp(&a.id)),
                (None, None) => stage_order(a, b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
            },
            _ => stage_order(a, b),
        })
    });
}

/// Pseudo-statuses only used for colouring and tooltips. The text before ` (` is the label to show.
pub const STATUS_FAILED_ALLOWED: &str = "failed (allowed)";
pub const STATUS_SUCCESS_WARNING: &str = "success (allowed failures)";
pub const STATUS_RUNNING_FAILING: &str = "running (failing)";
pub const STATUS_RUNNING_WARNING: &str = "running (allowed failures)";

/// Aggregate pipeline status derived from its jobs; `reported` is GitLab's own status.
pub fn pipeline_status(jobs: &[Job], reported: &str) -> String {
    if jobs.is_empty() {
        return reported.to_owned();
    }
    // GitLab may know about failures we cannot see (e.g. downstream trigger jobs).
    let hard = reported == "failed" || jobs.iter().any(Job::failed_hard);
    let soft = jobs.iter().any(|j| j.status == "failed" && j.allow_failure);
    let active = jobs.iter().any(|j| {
        matches!(
            j.status.as_str(),
            "running" | "pending" | "preparing" | "waiting_for_resource"
        )
    }) || reported == "running";
    if active {
        return if hard {
            STATUS_RUNNING_FAILING
        } else if soft {
            STATUS_RUNNING_WARNING
        } else {
            "running"
        }
        .to_owned();
    }
    if jobs.iter().any(Job::failed_hard) {
        return "failed".to_owned();
    }
    match reported {
        "success" | "" if soft => STATUS_SUCCESS_WARNING.to_owned(),
        "" => "success".to_owned(),
        other => other.to_owned(),
    }
}

/// The text to display for a (pseudo-)status.
pub fn status_label(status: &str) -> &str {
    if status == "partially_visible" {
        return "visible";
    }
    status.split(" (").next().unwrap_or(status)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Diff {
    pub old_path: String,
    pub new_path: String,
    pub diff: String,
    pub new_file: bool,
    pub deleted_file: bool,
    pub too_large: bool,
    pub collapsed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DetailPart {
    Core,
    Activity,
    Discussions,
    Pipeline,
    Changes,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    pub item: WorkItem,
    pub notes: Vec<Note>,
    pub discussions: Vec<Discussion>,
    pub jobs: Vec<Job>,
    /// Fork pipelines can run in a different project from the merge request.
    pub jobs_project: Option<u64>,
    pub diffs: Vec<Diff>,
    pub warnings: Vec<String>,
    /// Sections with usable content. Empty sections are distinct from unfetched ones.
    pub loaded: std::collections::HashSet<DetailPart>,
}

#[derive(Clone, Debug)]
pub struct TraceChunk {
    pub text: String,
    pub next_offset: u64,
    pub reset: bool,
}

#[derive(Clone, Debug)]
pub enum Mutation {
    Edit {
        key: ItemKey,
        field: String,
        value: String,
    },
    Comment {
        key: ItemKey,
        body: String,
    },
    Reply {
        key: ItemKey,
        discussion: String,
        body: String,
    },
    Resolve {
        key: ItemKey,
        discussion: String,
        resolved: bool,
    },
    RetryJob {
        project: u64,
        job: u64,
    },
    CreateIssue {
        project: u64,
        title: String,
        description: String,
    },
    CreateMergeRequest {
        project: u64,
        title: String,
        source: String,
        target: String,
        description: String,
    },
}

#[cfg(test)]
mod job_order_tests {
    use super::*;

    fn job(id: u64, name: &str, stage: &str, started: Option<&str>) -> Job {
        Job {
            id,
            name: name.into(),
            stage: stage.into(),
            started_at: started.map(Into::into),
            status: "created".into(),
            ..Job::default()
        }
    }

    #[test]
    fn hard_failures_stay_pinned_in_run_order_as_other_jobs_start() {
        let mut jobs = vec![
            job(1, "first-failure", "test", Some("2025-01-01T10:00:00Z")),
            job(3, "later-failure", "test", Some("2025-01-01T10:05:00Z")),
            job(
                2,
                "same-start-failure",
                "test",
                Some("2025-01-01T10:00:00Z"),
            ),
            job(4, "missing-start-failure", "test", None),
            job(5, "allowed-failure", "test", Some("2025-01-01T10:06:00Z")),
            job(6, "running", "test", Some("2025-01-01T10:07:00Z")),
            job(7, "pending", "deploy", None),
        ];
        for job in &mut jobs[..5] {
            job.status = "failed".into();
        }
        jobs[4].allow_failure = true;
        jobs[5].status = "running".into();
        sort_jobs(&mut jobs);
        assert_eq!(
            jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
            [1, 2, 3, 4, 6, 7, 5]
        );
        let next = jobs.iter_mut().find(|j| j.id == 7).unwrap();
        next.started_at = Some("2025-01-01T10:08:00Z".into());
        next.status = "running".into();
        sort_jobs(&mut jobs);
        assert_eq!(
            jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
            [1, 2, 3, 4, 7, 6, 5]
        );
    }

    #[test]
    fn all_job_states_follow_attention_priority_not_start_time() {
        let cases = [
            (1, "failed", false, Some("2025-01-01T10:00:00Z")),
            (5, "running", false, Some("2025-01-01T10:05:00Z")),
            (4, "preparing", false, Some("2025-01-01T10:04:00Z")),
            (3, "canceling", false, Some("2025-01-01T10:03:00Z")),
            (2, "waiting_for_callback", false, None),
            (6, "created", false, None),
            (7, "pending", false, None),
            (8, "waiting_for_resource", false, None),
            (9, "scheduled", false, None),
            (15, "future-status", false, None),
            (10, "canceled", false, Some("2025-01-01T11:00:00Z")),
            (11, "success", false, Some("2025-01-01T12:00:00Z")),
            (12, "failed", true, Some("2025-01-01T11:30:00Z")),
            (13, "skipped", false, None),
            (14, "manual", false, None),
        ];
        let mut jobs: Vec<_> = cases
            .iter()
            .map(|&(id, status, allow_failure, started)| Job {
                status: status.into(),
                allow_failure,
                ..job(id, &format!("job-{id:02}"), "test", started)
            })
            .collect();
        jobs.reverse();
        jobs.rotate_left(4);
        sort_jobs(&mut jobs);
        assert_eq!(
            jobs.iter().map(|j| j.id).collect::<Vec<_>>(),
            cases.iter().map(|case| case.0).collect::<Vec<_>>()
        );
        let sorted = jobs.iter().map(|j| j.id).collect::<Vec<_>>();
        sort_jobs(&mut jobs);
        assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), sorted);
    }

    #[test]
    fn upcoming_skipped_and_manual_jobs_use_stage_then_name_even_with_start_times() {
        let mut jobs = vec![
            job(1, "finished-build", "build", None),
            job(8, "z-test", "test", Some("2025-01-01T10:10:00Z")),
            job(7, "a-test", "test", None),
            job(6, "b-build", "build", None),
            job(5, "a-build", "build", None),
        ];
        jobs[0].status = "success".into();
        for status in ["pending", "skipped", "manual"] {
            for job in &mut jobs {
                if job.id != 1 {
                    job.status = status.into();
                }
            }
            sort_jobs(&mut jobs);
            let ids: Vec<_> = jobs.iter().map(|j| j.id).collect();
            if status == "pending" {
                assert_eq!(ids, [5, 6, 7, 8, 1]);
            } else {
                assert_eq!(ids, [1, 5, 6, 7, 8]);
            }
        }
    }

    #[test]
    fn manual_job_moves_up_when_started_or_failed_and_down_when_nonblocking() {
        let mut jobs = vec![
            job(1, "automatic", "test", None),
            job(2, "manual", "test", None),
        ];
        jobs[1].status = "manual".into();
        for (status, allow_failure, expected) in [
            ("manual", false, [1, 2]),
            ("running", false, [2, 1]),
            ("failed", false, [2, 1]),
            ("failed", true, [1, 2]),
            ("success", false, [1, 2]),
        ] {
            let manual = jobs.iter_mut().find(|j| j.id == 2).unwrap();
            manual.status = status.into();
            manual.allow_failure = allow_failure;
            sort_jobs(&mut jobs);
            assert_eq!(jobs.iter().map(|j| j.id).collect::<Vec<_>>(), expected);
        }
    }

    #[test]
    fn active_jobs_newest_first_then_upcoming_by_stage_and_name() {
        let mut jobs = vec![
            job(1, "lint", "check", Some("2025-01-01T10:00:00Z")),
            job(2, "unit-2", "test", Some("2025-01-01T10:05:00Z")),
            job(3, "unit-1", "test", Some("2025-01-01T10:06:00Z")),
            job(9, "deploy", "deploy", None),
            job(5, "e2e-b", "e2e", None),
            job(4, "e2e-a", "e2e", None),
        ];
        for job in &mut jobs[..3] {
            job.status = "running".into();
        }
        sort_jobs(&mut jobs);
        let names: Vec<_> = jobs.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(
            names,
            ["unit-1", "unit-2", "lint", "e2e-a", "e2e-b", "deploy"]
        );
    }
}
