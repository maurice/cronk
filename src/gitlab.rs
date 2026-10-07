//! Blocking, cloneable GitLab REST client. Call from background workers, not the UI thread.
//!
//! Installation URLs retain their subpath (an existing `/api/v4` suffix is accepted).
//! Collections are paginated to completion, with no arbitrary item/page cutoff. Individual
//! JSON responses are limited to 8 MiB and the shared validator cache to 128 entries/16 MiB.
//! Persistent list sync uses overlapping updated_at windows and resumable timestamp
//! boundaries. Full reconciliation runs daily or on demand; missing identities are
//! verified individually before deletion. Pipelines/discussions refresh independently.
//!
//! Dashboard reads enrich at most 12 recently updated open MRs missing a pipeline or exact
//! discussion count, with up to one MR-detail request each (only for a missing pipeline)
//! and a shared budget of 12 discussion pages. Counts are populated only after every page
//! has loaded; budget exhaustion or an unsupported endpoint leaves None, never a partial
//! count or an inferred zero. Paginated reads are not atomic server snapshots.
//! Optional dashboard enrichment HTTP 403/404/405/501 leaves missing fields unknown; other
//! failures (including rate limits) are errors since WorkItem has no warning channel.
//! Issue/MR collection 403/404 is skipped ONLY if project metadata confirms that feature
//! is disabled. Missing permissions/resources must not masquerade as an empty collection.
//! Optional detail failures preserve partial results in Details::warnings. Retry-After is reported,
//! never slept on. Jobs belong to the current head pipeline; downstream/child pipelines
//! are not recursively traversed. Server-side diff limits cannot always be detected on
//! older GitLab versions. Editing is restricted to the fields documented on `mutate`.
//!
//! Trace offsets count original bytes, including stripped controls. Bounded numeric SGR
//! color/style sequences are preserved for widget rendering; other escape commands and
//! control-string payloads are stripped, including across fetch boundaries. Reads return at most
//! 256 KiB of source bytes. Byte query parameters are capability-probed; older servers
//! fall back to Range. Servers ignoring Range require streaming past the prefix, limited
//! to 8 MiB per call (an explicit error beyond that, never silent truncation). Sequential
//! reads share UTF-8/escape state for up to 64 jobs, keyed by project AND job. Requests for
//! a retained job are serialized without holding the global state lock during network I/O.
//! A nonzero offset without matching parser state restarts at zero and returns reset=true,
//! rather than exposing the tail of an OSC/DCS payload. This also applies after state eviction.
//! A 65th concurrently active job returns an explicit capacity error; idle states are evicted.
//! Same-length log replacement cannot reliably be detected by either trace read API.
//! A trailing incomplete UTF-8 character (at most three bytes) is held until another poll.

use crate::model::{
    CurrentIteration, DetailPart, Details, Diff, Discussion, ItemKey, ItemKind, Label, LookupKind,
    LookupOption, Mutation, Pipeline, Project, TraceChunk, User, WorkItem, sort_jobs,
};
use anyhow::{Result, anyhow, bail};
use reqwest::{
    Method, StatusCode, Url,
    blocking::{Client, Response},
    header::{
        ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, ETAG, HeaderMap,
        HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, LINK, RANGE, RETRY_AFTER,
    },
    redirect::Policy,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

const PAGE_SIZE: usize = 100;
const JSON_LIMIT: usize = 8 * 1024 * 1024;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;
const DASHBOARD_ENRICHMENT: usize = 12;
const DASHBOARD_DISCUSSION_PAGES: usize = 12;
const TRACE_BYTES: usize = 256 * 1024;
const TRACE_SKIP_LIMIT: u64 = 8 * 1024 * 1024;
const TRACE_STATES: usize = 64;

#[derive(Clone)]
pub struct GitLab {
    client: Client,
    api: Url,
    token: Arc<str>,
    cache: Arc<Mutex<Cache>>,
    traces: Arc<Mutex<VecDeque<Arc<TraceState>>>>,
    // 0 = unknown (including empty traces), 1 = supported, 2 = legacy Range fallback.
    trace_byte_params: Arc<AtomicU8>,
    persistent: Option<Arc<crate::cache::ContentCache>>,
    sync_locks: Arc<Mutex<HashMap<u64, Arc<Mutex<()>>>>>,
}

#[derive(Clone, Debug)]
pub struct SyncProgress {
    pub kind: ItemKind,
    pub loaded: usize,
    pub total: Option<usize>,
    pub page: usize,
    pub phase: &'static str,
}

impl SyncProgress {
    pub fn label(&self) -> String {
        if self.phase == "dashboard enrichment" {
            return "Refreshing dashboard signals…".into();
        }
        let resource = match self.kind {
            ItemKind::Issue => "Issues",
            ItemKind::MergeRequest => "MRs",
        };
        let count = self.total.map_or_else(
            || format!("{} loaded", self.loaded),
            |total| format!("{}/{total}", self.loaded),
        );
        let bar = self
            .total
            .filter(|total| *total > 0)
            .map_or_else(String::new, |total| {
                let filled = (self.loaded.saturating_mul(8) / total).min(8);
                format!(" [{}{}]", "█".repeat(filled), "░".repeat(8 - filled))
            });
        format!(
            "{resource}: {count}{bar} · page {} · {}",
            self.page, self.phase
        )
    }
}

#[derive(Debug)]
struct HttpFailure {
    status: StatusCode,
    message: String,
}

impl std::fmt::Display for HttpFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HttpFailure {}

fn http_status(error: &anyhow::Error) -> Option<StatusCode> {
    error
        .downcast_ref::<HttpFailure>()
        .map(|failure| failure.status)
}

fn unsupported_enrichment(error: &anyhow::Error) -> bool {
    http_status(error).is_some_and(|status| matches!(status.as_u16(), 403 | 404 | 405 | 501))
}

#[derive(Clone)]
struct Document {
    body: Arc<[u8]>,
    headers: HeaderMap,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Document>,
    order: VecDeque<String>,
    bytes: usize,
    generation: u64,
}

impl Cache {
    fn size(url: &str, doc: &Document) -> usize {
        url.len()
            + doc.body.len()
            + doc
                .headers
                .iter()
                .map(|(k, v)| k.as_str().len() + v.len())
                .sum::<usize>()
    }

    fn get(&mut self, url: &str) -> Option<Document> {
        let entry = self.entries.get(url)?.clone();
        self.order.retain(|key| key != url);
        self.order.push_back(url.to_owned());
        Some(entry)
    }

    fn insert(&mut self, url: String, doc: Document) {
        if let Some(old) = self.entries.remove(&url) {
            self.bytes -= Self::size(&url, &old);
            self.order.retain(|key| key != &url);
        }
        let size = Self::size(&url, &doc);
        if size > CACHE_BYTES {
            return;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + size > CACHE_BYTES {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if let Some(old) = self.entries.remove(&key) {
                self.bytes -= Self::size(&key, &old);
            }
        }
        self.bytes += size;
        self.order.push_back(url.clone());
        self.entries.insert(url, doc);
    }

    fn invalidate(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.bytes = 0;
        self.generation = self.generation.wrapping_add(1);
    }
}

#[derive(Deserialize)]
struct IterationMatch {
    id: u64,
    #[serde(default)]
    iid: Option<u64>,
    title: Option<String>,
    start_date: Option<String>,
    due_date: Option<String>,
    #[serde(default)]
    state: Option<Value>,
    #[serde(default)]
    web_url: Option<String>,
}

/// GitLab reports iteration state as 1/2/3 (older) or upcoming/current/closed (newer).
fn iteration_state(state: Option<&Value>) -> Option<&'static str> {
    match state? {
        Value::Number(n) => match n.as_u64()? {
            1 => Some("upcoming"),
            2 => Some("current"),
            3 => Some("closed"),
            _ => None,
        },
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "upcoming" => Some("upcoming"),
            "current" => Some("current"),
            "closed" => Some("closed"),
            _ => None,
        },
        _ => None,
    }
}

/// Use the reported state; when absent, fall back to whether today is inside the date range.
fn iteration_is_current(
    state: Option<&Value>,
    start: Option<&str>,
    due: Option<&str>,
    today: chrono::NaiveDate,
) -> bool {
    if let Some(state) = iteration_state(state) {
        return state == "current";
    }
    let parse = |value: Option<&str>| {
        value.and_then(|v| chrono::NaiveDate::parse_from_str(v.trim(), "%Y-%m-%d").ok())
    };
    matches!((parse(start), parse(due)), (Some(s), Some(d)) if s <= today && today <= d)
}

/// "28 Sep 2026 - 11 Oct 2026", prefixed by the title when there is one, and
/// followed by "(Current)" for the running iteration.
fn iteration_label(
    title: Option<&str>,
    start: Option<&str>,
    due: Option<&str>,
    current: bool,
) -> String {
    let dates = if start.is_none() && due.is_none() {
        "Undated".to_owned()
    } else {
        crate::dates::format_range(start, due)
    };
    let mut label = match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => format!("{title} ({dates})"),
        None => dates,
    };
    if current {
        label.push_str(" (Current)");
    }
    label
}

impl IterationMatch {
    fn is_current(&self) -> bool {
        iteration_is_current(
            self.state.as_ref(),
            self.start_date.as_deref(),
            self.due_date.as_deref(),
            chrono::Local::now().date_naive(),
        )
    }

    fn label(&self, current: bool) -> String {
        iteration_label(
            self.title.as_deref(),
            self.start_date.as_deref(),
            self.due_date.as_deref(),
            current,
        )
    }

    /// Full path of the owning group or project, from `…/groups/<path>/-/…` or `…/<path>/-/…`.
    fn group_path(&self) -> Option<String> {
        let url = Url::parse(self.web_url.as_deref()?).ok()?;
        let segments: Vec<_> = url.path_segments()?.collect();
        let end = segments.iter().position(|s| *s == "-")?;
        let start = if segments.first() == Some(&"groups") {
            1
        } else {
            0
        };
        (start < end).then(|| segments[start..end].join("/"))
    }

    /// Milestones are plain titles: no iteration-style date label or "Current" marker.
    fn milestone_option(&self) -> LookupOption {
        let title = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map_or_else(|| format!("Milestone {}", self.id), str::to_owned);
        let dates = (self.start_date.is_some() || self.due_date.is_some()).then(|| {
            crate::dates::format_range(self.start_date.as_deref(), self.due_date.as_deref())
        });
        LookupOption {
            id: self.id,
            value: title.clone(),
            label: title,
            api_value: self.id.to_string(),
            description: [self.group_path(), dates]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · "),
            color: String::new(),
            text_color: String::new(),
        }
    }

    /// Second line of a suggestion: owning group and, unless running, the state.
    fn description(&self, current: bool) -> String {
        let state = match iteration_state(self.state.as_ref()) {
            _ if current => Some("Current iteration"),
            Some("upcoming") => Some("Upcoming"),
            Some("closed") => Some("Closed"),
            _ => None,
        };
        [state.map(str::to_owned), self.group_path()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn lookup_option(&self) -> LookupOption {
        let current = self.is_current();
        let label = self.label(current);
        LookupOption {
            id: self.id,
            value: label.clone(),
            label,
            api_value: self.id.to_string(),
            description: self.description(current),
            color: String::new(),
            text_color: String::new(),
        }
    }

    fn current_iteration(&self) -> CurrentIteration {
        CurrentIteration {
            id: self.id,
            title: self.label(true),
            description: self.description(true),
        }
    }
}

impl GitLab {
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        let mut api =
            Url::parse(base_url).map_err(|_| anyhow!("Invalid GitLab installation URL"))?;
        let host = api.host_str().unwrap_or_default();
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if api.scheme() != "https" && !(api.scheme() == "http" && loopback) {
            bail!("GitLab requires HTTPS; HTTP is allowed only for loopback hosts");
        }
        if host.is_empty()
            || !api.username().is_empty()
            || api.password().is_some()
            || api.query().is_some()
            || api.fragment().is_some()
        {
            bail!("GitLab URL must have a host and no credentials, query, or fragment");
        }
        if !api.path().trim_end_matches('/').ends_with("/api/v4") {
            api.path_segments_mut()
                .map_err(|_| anyhow!("Invalid GitLab URL path"))?
                .pop_if_empty()
                .extend(["api", "v4"]);
        }
        api.path_segments_mut()
            .map_err(|_| anyhow!("Invalid GitLab URL path"))?
            .pop_if_empty()
            .push("");
        if token.trim().is_empty() {
            bail!("GitLab token must not be empty");
        }
        let mut auth =
            HeaderValue::from_str(token).map_err(|_| anyhow!("Invalid GitLab token header"))?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert("PRIVATE-TOKEN", auth);
        // Keep native system roots and supplement them with an optional corporate CA bundle.
        // Read once per client construction; clones share the resulting TLS configuration.
        let mut builder = Client::builder();
        if let Some(path) = std::env::var_os("NODE_EXTRA_CA_CERTS").filter(|path| !path.is_empty())
        {
            let pem = std::fs::read(path).map_err(|_| {
                anyhow!("Could not read the PEM CA bundle from NODE_EXTRA_CA_CERTS")
            })?;
            let certificates = reqwest::Certificate::from_pem_bundle(&pem)
                .map_err(|_| anyhow!("Invalid PEM CA bundle in NODE_EXTRA_CA_CERTS"))?;
            if certificates.is_empty() {
                bail!("NODE_EXTRA_CA_CERTS must contain at least one PEM certificate");
            }
            for certificate in certificates {
                builder = builder.add_root_certificate(certificate);
            }
        }
        let client = builder
            .default_headers(headers)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("cronk-gitlab-tui")
            .build()
            .map_err(|_| {
                anyhow!(
                    "Could not initialize GitLab HTTP client/TLS roots; check native roots and NODE_EXTRA_CA_CERTS"
                )
            })?;
        Ok(Self {
            client,
            api,
            token: Arc::from(token),
            cache: Arc::new(Mutex::new(Cache::default())),
            traces: Arc::new(Mutex::new(VecDeque::new())),
            trace_byte_params: Arc::new(AtomicU8::new(0)),
            persistent: None,
            sync_locks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.api.clone();
        url.path_segments_mut()
            .expect("validated hierarchical URL")
            .pop_if_empty()
            .extend(segments.iter().copied());
        url
    }

    pub fn current_iteration(&self, project: u64) -> Result<Option<CurrentIteration>> {
        let project = project.to_string();
        let mut url = self.url(&["projects", &project, "iterations"]);
        url.query_pairs_mut()
            .append_pair("per_page", "20")
            .append_pair("page", "1")
            .append_pair("include_ancestors", "true")
            .append_pair("state", "current");
        let entries: Vec<IterationMatch> = self.get(&url)?;
        Ok(entries
            .into_iter()
            .next()
            .map(|entry| entry.current_iteration()))
    }

    /// Bounded, first-page typeahead. Refine the query rather than fetching every page.
    pub fn lookup(&self, project: u64, kind: LookupKind, query: &str) -> Result<Vec<LookupOption>> {
        let project = project.to_string();
        let mut url = match kind {
            LookupKind::Users => self.url(&["projects", &project, "users"]),
            LookupKind::Milestones => self.url(&["projects", &project, "milestones"]),
            LookupKind::Iterations => self.url(&["projects", &project, "iterations"]),
            LookupKind::Epics => {
                // Epics belong to a project's group (and ancestor groups), not the project.
                #[derive(Deserialize)]
                struct Namespace {
                    id: u64,
                    kind: String,
                }
                #[derive(Deserialize)]
                struct ProjectNamespace {
                    namespace: Namespace,
                }
                let metadata: ProjectNamespace = self.get(&self.url(&["projects", &project]))?;
                if metadata.namespace.kind != "group" {
                    return Ok(vec![]);
                }
                self.url(&["groups", &metadata.namespace.id.to_string(), "epics"])
            }
            LookupKind::Projects => self.url(&["projects"]),
            LookupKind::Labels => self.url(&["projects", &project, "labels"]),
        };
        let query = query.trim();
        let query = if kind == LookupKind::Users {
            query.trim_start_matches('@')
        } else {
            query
        };
        url.query_pairs_mut()
            .append_pair("per_page", "20")
            .append_pair("page", "1");
        if !query.is_empty() {
            url.query_pairs_mut().append_pair("search", query);
        }
        match kind {
            LookupKind::Users => {
                let users: Vec<User> = self.get(&url)?;
                Ok(users.iter().take(20).map(LookupOption::user).collect())
            }
            LookupKind::Projects => {
                url.query_pairs_mut()
                    .append_pair("membership", "true")
                    .append_pair("simple", "true")
                    .append_pair("search_namespaces", "true");
                #[derive(Deserialize)]
                struct Match {
                    id: u64,
                    name: String,
                    path_with_namespace: String,
                }
                let projects: Vec<Match> = self.get(&url)?;
                Ok(projects
                    .into_iter()
                    .take(20)
                    .map(|p| {
                        let description = format!("{} · ID {}", p.path_with_namespace, p.id);
                        LookupOption {
                            id: p.id,
                            label: p.name,
                            value: p.path_with_namespace,
                            api_value: p.id.to_string(),
                            description,
                            color: String::new(),
                            text_color: String::new(),
                        }
                    })
                    .collect())
            }
            LookupKind::Milestones | LookupKind::Iterations => {
                url.query_pairs_mut()
                    .append_pair("include_ancestors", "true");
                if kind == LookupKind::Iterations {
                    // With nothing typed, offer what can still be chosen: current and upcoming.
                    let state = if query.trim().is_empty() {
                        "opened"
                    } else {
                        "all"
                    };
                    url.query_pairs_mut()
                        .append_pair("state", state)
                        .append_pair("in[]", "title")
                        .append_pair("in[]", "cadence_title");
                }
                let entries: Vec<IterationMatch> = self.get(&url)?;
                let mut options: Vec<_> = entries
                    .iter()
                    .take(20)
                    .map(|entry| {
                        (
                            entry,
                            if kind == LookupKind::Iterations {
                                entry.lookup_option()
                            } else {
                                entry.milestone_option()
                            },
                        )
                    })
                    .collect();
                // The label is the identity the user picks by, so it must be unique.
                let labels: Vec<_> = options.iter().map(|(_, o)| o.value.clone()).collect();
                for (entry, option) in &mut options {
                    if labels
                        .iter()
                        .filter(|label| **label == option.value)
                        .count()
                        > 1
                    {
                        let suffix = entry
                            .group_path()
                            .or_else(|| entry.iid.map(|iid| format!("#{iid}")))
                            .unwrap_or_else(|| format!("#{}", entry.id));
                        option.value = format!("{} · {suffix}", option.value);
                        option.label = option.value.clone();
                    }
                }
                Ok(options.into_iter().map(|(_, option)| option).collect())
            }
            LookupKind::Epics => {
                url.query_pairs_mut()
                    .append_pair("include_ancestor_groups", "true")
                    .append_pair("include_descendant_groups", "false");
                #[derive(Deserialize)]
                struct EpicMatch {
                    id: u64,
                    iid: u64,
                    title: String,
                    group_id: u64,
                }
                let epics: Vec<EpicMatch> = self.get(&url)?;
                Ok(epics
                    .into_iter()
                    .take(20)
                    .map(|epic| LookupOption {
                        id: epic.id,
                        label: epic.title.clone(),
                        // Qualify identities so equal titles never resolve to the wrong epic.
                        value: format!("{} · group {} &{}", epic.title, epic.group_id, epic.iid),
                        api_value: epic.id.to_string(),
                        description: format!(
                            "group {} · &{} · ID {}",
                            epic.group_id, epic.iid, epic.id
                        ),
                        color: String::new(),
                        text_color: String::new(),
                    })
                    .collect())
            }
            LookupKind::Labels => {
                url.query_pairs_mut()
                    .append_pair("include_ancestor_groups", "true");
                let labels: Vec<Label> = self.get(&url)?;
                Ok(labels
                    .into_iter()
                    .take(20)
                    .map(label_contrast)
                    .map(|label| LookupOption::label(&label))
                    .collect())
            }
        }
    }

    fn item_url(&self, key: &ItemKey, suffix: &[&str]) -> Url {
        let project = key.project.to_string();
        let iid = key.iid.to_string();
        let mut segments = vec!["projects", &project, key.kind.segment(), &iid];
        segments.extend_from_slice(suffix);
        self.url(&segments)
    }

    fn redacted(&self, text: &str) -> String {
        let encoded: String = self.token.bytes().map(|b| format!("%{b:02X}")).collect();
        let safe = text
            .replace(self.token.as_ref(), "[REDACTED]")
            .replace(&encoded, "[REDACTED]")
            .replace(&encoded.to_lowercase(), "[REDACTED]");
        safe.chars().filter(|c| !c.is_control()).take(512).collect()
    }

    fn context(&self, method: &Method, url: &Url) -> String {
        // Do not expose credentials, query parameters, response bodies, or transport source chains.
        format!("{method} {}", self.redacted(url.path()))
    }

    fn send(
        &self,
        method: Method,
        url: &Url,
        headers: HeaderMap,
        body: Option<&Value>,
    ) -> Result<Response> {
        let mut request = self
            .client
            .request(method.clone(), url.clone())
            .headers(headers);
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().map_err(|error| {
            let reason = if error.is_timeout() {
                "request timed out"
            } else if error.is_connect() {
                "connection or TLS failure"
            } else {
                "HTTP transport failure"
            };
            anyhow!("{}: {reason}", self.context(&method, url))
        })
    }

    fn status_error(&self, method: &Method, url: &Url, response: &Response) -> anyhow::Error {
        let retry = response
            .headers()
            .get(RETRY_AFTER)
            .map(|value| {
                format!(
                    "; Retry-After: {} (retry later; no automatic retry)",
                    self.redacted(value.to_str().unwrap_or("unreadable header"))
                )
            })
            .unwrap_or_default();
        let hint = match response.status().as_u16() {
            401 => "; check token validity",
            403 => "; check token scopes and project permissions",
            404 => "; resource unavailable, missing, or not visible to this token",
            300..=399 => {
                "; redirects are disabled to protect the token; check the installation URL"
            }
            429 => "; GitLab rate limit reached",
            _ => "",
        };
        HttpFailure {
            status: response.status(),
            message: format!(
                "{}: HTTP {}{hint}{retry}",
                self.context(method, url),
                response.status()
            ),
        }
        .into()
    }

    fn document(&self, url: &Url) -> Result<Document> {
        let (cached, generation) = {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            (cache.get(url.as_str()), cache.generation)
        };
        let mut conditional = HeaderMap::new();
        if let Some(doc) = &cached {
            if let Some(etag) = doc.headers.get(ETAG) {
                conditional.insert(IF_NONE_MATCH, etag.clone());
            }
            if let Some(modified) = doc.headers.get(LAST_MODIFIED) {
                conditional.insert(IF_MODIFIED_SINCE, modified.clone());
            }
        }
        let response = self.send(Method::GET, url, conditional, None)?;
        let doc = if response.status() == StatusCode::NOT_MODIFIED {
            let mut doc = cached.ok_or_else(|| {
                anyhow!(
                    "{}: unexpected 304 without a cached response",
                    self.context(&Method::GET, url)
                )
            })?;
            // A 304 usually omits pagination metadata; retain the cached headers.
            for (name, value) in response.headers() {
                doc.headers.insert(name.clone(), value.clone());
            }
            doc
        } else {
            if !response.status().is_success() {
                return Err(self.status_error(&Method::GET, url, &response));
            }
            let headers = response.headers().clone();
            let mut body = Vec::new();
            response
                .take(JSON_LIMIT as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|_| {
                    anyhow!(
                        "{}: could not read JSON response",
                        self.context(&Method::GET, url)
                    )
                })?;
            if body.len() > JSON_LIMIT {
                bail!(
                    "{}: JSON response exceeds the explicit {} MiB safety limit",
                    self.context(&Method::GET, url),
                    JSON_LIMIT / 1024 / 1024
                );
            }
            Document {
                body: body.into(),
                headers,
            }
        };
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        // A GET already in flight when a mutation completes must not repopulate stale cache data.
        if cache.generation == generation {
            cache.insert(url.as_str().to_owned(), doc.clone());
        }
        Ok(doc)
    }

    fn decode<T: DeserializeOwned>(&self, url: &Url, doc: &Document) -> Result<T> {
        serde_json::from_slice(&doc.body).map_err(|error| {
            anyhow!(
                "{}: invalid JSON/schema at line {}, column {}",
                self.context(&Method::GET, url),
                error.line(),
                error.column()
            )
        })
    }

    fn get<T: DeserializeOwned>(&self, url: &Url) -> Result<T> {
        self.decode(url, &self.document(url)?)
    }

    fn pages<T: DeserializeOwned>(&self, url: Url, result: &mut Vec<T>) -> Result<()> {
        self.pages_progress(url, Some(result), |_, _| Ok(()))
    }

    fn pages_progress<T: DeserializeOwned>(
        &self,
        mut url: Url,
        mut result: Option<&mut Vec<T>>,
        mut progress: impl FnMut(&[T], &HeaderMap) -> Result<()>,
    ) -> Result<()> {
        url.query_pairs_mut()
            .append_pair("per_page", &PAGE_SIZE.to_string())
            .append_pair("page", "1");
        let mut visited = HashSet::new();
        let mut previous: Option<Arc<[u8]>> = None;
        loop {
            if !visited.insert(url.as_str().to_owned()) {
                bail!(
                    "{}: pagination cycle detected",
                    self.context(&Method::GET, &url)
                );
            }
            let doc = self.document(&url)?;
            let page: Vec<T> = self.decode(&url, &doc)?;
            let count = page.len();
            if count > 0 && previous.as_deref() == Some(doc.body.as_ref()) {
                bail!(
                    "{}: repeated page; server may be ignoring pagination",
                    self.context(&Method::GET, &url)
                );
            }
            previous = Some(doc.body.clone());
            progress(&page, &doc.headers)?;
            if let Some(result) = result.as_mut() {
                result.extend(page);
            }
            let next = self.next_page(&url, &doc.headers, count)?;
            let Some(next) = next else { return Ok(()) };
            url = next;
        }
    }

    fn next_page(&self, url: &Url, headers: &HeaderMap, count: usize) -> Result<Option<Url>> {
        let invalid = || {
            anyhow!(
                "{}: invalid or unsafe pagination metadata",
                self.context(&Method::GET, url)
            )
        };
        if let Some(next) = headers.get("x-next-page") {
            let next = next.to_str().map_err(|_| invalid())?.trim();
            if next.is_empty() {
                return Ok(None);
            }
            let next: u64 = next.parse().map_err(|_| invalid())?;
            let current: u64 = url
                .query_pairs()
                .find(|(k, _)| k == "page")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(1);
            if next <= current {
                return Err(invalid());
            }
            return Ok(Some(with_page(url, next)));
        }
        if headers.contains_key(LINK) {
            for header in headers.get_all(LINK) {
                for link in header.to_str().map_err(|_| invalid())?.split(',') {
                    let mut parts = link.trim().split(';');
                    let target = parts.next().unwrap_or_default().trim();
                    let is_next = parts.any(|part| {
                        part.trim().split_once('=').is_some_and(|(key, value)| {
                            key.trim().eq_ignore_ascii_case("rel")
                                && value
                                    .trim()
                                    .trim_matches('"')
                                    .split_ascii_whitespace()
                                    .any(|rel| rel == "next")
                        })
                    });
                    if is_next {
                        let target = target
                            .strip_prefix('<')
                            .and_then(|v| v.strip_suffix('>'))
                            .ok_or_else(invalid)?;
                        let next = url.join(target).map_err(|_| invalid())?;
                        if next.origin() != url.origin()
                            || next.path() != url.path()
                            || !next.username().is_empty()
                            || next.password().is_some()
                            || next.fragment().is_some()
                        {
                            return Err(invalid());
                        }
                        return Ok(Some(next));
                    }
                }
            }
            return Ok(None);
        }
        // Some proxies strip pagination headers. A full page requires probing the next page.
        if count >= PAGE_SIZE {
            let current: u64 = url
                .query_pairs()
                .find(|(k, _)| k == "page")
                .and_then(|(_, v)| v.parse().ok())
                .ok_or_else(invalid)?;
            return Ok(Some(with_page(
                url,
                current.checked_add(1).ok_or_else(invalid)?,
            )));
        }
        Ok(None)
    }

    fn all<T: DeserializeOwned>(&self, url: Url) -> Result<Vec<T>> {
        let mut result = Vec::new();
        self.pages(url, &mut result)?;
        Ok(result)
    }

    fn optional<T: DeserializeOwned>(
        &self,
        url: Url,
        label: &str,
        warnings: &mut Vec<String>,
    ) -> (Vec<T>, bool) {
        let mut result = Vec::new();
        match self.pages(url, &mut result) {
            Ok(()) => (result, true),
            Err(error) => {
                warnings.push(format!(
                    "{label} incomplete ({} entries loaded): {error}",
                    result.len()
                ));
                (result, false)
            }
        }
    }

    pub fn enable_persistence(&mut self, workspace: &std::path::Path) -> Result<()> {
        self.persistent = Some(Arc::new(crate::cache::ContentCache::open(
            workspace,
            self.api.as_str(),
            &self.token,
        )?));
        Ok(())
    }

    pub fn cached_items(&self, project: u64) -> Result<Vec<WorkItem>> {
        self.persistent
            .as_ref()
            .map_or_else(|| Ok(Vec::new()), |cache| cache.items(project))
    }

    pub fn cached_details(&self, key: &ItemKey) -> Result<Option<Details>> {
        self.persistent
            .as_ref()
            .map_or_else(|| Ok(None), |cache| cache.detail(key))
    }

    pub fn clear_content_cache(&self) -> Result<()> {
        self.persistent
            .as_ref()
            .map_or(Ok(()), |cache| cache.clear())
    }

    pub fn current_user(&self) -> Result<User> {
        self.get(&self.url(&["user"]))
    }

    pub fn resolve_project(&self, path: &str) -> Result<Project> {
        if path.trim().is_empty() || matches!(path, "." | "..") {
            bail!("Project path must not be empty or a dot segment");
        }
        #[derive(Deserialize)]
        struct ApiProject {
            id: u64,
            path_with_namespace: String,
            #[serde(default)]
            name: String,
        }
        let project: ApiProject = self.get(&self.url(&["projects", path]))?;
        Ok(Project {
            id: project.id,
            path: project.path_with_namespace,
            // Prefer GitLab's short display name as the starting local alias.
            alias: project.name,
            visible: true,
        })
    }

    fn labels(&self, project: u64) -> Result<HashMap<String, Label>> {
        let mut url = self.url(&["projects", &project.to_string(), "labels"]);
        url.query_pairs_mut()
            .append_pair("include_ancestor_groups", "true");
        Ok(self
            .all::<Label>(url)?
            .into_iter()
            .map(|label| (label.name.clone(), label))
            .collect())
    }

    fn feature_disabled(&self, project: u64, kind: ItemKind) -> Result<bool> {
        #[derive(Deserialize)]
        struct Features {
            id: u64,
            issues_access_level: Option<String>,
            issues_enabled: Option<bool>,
            merge_requests_access_level: Option<String>,
            merge_requests_enabled: Option<bool>,
        }
        let features: Features = self.get(&self.url(&["projects", &project.to_string()]))?;
        if features.id != project {
            bail!("Project feature response does not match the requested project");
        }
        let (access, enabled) = match kind {
            ItemKind::Issue => (features.issues_access_level, features.issues_enabled),
            ItemKind::MergeRequest => (
                features.merge_requests_access_level,
                features.merge_requests_enabled,
            ),
        };
        // The modern access level takes precedence over the deprecated enabled flag.
        Ok(access.map_or(enabled == Some(false), |access| access == "disabled"))
    }

    fn dashboard_discussions(&self, key: &ItemKey, budget: &mut usize) -> Result<Option<usize>> {
        let mut url = self.item_url(key, &["discussions"]);
        url.query_pairs_mut()
            .append_pair("per_page", &PAGE_SIZE.to_string())
            .append_pair("page", "1");
        let mut visited = HashSet::new();
        let mut discussions = HashSet::new();
        let mut unresolved = 0;
        while *budget > 0 {
            if !visited.insert(url.as_str().to_owned()) {
                bail!(
                    "{}: discussion pagination cycle detected",
                    self.context(&Method::GET, &url)
                );
            }
            *budget -= 1;
            let doc = match self.document(&url) {
                Ok(doc) => doc,
                Err(error) if unsupported_enrichment(&error) => return Ok(None),
                Err(error) => return Err(error),
            };
            let page: Vec<Discussion> = self.decode(&url, &doc)?;
            for discussion in &page {
                if !discussions.insert(discussion.id.clone()) {
                    // Page movement during concurrent writes cannot establish an exact count.
                    return Ok(None);
                }
                if discussion
                    .notes
                    .iter()
                    .any(|note| note.resolvable && !note.resolved)
                {
                    unresolved += 1;
                }
            }
            match self.next_page(&url, &doc.headers, page.len())? {
                Some(next) => url = next,
                None => return Ok(Some(unresolved)),
            }
        }
        Ok(None)
    }

    pub fn list_project(&self, project: &Project) -> Result<Vec<WorkItem>> {
        self.sync_project(project, false, |_, _| {})
    }

    /// Publish durable pages immediately. A per-project lock prevents overlapping
    /// refreshes from racing checkpoints, including refreshes after remote writes.
    pub fn sync_project(
        &self,
        project: &Project,
        full: bool,
        progress: impl FnMut(SyncProgress, Vec<WorkItem>) + Send,
    ) -> Result<Vec<WorkItem>> {
        let lock = self
            .sync_locks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(project.id)
            .or_default()
            .clone();
        let _sync = lock.lock().unwrap_or_else(|e| e.into_inner());
        let revision = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation;
        let progress = Mutex::new(progress);
        let report =
            |status, items| (progress.lock().unwrap_or_else(|e| e.into_inner()))(status, items);
        let fetch_kind = |kind: ItemKind| -> Result<Vec<WorkItem>> {
            let mut items = Vec::new();
            let mut url = self.url(&["projects", &project.id.to_string(), kind.segment()]);
            url.query_pairs_mut()
                .append_pair("state", "all")
                .append_pair("scope", "all")
                .append_pair("order_by", "updated_at")
                .append_pair("sort", "desc")
                .append_pair("with_labels_details", "true");
            let plan = self
                .persistent
                .as_ref()
                .map(|cache| cache.plan(project.id, kind, full))
                .transpose()?;
            if let Some(plan) = &plan {
                if let Some(after) = &plan.after {
                    url.query_pairs_mut().append_pair("updated_after", after);
                }
                url.query_pairs_mut()
                    .append_pair("updated_before", &plan.before);
            }
            let mut loaded = 0;
            let mut page_number = 0;
            let mut label_colors = None;
            match self.pages_progress::<ApiItem>(url, None, |raw, headers| {
                page_number += 1;
                loaded += raw.len();
                let total = headers
                    .get("x-total")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok());
                let mut page: Vec<WorkItem> = raw
                    .iter()
                    .cloned()
                    .map(|raw| raw.into_item(project.id, kind))
                    .collect();
                if page.iter().any(needs_label_colors) {
                    if label_colors.is_none() {
                        label_colors = Some(self.labels(project.id)?);
                    }
                    for item in &mut page {
                        apply_label_colors(item, label_colors.as_ref().unwrap());
                    }
                }
                let mut status = SyncProgress {
                    kind,
                    loaded,
                    total,
                    page: page_number,
                    phase: "saving",
                };
                report(status.clone(), Vec::new());
                // Serialize writes with successful mutations: an older GET cannot
                // repopulate disk after mutation invalidation.
                let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                if guard.generation != revision {
                    bail!("Sync superseded by a GitLab write; refresh again");
                }
                if let (Some(cache), Some(plan)) = (&self.persistent, &plan) {
                    cache.page(project.id, kind, plan, &page)?;
                }
                drop(guard);
                if self.persistent.is_none() {
                    items.extend(page.clone());
                }
                status.phase = "fetching";
                report(status, page);
                Ok(())
            }) {
                Ok(()) => (),
                Err(error)
                    if matches!(
                        http_status(&error),
                        Some(StatusCode::FORBIDDEN | StatusCode::NOT_FOUND)
                    ) =>
                {
                    match self.feature_disabled(project.id, kind) {
                        Ok(true) => (),
                        Ok(false) => return Err(error),
                        Err(verification) => {
                            return Err(error.context(format!(
                                "Cannot confirm the project feature is disabled: {verification}"
                            )));
                        }
                    }
                }
                Err(error) => return Err(error),
            };
            // Feature-disabled handling above must still finish its empty collection.
            if let (Some(cache), Some(plan)) = (&self.persistent, &plan) {
                report(
                    SyncProgress {
                        kind,
                        loaded,
                        total: None,
                        page: page_number,
                        phase: "reconciling",
                    },
                    Vec::new(),
                );
                let mut removed = Vec::new();
                for key in cache.missing(project.id, kind, plan)? {
                    // Offset pagination is not an atomic snapshot: an updated item
                    // may have moved out of the bounded import window. Verify it.
                    match self.get::<ApiItem>(&self.item_url(&key, &[])) {
                        Ok(raw) => {
                            let item = raw.into_item(project.id, kind);
                            let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                            if guard.generation != revision {
                                bail!("Sync superseded by a GitLab write; refresh again");
                            }
                            cache.enrich(std::slice::from_ref(&item))?;
                            items.push(item);
                        }
                        Err(error)
                            if matches!(
                                http_status(&error),
                                Some(StatusCode::NOT_FOUND | StatusCode::FORBIDDEN)
                            ) =>
                        {
                            removed.push(key)
                        }
                        Err(error) => return Err(error),
                    }
                }
                let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
                if guard.generation != revision {
                    bail!("Sync superseded by a GitLab write; refresh again");
                }
                cache.finish(project.id, kind, plan, &removed)?;
            }
            Ok(items)
        };
        let mut items = if self.persistent.is_some() {
            // Show recent MRs even while a long issue history is being imported.
            std::thread::scope(|scope| -> Result<Vec<WorkItem>> {
                let issues = scope.spawn(|| fetch_kind(ItemKind::Issue));
                let mrs = fetch_kind(ItemKind::MergeRequest);
                let mut items = issues
                    .join()
                    .map_err(|_| anyhow!("List worker panicked"))??;
                items.extend(mrs?);
                Ok(items)
            })?
        } else {
            let mut items = fetch_kind(ItemKind::Issue)?;
            items.extend(fetch_kind(ItemKind::MergeRequest)?);
            items
        };
        if let Some(cache) = &self.persistent {
            items = cache.items(project.id)?;
        }
        report(
            SyncProgress {
                kind: ItemKind::MergeRequest,
                loaded: items.len(),
                total: None,
                page: 0,
                phase: "dashboard enrichment",
            },
            Vec::new(),
        );
        let mut enriched_keys = HashSet::new();
        if items.iter().any(needs_label_colors) {
            let labels = self.labels(project.id)?;
            for item in &mut items {
                if needs_label_colors(item) {
                    enriched_keys.insert(item.key.clone());
                }
                apply_label_colors(item, &labels);
            }
        }
        let mut candidates: Vec<usize> = items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                (item.key.kind == ItemKind::MergeRequest
                    && item.state == "opened"
                    && (self.persistent.is_some()
                        || item.pipeline.is_none()
                        || item.unresolved.is_none()))
                .then_some(i)
            })
            .collect();
        candidates.sort_by(|a, b| {
            items[*b]
                .updated_at
                .cmp(&items[*a].updated_at)
                .then_with(|| items[*b].key.iid.cmp(&items[*a].key.iid))
        });
        let mut discussion_pages = DASHBOARD_DISCUSSION_PAGES;
        for index in candidates.into_iter().take(DASHBOARD_ENRICHMENT) {
            enriched_keys.insert(items[index].key.clone());
            // Pipeline/discussion changes need not bump MR updated_at. Refresh the
            // bounded dashboard candidates independently of the list delta feed.
            if self.persistent.is_some() {
                items[index].pipeline = None;
                items[index].unresolved = None;
            }
            if items[index].pipeline.is_none() {
                match self.get::<ApiItem>(&self.item_url(&items[index].key, &[])) {
                    Ok(raw) => {
                        let enriched = raw.into_item(project.id, ItemKind::MergeRequest);
                        items[index].pipeline = enriched.pipeline;
                        items[index].unresolved = enriched.unresolved.or(items[index].unresolved);
                    }
                    Err(error) if unsupported_enrichment(&error) => continue,
                    Err(error) => return Err(error),
                }
            }
            if items[index].unresolved.is_none() {
                items[index].unresolved =
                    self.dashboard_discussions(&items[index].key, &mut discussion_pages)?;
            }
        }
        if let Some(cache) = &self.persistent {
            // Write only enriched identities, not the entire historical snapshot on
            // every small delta. Preserve synchronization cursors/identity markers.
            let enriched: Vec<_> = items
                .iter()
                .filter(|i| enriched_keys.contains(&i.key))
                .cloned()
                .collect();
            let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if guard.generation != revision {
                bail!("Sync superseded by a GitLab write; refresh again");
            }
            if !enriched.is_empty() {
                cache.enrich(&enriched)?;
            }
        }
        items.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.key.kind.segment().cmp(b.key.kind.segment()))
                .then_with(|| b.key.iid.cmp(&a.key.iid))
        });
        Ok(items)
    }

    pub fn details(&self, key: &ItemKey) -> Result<Details> {
        self.fetch_details(key, false, None, |_, _| {})
    }

    pub fn details_progress(
        &self,
        key: &ItemKey,
        previous: Option<Details>,
        progress: impl Fn(DetailPart, Details) + Sync,
    ) -> Result<Details> {
        self.fetch_details(key, true, previous, progress)
    }

    fn fetch_details(
        &self,
        key: &ItemKey,
        parallel: bool,
        previous: Option<Details>,
        progress: impl Fn(DetailPart, Details) + Sync,
    ) -> Result<Details> {
        let revision = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation;
        let raw: ApiItem = self.get(&self.item_url(key, &[]))?;
        let mut details = previous.unwrap_or_default();
        let colors: HashMap<_, _> = details
            .item
            .labels
            .iter()
            .map(|label| (label.name.clone(), label.clone()))
            .collect();
        details.item = raw.clone().into_item(key.project, key.kind);
        apply_label_colors(&mut details.item, &colors);
        details.warnings.clear();
        details.loaded.insert(DetailPart::Core);
        let shared = Mutex::new(details);
        let publish = |part, fresh: Option<Details>| -> Result<()> {
            let mut details = shared.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(fresh) = fresh {
                // Failed optional sections retain cached content; partial first loads
                // remain useful and carry a visible warning instead of a fake empty state.
                let section_failed = fresh.warnings.iter().any(|warning| match part {
                    DetailPart::Activity => warning.starts_with("Notes incomplete"),
                    _ => true,
                });
                let usable = !section_failed || !details.loaded.contains(&part);
                if part == DetailPart::Activity {
                    details.item.labels = fresh.item.labels.clone();
                }
                if usable {
                    match part {
                        DetailPart::Activity => details.notes = fresh.notes,
                        DetailPart::Discussions => {
                            details.discussions = fresh.discussions;
                            details.item.unresolved = fresh.item.unresolved;
                        }
                        DetailPart::Pipeline => {
                            details.jobs = fresh.jobs;
                            details.jobs_project = fresh.jobs_project;
                            details.item.pipeline = fresh.item.pipeline;
                        }
                        DetailPart::Changes => details.diffs = fresh.diffs,
                        DetailPart::Core => {}
                    }
                }
                details.loaded.insert(part);
                details.warnings.extend(fresh.warnings);
            }
            let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if guard.generation != revision {
                bail!("Detail request superseded by a GitLab write");
            }
            if let Some(cache) = &self.persistent {
                cache.save_detail(&details)?;
            }
            drop(guard);
            progress(part, details.clone());
            Ok(())
        };
        publish(DetailPart::Core, None)?;
        let mut parts = vec![DetailPart::Activity, DetailPart::Discussions];
        if key.kind == ItemKind::MergeRequest {
            parts.extend([DetailPart::Pipeline, DetailPart::Changes]);
        }
        if parallel {
            std::thread::scope(|scope| -> Result<()> {
                let handles: Vec<_> = parts
                    .into_iter()
                    .map(|part| {
                        let raw = &raw;
                        let publish = &publish;
                        scope.spawn(move || publish(part, Some(self.detail_part(key, raw, part))))
                    })
                    .collect();
                for handle in handles {
                    handle
                        .join()
                        .map_err(|_| anyhow!("Detail worker panicked"))??;
                }
                Ok(())
            })?;
        } else {
            for part in parts {
                publish(part, Some(self.detail_part(key, &raw, part)))?;
            }
        }
        Ok(shared.into_inner().unwrap_or_else(|e| e.into_inner()))
    }

    fn detail_part(&self, key: &ItemKey, raw: &ApiItem, part: DetailPart) -> Details {
        let mut details = Details {
            item: raw.clone().into_item(key.project, key.kind),
            ..Default::default()
        };
        match part {
            DetailPart::Activity => {
                // Optional label enrichment must not gate the first usable core or
                // unrelated sections. Share the activity worker to keep the cap at four.
                if needs_label_colors(&details.item) {
                    match self.labels(key.project) {
                        Ok(labels) => apply_label_colors(&mut details.item, &labels),
                        Err(error) => details
                            .warnings
                            .push(format!("Label colors unavailable: {error}")),
                    }
                }
                let mut notes = self.item_url(key, &["notes"]);
                notes
                    .query_pairs_mut()
                    .append_pair("order_by", "created_at")
                    .append_pair("sort", "desc");
                details.notes = self.optional(notes, "Notes", &mut details.warnings).0;
                details.notes.sort_by(|a, b| {
                    b.created_at
                        .cmp(&a.created_at)
                        .then_with(|| b.id.cmp(&a.id))
                });
            }
            DetailPart::Discussions => {
                let (discussions, complete) = self.optional::<Discussion>(
                    self.item_url(key, &["discussions"]),
                    "Discussions",
                    &mut details.warnings,
                );
                details.item.unresolved = complete.then(|| {
                    discussions
                        .iter()
                        .filter(|d| d.notes.iter().any(|n| n.resolvable && !n.resolved))
                        .count()
                });
                details.discussions = discussions;
            }
            DetailPart::Pipeline => {
                let mut pipeline = raw.head_pipeline.clone();
                let head_sha = raw
                    .sha
                    .clone()
                    .or_else(|| raw.diff_refs.as_ref().map(|refs| refs.head_sha.clone()));
                if pipeline.is_none() {
                    if let Some(sha) = head_sha.filter(|sha| !sha.is_empty()) {
                        let (pipelines, complete) = self.optional::<ApiPipeline>(
                            self.item_url(key, &["pipelines"]),
                            "Head pipelines",
                            &mut details.warnings,
                        );
                        if complete {
                            pipeline = pipelines
                                .into_iter()
                                .filter(|p| p.sha.as_deref() == Some(sha.as_str()))
                                .max_by_key(|p| p.pipeline.id);
                            if let Some(summary) = &pipeline {
                                let project = summary.project_id.unwrap_or(key.project);
                                match self.get::<ApiPipeline>(&self.url(&[
                                    "projects",
                                    &project.to_string(),
                                    "pipelines",
                                    &summary.pipeline.id.to_string(),
                                ])) {
                                    Ok(full) => pipeline = Some(full),
                                    Err(error) => details.warnings.push(format!(
                                        "Pipeline details unavailable; using summary: {error}"
                                    )),
                                }
                            }
                        }
                    } else {
                        details.warnings.push("Head pipeline unknown: GitLab supplied neither head_pipeline nor a current head SHA".into());
                    }
                }
                if let Some(pipeline) = pipeline {
                    let project = pipeline.project_id.unwrap_or(key.project);
                    details.jobs_project = Some(project);
                    let mut jobs = self.url(&[
                        "projects",
                        &project.to_string(),
                        "pipelines",
                        &pipeline.pipeline.id.to_string(),
                        "jobs",
                    ]);
                    jobs.query_pairs_mut()
                        .append_pair("include_retried", "false");
                    details.jobs = self
                        .optional(jobs, "Pipeline jobs", &mut details.warnings)
                        .0;
                    sort_jobs(&mut details.jobs);
                    details.item.pipeline = Some(pipeline.pipeline);
                }
            }
            DetailPart::Changes => {
                details.diffs = self
                    .optional::<Diff>(
                        self.item_url(key, &["diffs"]),
                        "Diffs",
                        &mut details.warnings,
                    )
                    .0;
                if details
                    .diffs
                    .iter()
                    .any(|diff| diff.too_large || diff.collapsed)
                {
                    details.warnings.push(
                    "Some diffs are collapsed or too large on GitLab; patch content is incomplete"
                        .into(),
                );
                }
                if let Some(count) = raw.changes_count.clone() {
                    let lower_bound = count.trim_end_matches('+').parse::<usize>().ok();
                    if count.ends_with('+') || lower_bound.is_some_and(|n| n > details.diffs.len())
                    {
                        details.warnings.push(format!("GitLab reports {} changed files but returned {} diffs; server diff limits may apply", self.redacted(&count), details.diffs.len()));
                    }
                }
            }
            DetailPart::Core => {}
        }
        details
    }

    /// Supported edits: title, description, labels/add_labels/remove_labels, state_event
    /// (close/reopen), assignee_id/assignee_ids, milestone_id, discussion_locked; plus
    /// reviewer_ids/target_branch/draft for MRs and iteration_id/epic_id/due_date/confidential
    /// for issues. Values are text except comma-separated assignee/reviewer IDs, numeric
    /// single IDs (empty clears), and true/false booleans. Unknown fields are rejected.
    pub fn mutate(&self, mutation: Mutation) -> Result<()> {
        if let Mutation::Edit { key, field, value } = &mutation
            && field == "iteration_id"
        {
            let id = edit_value(key, field, value)?;
            return self.set_iteration(key, id.as_u64().filter(|id| *id > 0));
        }
        let (method, url, body) = match mutation {
            Mutation::Edit { key, field, value } => {
                let value = edit_value(&key, &field, &value)?;
                let mut body = serde_json::Map::new();
                body.insert(field, value);
                (Method::PUT, self.item_url(&key, &[]), Value::Object(body))
            }
            Mutation::Comment { key, body } => (
                Method::POST,
                self.item_url(&key, &["notes"]),
                json!({"body": body}),
            ),
            Mutation::Reply {
                key,
                discussion,
                body,
            } => {
                validate_discussion(&discussion)?;
                (
                    Method::POST,
                    self.item_url(&key, &["discussions", &discussion, "notes"]),
                    json!({"body": body}),
                )
            }
            Mutation::Resolve {
                key,
                discussion,
                resolved,
            } => {
                validate_discussion(&discussion)?;
                if key.kind != ItemKind::MergeRequest {
                    bail!("Only merge-request discussions can be resolved");
                }
                (
                    Method::PUT,
                    self.item_url(&key, &["discussions", &discussion]),
                    json!({"resolved": resolved}),
                )
            }
            Mutation::RetryJob { project, job } => (
                Method::POST,
                self.url(&[
                    "projects",
                    &project.to_string(),
                    "jobs",
                    &job.to_string(),
                    "retry",
                ]),
                json!({}),
            ),
            Mutation::CreateIssue {
                project,
                title,
                description,
            } => (
                Method::POST,
                self.url(&["projects", &project.to_string(), "issues"]),
                json!({"title": title, "description": description}),
            ),
            Mutation::CreateMergeRequest {
                project,
                title,
                source,
                target,
                description,
            } => (
                Method::POST,
                self.url(&["projects", &project.to_string(), "merge_requests"]),
                json!({"title": title, "source_branch": source, "target_branch": target, "description": description}),
            ),
        };
        let response = self.send(method.clone(), &url, HeaderMap::new(), Some(&body))?;
        if !response.status().is_success() {
            return Err(self.mutation_failure(&method, &url, &body, response));
        }
        self.written()
    }

    /// Drop cached reads after a successful write.
    fn written(&self) -> Result<()> {
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        guard.invalidate();
        if let Some(cache) = &self.persistent {
            cache.invalidate_details().map_err(|error| anyhow!("GitLab write succeeded, but local cache invalidation failed: {error:#}. Do not repeat the write; clear the local cache."))?;
        }
        Ok(())
    }

    /// A failed write explained in full: status, what was sent, and GitLab's own words.
    /// The first line stays the compact summary; the rest is shown in the error dialog.
    fn mutation_failure(
        &self,
        method: &Method,
        url: &Url,
        body: &Value,
        response: Response,
    ) -> anyhow::Error {
        let summary = self.status_error(method, url, &response);
        let status = response.status();
        let mut bytes = Vec::new();
        let _ = response.take(4096).read_to_end(&mut bytes);
        let text = String::from_utf8_lossy(&bytes);
        let text: String = text
            .chars()
            .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
            .collect();
        let sent = body.to_string();
        let sent: String = if sent.chars().count() > 300 {
            sent.chars().take(300).chain("…".chars()).collect()
        } else {
            sent
        };
        let detail = format!(
            "{summary}\nRequest: {method} {}\nBody: {}\nResponse: {}",
            self.redacted(url.path()),
            self.redacted(&sent),
            if text.trim().is_empty() {
                "(empty)".to_owned()
            } else {
                self.redacted(text.trim())
            }
        );
        HttpFailure {
            status,
            message: detail,
        }
        .into()
    }

    /// The REST issue-update endpoint has no `iteration_id` parameter (it is only a list
    /// filter), so a lone `iteration_id` is rejected as "no valid parameter" with HTTP 400.
    /// Iterations are assigned with the GraphQL `issueSetIteration` mutation instead.
    fn set_iteration(&self, key: &ItemKey, iteration: Option<u64>) -> Result<()> {
        #[derive(Deserialize)]
        struct ProjectPath {
            path_with_namespace: String,
        }
        let project: ProjectPath = self.get(&self.url(&["projects", &key.project.to_string()]))?;
        let mut url = self.api.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow!("Invalid GitLab URL path"))?
            .pop_if_empty()
            .pop()
            .push("graphql");
        let body = json!({
            "query": "mutation($input: IssueSetIterationInput!) { issueSetIteration(input: $input) { errors } }",
            "variables": {"input": {
                "projectPath": project.path_with_namespace,
                "iid": key.iid.to_string(),
                "iterationId": iteration.map(|id| format!("gid://gitlab/Iteration/{id}")),
            }},
        });
        let response = self.send(Method::POST, &url, HeaderMap::new(), Some(&body))?;
        if !response.status().is_success() {
            return Err(self.mutation_failure(&Method::POST, &url, &body, response));
        }
        let mut bytes = Vec::new();
        response
            .take(JSON_LIMIT as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("Could not read the GraphQL response"))?;
        let reply: Value = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("GraphQL returned a response that is not JSON"))?;
        let messages = |value: Option<&Value>| -> Vec<String> {
            value
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|error| {
                    error
                        .get("message")
                        .unwrap_or(error)
                        .as_str()
                        .map_or_else(|| error.to_string(), str::to_owned)
                })
                .collect()
        };
        let mut errors = messages(reply.get("errors"));
        errors.extend(messages(reply.pointer("/data/issueSetIteration/errors")));
        if errors.is_empty()
            && !reply
                .pointer("/data/issueSetIteration")
                .is_some_and(Value::is_object)
        {
            errors.push("GraphQL returned no issueSetIteration result".into());
        }
        if !errors.is_empty() {
            bail!(
                "{}: GraphQL issueSetIteration failed\nRequest: POST {}\nBody: {}\nResponse: {}",
                self.context(&Method::POST, &url),
                self.redacted(url.path()),
                self.redacted(&body.to_string()),
                self.redacted(&errors.join("; "))
            );
        }
        self.written()
    }

    pub fn trace(&self, project: u64, job: u64, offset: u64) -> Result<TraceChunk> {
        let url = self.url(&[
            "projects",
            &project.to_string(),
            "jobs",
            &job.to_string(),
            "trace",
        ]);
        let state = {
            let mut states = self.traces.lock().unwrap_or_else(|e| e.into_inner());
            let previous = states
                .iter()
                .position(|state| state.project == project && state.job == job)
                .and_then(|index| states.remove(index));
            let state = previous.unwrap_or_else(|| {
                Arc::new(TraceState {
                    project,
                    job,
                    cursor: Mutex::new(TraceCursor::default()),
                })
            });
            if states.len() == TRACE_STATES {
                // Do not evict an in-flight job and create a second parser/lock for it.
                if let Some(idle) = states
                    .iter()
                    .position(|state| Arc::strong_count(state) == 1)
                {
                    states.remove(idle);
                } else {
                    bail!(
                        "Trace parser capacity reached (64 active jobs); retry when another read finishes"
                    );
                }
            }
            states.push_back(state.clone());
            state
        };
        let mut cursor = state.cursor.lock().unwrap_or_else(|e| e.into_inner());
        let lost_state = offset != 0 && cursor.offset != offset;
        let (bytes, start, reset) =
            self.trace_bytes(&url, if lost_state { 0 } else { offset }, lost_state)?;
        let next_offset = start
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| anyhow!("Trace byte offset overflow"))?;
        if start == 0 || reset {
            cursor.sanitizer = Sanitizer::default();
        }
        let text = cursor.sanitizer.feed(&bytes);
        cursor.offset = next_offset;
        Ok(TraceChunk {
            text,
            next_offset,
            reset,
        })
    }

    fn trace_bytes(
        &self,
        url: &Url,
        offset: u64,
        restarting: bool,
    ) -> Result<(Vec<u8>, u64, bool)> {
        if self.supports_trace_byte_params(url)? {
            let bytes = self.trace_parameter_bytes(url, offset, TRACE_BYTES)?;
            // An empty slice can mean EOF or a shortened log. Check the preceding byte
            // before deciding to restart, retaining the legacy truncation semantics.
            if bytes.is_empty()
                && offset > 0
                && self.trace_parameter_bytes(url, offset - 1, 1)?.is_empty()
            {
                return self.trace_bytes(url, 0, true);
            }
            return Ok((bytes, offset, restarting));
        }
        self.trace_range_bytes(url, offset, restarting)
    }

    fn supports_trace_byte_params(&self, url: &Url) -> Result<bool> {
        match self.trace_byte_params.load(Ordering::Relaxed) {
            1 => return Ok(true),
            2 => return Ok(false),
            _ => {}
        }
        // Neither status 200 nor Content-Length distinguishes a slice from a full
        // trace. Probe both parameters rather than guessing from the actual poll.
        let first = match self.trace_parameter_bytes(url, 0, 1) {
            Ok(bytes) => bytes,
            Err(error) if http_status(&error) == Some(StatusCode::BAD_REQUEST) => {
                self.trace_byte_params.store(2, Ordering::Relaxed);
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if first.is_empty() {
            // An empty job cannot establish support. Retry on a later poll.
            return Ok(false);
        }
        // A one-byte response proves only the limit. Request beyond any realistic
        // trace to verify the offset too (including a one-byte legacy job log).
        let supported = if first.len() == 1 {
            match self.trace_parameter_bytes(url, i64::MAX as u64, 1) {
                Ok(bytes) => bytes.is_empty(),
                Err(error) if http_status(&error) == Some(StatusCode::BAD_REQUEST) => false,
                Err(error) => return Err(error),
            }
        } else {
            false
        };
        self.trace_byte_params
            .store(if supported { 1 } else { 2 }, Ordering::Relaxed);
        Ok(supported)
    }

    fn trace_parameter_bytes(&self, url: &Url, offset: u64, limit: usize) -> Result<Vec<u8>> {
        let mut url = url.clone();
        url.query_pairs_mut()
            .append_pair("byte_offset", &offset.to_string())
            .append_pair("byte_limit", &limit.to_string());
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let response = self.send(Method::GET, &url, headers, None)?;
        if !response.status().is_success() {
            return Err(self.status_error(&Method::GET, &url, &response));
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|h| h != "identity")
        {
            bail!(
                "{}: compressed trace would make byte offsets ambiguous",
                self.context(&Method::GET, &url)
            );
        }
        let mut bytes = Vec::new();
        // One extra byte makes ignored byte_limit observable without downloading
        // the entire trace. Normal polls remain bounded to TRACE_BYTES.
        response
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                anyhow!(
                    "{}: could not read trace body",
                    self.context(&Method::GET, &url)
                )
            })?;
        if bytes.len() > limit && limit != 1 {
            bail!(
                "{}: trace body exceeds byte_limit",
                self.context(&Method::GET, &url)
            );
        }
        Ok(bytes)
    }

    fn trace_range_bytes(
        &self,
        url: &Url,
        offset: u64,
        restarting: bool,
    ) -> Result<(Vec<u8>, u64, bool)> {
        let end = offset.saturating_add(TRACE_BYTES as u64 - 1);
        let mut headers = HeaderMap::new();
        headers.insert(
            RANGE,
            HeaderValue::from_str(&format!("bytes={offset}-{end}")).expect("numeric range"),
        );
        headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let mut response = self.send(Method::GET, url, headers, None)?;
        let error = |message: &str| anyhow!("{}: {message}", self.context(&Method::GET, url));
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            let total = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
                .and_then(|s| s.strip_prefix("bytes */"))
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| error("416 without a valid Content-Range length"))?;
            if offset == total {
                return Ok((Vec::new(), offset, restarting));
            }
            if offset > total && !restarting {
                return self.trace_bytes(url, 0, true);
            }
            return Err(error("inconsistent 416 Content-Range"));
        }
        if !response.status().is_success() {
            return Err(self.status_error(&Method::GET, url, &response));
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|h| h != "identity")
        {
            return Err(error("compressed trace would make byte offsets ambiguous"));
        }
        match response.status() {
            StatusCode::PARTIAL_CONTENT => {
                let range = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|h| h.to_str().ok())
                    .and_then(parse_content_range)
                    .ok_or_else(|| error("206 without a valid Content-Range"))?;
                if range.0 != offset {
                    return Err(error("Content-Range starts at the wrong byte offset"));
                }
                let expected = range
                    .1
                    .checked_sub(range.0)
                    .and_then(|n| n.checked_add(1))
                    .ok_or_else(|| error("Content-Range overflow"))?
                    .min(TRACE_BYTES as u64);
                let mut bytes = Vec::new();
                response
                    .take(expected)
                    .read_to_end(&mut bytes)
                    .map_err(|_| error("could not read trace body"))?;
                if bytes.len() as u64 != expected {
                    return Err(error("trace body shorter than Content-Range"));
                }
                Ok((bytes, offset, restarting))
            }
            StatusCode::OK => {
                let length = response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                if length == Some(offset) {
                    return Ok((Vec::new(), offset, restarting));
                }
                let reset = restarting || length.is_some_and(|len| len < offset);
                let start = if reset { 0 } else { offset };
                if start > TRACE_SKIP_LIMIT {
                    return Err(error(
                        "server ignored Range; offset exceeds the explicit 8 MiB fallback scan limit",
                    ));
                }
                let mut skipped = 0;
                let mut prefix = Vec::new();
                let mut buffer = [0u8; 8192];
                while skipped < start {
                    let read_len = (start - skipped).min(buffer.len() as u64) as usize;
                    let n = response
                        .read(&mut buffer[..read_len])
                        .map_err(|_| error("could not scan trace prefix"))?;
                    if n == 0 {
                        return Ok((prefix, 0, true));
                    }
                    let keep = n.min(TRACE_BYTES - prefix.len());
                    prefix.extend_from_slice(&buffer[..keep]);
                    skipped += n as u64;
                }
                let mut bytes = Vec::new();
                response
                    .take(TRACE_BYTES as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|_| error("could not read trace body"))?;
                Ok((bytes, start, reset))
            }
            StatusCode::NO_CONTENT => Ok((Vec::new(), offset, restarting)),
            _ => Err(error(
                "unexpected successful trace status (expected 200 or 206)",
            )),
        }
    }
}

fn with_page(url: &Url, page: u64) -> Url {
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key != "page")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut next = url.clone();
    next.set_query(None);
    next.query_pairs_mut()
        .extend_pairs(pairs)
        .append_pair("page", &page.to_string());
    next
}

fn validate_discussion(discussion: &str) -> Result<()> {
    if discussion.is_empty() || matches!(discussion, "." | "..") {
        bail!("Invalid discussion ID");
    }
    Ok(())
}

fn edit_value(key: &ItemKey, field: &str, value: &str) -> Result<Value> {
    let mr = key.kind == ItemKind::MergeRequest;
    match field {
        "title" | "description" | "labels" | "add_labels" | "remove_labels" => Ok(json!(value)),
        "target_branch" if mr => Ok(json!(value)),
        "due_date" if !mr => Ok(json!(value)),
        "state_event" if matches!(value, "close" | "reopen") => Ok(json!(value)),
        "assignee_ids" | "reviewer_ids" if field == "assignee_ids" || mr => {
            let ids: Result<Vec<u64>> = value
                .split(',')
                .filter(|part| !part.trim().is_empty())
                .map(|part| {
                    part.trim()
                        .parse::<u64>()
                        .map_err(|_| anyhow!("{field} requires comma-separated numeric IDs"))
                })
                .collect();
            Ok(json!(ids?))
        }
        "milestone_id" | "assignee_id" | "iteration_id" | "epic_id"
            if !matches!(field, "iteration_id" | "epic_id") || !mr =>
        {
            let id = if value.trim().is_empty() {
                0
            } else {
                value
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| anyhow!("{field} requires a numeric ID (empty clears)"))?
            };
            Ok(json!(id))
        }
        "discussion_locked" | "confidential" | "draft"
            if field == "discussion_locked" || (field == "draft") == mr =>
        {
            Ok(json!(
                value
                    .parse::<bool>()
                    .map_err(|_| anyhow!("{field} requires true or false"))?
            ))
        }
        _ => bail!("Unsupported edit field or value for this item kind"),
    }
}

#[derive(Clone, Deserialize)]
struct ApiPipeline {
    #[serde(flatten)]
    pipeline: Pipeline,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    project_id: Option<u64>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum ApiLabel {
    Name(String),
    Detail(Label),
}

#[derive(Clone, Deserialize)]
struct Title {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    iid: Option<u64>,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    state: Option<Value>,
}

#[derive(Clone, Deserialize)]
struct DiffRefs {
    head_sha: String,
}

#[derive(Clone, Deserialize)]
struct ApiItem {
    iid: u64,
    title: String,
    state: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    labels: Vec<ApiLabel>,
    #[serde(default)]
    author: Option<User>,
    #[serde(default)]
    assignees: Vec<User>,
    #[serde(default)]
    reviewers: Vec<User>,
    #[serde(default)]
    milestone: Option<Title>,
    #[serde(default)]
    iteration: Option<Title>,
    #[serde(default)]
    epic: Option<Title>,
    #[serde(default)]
    source_branch: Option<String>,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    web_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    work_in_progress: bool,
    #[serde(default)]
    head_pipeline: Option<ApiPipeline>,

    #[serde(default)]
    unresolved_discussions_count: Option<usize>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    diff_refs: Option<DiffRefs>,
    #[serde(default)]
    changes_count: Option<String>,
}

impl ApiItem {
    fn into_item(self, project: u64, kind: ItemKind) -> WorkItem {
        WorkItem {
            key: ItemKey {
                project,
                iid: self.iid,
                kind,
            },
            title: self.title,
            description: self.description.unwrap_or_default(),
            state: self.state,
            labels: self
                .labels
                .into_iter()
                .map(|label| {
                    let label = match label {
                        ApiLabel::Name(name) => Label {
                            name,
                            ..Label::default()
                        },
                        ApiLabel::Detail(label) => label,
                    };
                    label_contrast(label)
                })
                .collect(),
            author: self.author.unwrap_or_default(),
            assignees: self.assignees,
            reviewers: self.reviewers,
            milestone_id: self.milestone.as_ref().and_then(|m| m.id),
            iteration_id: self.iteration.as_ref().and_then(|i| i.id),
            epic_id: self.epic.as_ref().and_then(|e| e.id),
            epic: self
                .epic
                .map(|e| {
                    e.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
                        e.iid
                            .or(e.id)
                            .map(|id| format!("Epic &{id}"))
                            .unwrap_or_default()
                    })
                })
                .unwrap_or_default(),
            milestone: self
                .milestone
                .map(|m| {
                    m.title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
                        m.id.map(|id| format!("Milestone #{id}"))
                            .unwrap_or_default()
                    })
                })
                .unwrap_or_default(),
            iteration: self
                .iteration
                .map(|i| {
                    if i.start_date.is_some() || i.due_date.is_some() {
                        let current = iteration_is_current(
                            i.state.as_ref(),
                            i.start_date.as_deref(),
                            i.due_date.as_deref(),
                            chrono::Local::now().date_naive(),
                        );
                        iteration_label(
                            i.title.as_deref(),
                            i.start_date.as_deref(),
                            i.due_date.as_deref(),
                            current,
                        )
                    } else {
                        i.title
                            .filter(|t| !t.trim().is_empty())
                            .unwrap_or_else(|| "Undated".to_owned())
                    }
                })
                .unwrap_or_default(),
            source_branch: self.source_branch.unwrap_or_default(),
            target_branch: self.target_branch.unwrap_or_default(),
            updated_at: self.updated_at,
            web_url: self.web_url,
            draft: self.draft || self.work_in_progress,
            pipeline: self.head_pipeline.map(|p| p.pipeline),
            // A boolean about merge-blocking discussions is not an exact thread count.
            unresolved: self.unresolved_discussions_count,
        }
    }
}

fn needs_label_colors(item: &WorkItem) -> bool {
    item.labels.iter().any(|label| label.color.is_empty())
}

fn apply_label_colors(item: &mut WorkItem, labels: &HashMap<String, Label>) {
    for label in &mut item.labels {
        if label.color.is_empty()
            && let Some(full) = labels.get(&label.name)
        {
            *label = label_contrast(full.clone());
        }
    }
}

fn label_contrast(mut label: Label) -> Label {
    if label.text_color.is_empty()
        && let Some(rgb) = label
            .color
            .strip_prefix('#')
            .filter(|s| s.len() == 6)
            .and_then(|s| u32::from_str_radix(s, 16).ok())
    {
        let brightness = ((rgb >> 16) & 255) * 299 + ((rgb >> 8) & 255) * 587 + (rgb & 255) * 114;
        label.text_color = if brightness >= 128_000 {
            "#000000"
        } else {
            "#ffffff"
        }
        .into();
    }
    label
}

fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let (span, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let start = start.parse().ok()?;
    let end = end.parse().ok()?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };
    if end < start || total.is_some_and(|total| end >= total) {
        return None;
    }
    Some((start, end, total))
}

struct TraceState {
    project: u64,
    job: u64,
    cursor: Mutex<TraceCursor>,
}

#[derive(Default)]
struct TraceCursor {
    offset: u64,
    sanitizer: Sanitizer,
}

#[derive(Default)]
struct Sanitizer {
    mode: EscapeMode,
    utf8: Vec<u8>,
    csi: String,
    csi_valid: bool,
}

#[derive(Default, Clone, Copy)]
enum EscapeMode {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

impl Sanitizer {
    fn feed(&mut self, bytes: &[u8]) -> String {
        let mut input = std::mem::take(&mut self.utf8);
        input.extend_from_slice(bytes);
        let mut remaining = input.as_slice();
        let mut text = String::new();
        while !remaining.is_empty() {
            match std::str::from_utf8(remaining) {
                Ok(valid) => {
                    for c in valid.chars() {
                        self.character(c, &mut text);
                    }
                    break;
                }
                Err(error) => {
                    let valid = std::str::from_utf8(&remaining[..error.valid_up_to()])
                        .expect("valid UTF-8 prefix");
                    for c in valid.chars() {
                        self.character(c, &mut text);
                    }
                    remaining = &remaining[error.valid_up_to()..];
                    if let Some(length) = error.error_len() {
                        // Recognize legacy single-byte C1 controls without corrupting UTF-8 continuation bytes.
                        let c = if length == 1 && (0x80..=0x9f).contains(&remaining[0]) {
                            remaining[0] as char
                        } else {
                            '\u{fffd}'
                        };
                        self.character(c, &mut text);
                        remaining = &remaining[length..];
                    } else {
                        // EOF is only a snapshot for a growing job trace. Hold at most three
                        // bytes until the next poll, even when this response reached EOF.
                        self.utf8.extend_from_slice(remaining);
                        break;
                    }
                }
            }
        }
        text
    }

    fn start_csi(&mut self) {
        self.mode = EscapeMode::Csi;
        self.csi.clear();
        self.csi_valid = true;
    }

    fn character(&mut self, c: char, text: &mut String) {
        use EscapeMode::*;
        if c == '\u{9c}' {
            self.mode = Ground;
            return;
        }
        match self.mode {
            Ground => match c {
                '\u{1b}' => self.mode = Escape,
                '\u{9b}' => self.start_csi(),
                '\u{9d}' => self.mode = Osc,
                '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => self.mode = String,
                '\n' | '\t' => text.push(c),
                c if !c.is_control() => text.push(c),
                _ => {}
            },
            Escape => {
                if c == '[' {
                    self.start_csi();
                    return;
                }
                self.mode = match c {
                    ']' => Osc,
                    'P' | 'X' | '^' | '_' => String,
                    '\u{1b}' | '\u{20}'..='\u{2f}' => Escape,
                    _ => Ground,
                }
            }
            Csi => match c {
                '\u{1b}' => self.mode = Escape,
                '@'..='~' => {
                    if c == 'm'
                        && self.csi_valid
                        && let Some(params) = crate::ansi::normalize_sgr(&self.csi)
                    {
                        text.push_str("\x1b[");
                        text.push_str(&params);
                        text.push('m');
                    }
                    self.mode = Ground;
                    self.csi.clear();
                }
                c if ('\u{20}'..='\u{3f}').contains(&c)
                    && self.csi.len() < crate::ansi::SGR_LIMIT =>
                {
                    self.csi.push(c)
                }
                _ => self.csi_valid = false,
            },
            Osc => match c {
                '\u{7}' => self.mode = Ground,
                '\u{1b}' => self.mode = OscEscape,
                _ => {}
            },
            String => {
                if c == '\u{1b}' {
                    self.mode = StringEscape;
                }
            }
            OscEscape => {
                self.mode = match c {
                    '\\' | '\u{7}' => Ground,
                    '\u{1b}' => OscEscape,
                    _ => Osc,
                }
            }
            StringEscape => {
                self.mode = match c {
                    '\\' => Ground,
                    '\u{1b}' => StringEscape,
                    _ => String,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
        sync::atomic::{AtomicBool, Ordering},
        thread::{self, JoinHandle},
    };

    #[test]
    fn lookups_use_scoped_search_encoded_queries_and_bounded_pages() {
        let mock = Mock::new(|request| {
            let url = request.url();
            let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(params["per_page"], "20");
            assert_eq!(params["page"], "1");
            assert_eq!(params["search"], "Équipe & sprint/next");
            let value = if url.path().ends_with("/users") {
                assert_eq!(url.path(), "/gitlab/api/v4/projects/7/users");
                json!({"id": 90, "name": "Alex", "username": "alex"})
            } else if url.path().ends_with("/projects") {
                assert_eq!(params["membership"], "true");
                assert_eq!(params["search_namespaces"], "true");
                json!({"id": 50, "name": "Service", "path_with_namespace": "deep/group/service"})
            } else if url.path().ends_with("/labels") {
                assert_eq!(params["include_ancestor_groups"], "true");
                json!({"name": "team::platform", "color": "#123456", "text_color": "#ffffff"})
            } else {
                assert_eq!(params["include_ancestors"], "true");
                if url.path().ends_with("/iterations") {
                    assert!(
                        url.query_pairs()
                            .any(|(k, v)| k == "in[]" && v == "cadence_title")
                    );
                }
                json!({"id": 42, "title": "Sprint", "group_id": 6, "start_date": "2026-10-01", "due_date": "2026-10-14"})
            };
            Reply::json(Value::Array(vec![value; 25])).header("X-Next-Page", "2")
        });
        let api = GitLab::new(&format!("{}/gitlab", mock.base), "test-private-secret").unwrap();
        for kind in [
            LookupKind::Users,
            LookupKind::Milestones,
            LookupKind::Iterations,
            LookupKind::Projects,
            LookupKind::Labels,
        ] {
            let matches = api.lookup(7, kind, "  Équipe & sprint/next  ").unwrap();
            assert_eq!(matches.len(), 20);
            assert!(matches[0].id > 0);
        }
        assert_eq!(
            mock.requests.lock().unwrap().len(),
            5,
            "typeahead must not paginate all history"
        );
    }

    #[test]
    fn lookups_keep_duplicate_names_distinct_and_handle_untitled_iterations() {
        let mock = Mock::new(|request| {
            if request.url().path().ends_with("/users") {
                assert!(
                    request
                        .url()
                        .query_pairs()
                        .any(|(k, v)| k == "search" && v == "alex")
                );
                Reply::json(json!([
                    {"id":1,"name":"Alex","username":"alex-one"},
                    {"id":2,"name":"Alex","username":"alex-two"}
                ]))
            } else if request.url().path().ends_with("/labels") {
                Reply::json(json!([
                    {"name":"bug","color":"#d9534f","text_color":"#ffffff"},
                    {"name":"bug","color":"#d9534f","text_color":"#ffffff"}
                ]))
            } else {
                Reply::json(
                    json!([{"id":9,"title":null,"start_date":"2026-10-01","due_date":"2026-10-14","group_id":6}]),
                )
            }
        });
        let users = mock.client().lookup(7, LookupKind::Users, "@alex").unwrap();
        assert_ne!(users[0].value, users[1].value);
        assert_eq!(users[0].label, users[1].label);
        let iterations = mock.client().lookup(7, LookupKind::Iterations, "").unwrap();
        assert!(iterations[0].label.contains("2026"));
        assert!(!iterations[0].label.contains("Iteration #"));
        let labels = mock.client().lookup(7, LookupKind::Labels, "bug").unwrap();
        assert_eq!(labels[0].value, "bug");
        assert_eq!(labels[0].api_value, "bug");
        assert_eq!(labels[0].color, "#d9534f");
        let limited = Mock::new(|_| Reply::bytes(429, Vec::new()).header("Retry-After", "30"));
        assert!(
            limited
                .client()
                .lookup(7, LookupKind::Users, "alex")
                .unwrap_err()
                .to_string()
                .contains("Retry-After: 30")
        );
    }

    #[test]
    fn epic_lookup_searches_project_group_and_ancestors_with_distinct_identities() {
        let mock = Mock::new(|request| {
            let url = request.url();
            match url.path() {
                "/api/v4/projects/7" => Reply::json(json!({"namespace":{"id":60,"kind":"group"}})),
                "/api/v4/groups/60/epics" => {
                    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
                    assert_eq!(params["include_ancestor_groups"], "true");
                    assert_eq!(params["include_descendant_groups"], "false");
                    assert_eq!(params["search"], "Équipe & next");
                    assert_eq!(params["per_page"], "20");
                    assert_eq!(params["page"], "1");
                    Reply::json(json!([
                        {"id":900,"iid":1,"group_id":60,"title":"Same title"},
                        {"id":901,"iid":1,"group_id":6,"title":"Same title"}
                    ]))
                }
                _ => panic!("unexpected request: {}", request.target),
            }
        });
        let options = mock
            .client()
            .lookup(7, LookupKind::Epics, "Équipe & next")
            .unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].api_value, "900");
        assert_eq!(options[1].api_value, "901");
        assert_ne!(options[0].value, options[1].value);
        assert!(options[1].description.contains("group 6"));
    }

    #[test]
    fn epic_lookup_handles_personal_namespaces_and_reports_unavailable_features() {
        let personal = Mock::new(|request| {
            assert_eq!(request.url().path(), "/api/v4/projects/7");
            Reply::json(json!({"namespace":{"id":60,"kind":"user"}}))
        });
        assert!(
            personal
                .client()
                .lookup(7, LookupKind::Epics, "")
                .unwrap()
                .is_empty()
        );
        assert_eq!(personal.requests.lock().unwrap().len(), 1);
        for status in [403, 404] {
            let mock = Mock::new(move |request| {
                if request.url().path() == "/api/v4/projects/7" {
                    Reply::json(json!({"namespace":{"id":60,"kind":"group"}}))
                } else {
                    Reply::bytes(status, b"unavailable".to_vec())
                }
            });
            let error = mock
                .client()
                .lookup(7, LookupKind::Epics, "")
                .unwrap_err()
                .to_string();
            assert!(error.contains(&format!("HTTP {status}")), "{error}");
        }
    }

    #[test]
    fn epic_edits_use_global_ids_and_zero_to_clear_and_reject_mr_edits() {
        let mock = Mock::new(|_| Reply::bytes(204, Vec::new()));
        let client = mock.client();
        for value in ["900", ""] {
            client
                .mutate(Mutation::Edit {
                    key: key(ItemKind::Issue),
                    field: "epic_id".into(),
                    value: value.into(),
                })
                .unwrap();
        }
        for (kind, value) in [
            (ItemKind::MergeRequest, "900"),
            (ItemKind::Issue, "not-an-id"),
        ] {
            assert!(
                client
                    .mutate(Mutation::Edit {
                        key: key(kind),
                        field: "epic_id".into(),
                        value: value.into(),
                    })
                    .is_err()
            );
        }
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "PUT");
        assert_eq!(requests[0].url().path(), "/api/v4/projects/7/issues/1");
        assert_eq!(requests[0].json(), json!({"epic_id":900}));
        assert_eq!(requests[1].json(), json!({"epic_id":0}));
    }

    #[test]
    fn current_iteration_uses_symbolic_current_state() {
        let mock = Mock::new(|request| {
            let params: HashMap<_, _> = request.url().query_pairs().into_owned().collect();
            assert_eq!(request.url().path(), "/api/v4/projects/7/iterations");
            assert_eq!(params["include_ancestors"], "true");
            assert_eq!(params["state"], "current");
            Reply::json(json!([
                {"id":42,"title":"Sprint","start_date":"2026-10-01","due_date":"2026-10-14","group_id":6,
                 "web_url":"https://gl.example/groups/acme/platform/-/iterations/3"}
            ]))
        });
        let current = mock.client().current_iteration(7).unwrap().unwrap();
        assert_eq!(current.id, 42);
        assert!(current.title.starts_with("Sprint ("), "{}", current.title);
        assert!(current.title.contains("2026"), "{}", current.title);
        assert!(current.title.ends_with("(Current)"), "{}", current.title);
        assert_eq!(current.description, "Current iteration · acme/platform");
    }

    #[test]
    fn iteration_labels_show_dates_group_and_state_without_ids() {
        let entries: Vec<IterationMatch> = serde_json::from_value(json!([
            {"id":84001,"iid":84,"title":null,"start_date":"2026-09-28","due_date":"2026-10-11","state":2,
             "web_url":"https://gl.example/groups/acme/platform/-/iterations/84"},
            {"id":84002,"iid":85,"title":null,"start_date":"2026-10-12","due_date":"2026-10-25","state":"upcoming",
             "web_url":"https://gl.example/groups/acme/-/iterations/85"}
        ]))
        .unwrap();
        let current = entries[0].lookup_option();
        assert!(current.label.ends_with(" (Current)"), "{}", current.label);
        assert!(!current.label.contains("Iteration #"));
        assert_eq!(current.description, "Current iteration · acme/platform");
        assert_eq!(current.api_value, "84001");
        let upcoming = entries[1].lookup_option();
        assert!(!upcoming.label.contains("Current"));
        assert_eq!(upcoming.description, "Upcoming · acme");
        assert!(!upcoming.description.contains("8400"));
    }

    #[test]
    fn issue_iteration_shows_dates_instead_of_a_bare_number() {
        let mut value = raw_item(1, "opened");
        value["iteration"] = json!({"id":84001,"iid":84,"title":null,"state":2,
            "start_date":"2026-09-28","due_date":"2026-10-11"});
        let item = serde_json::from_value::<ApiItem>(value)
            .unwrap()
            .into_item(7, ItemKind::Issue);
        assert!(item.iteration.contains("2026"), "{}", item.iteration);
        assert!(item.iteration.ends_with(" (Current)"), "{}", item.iteration);
        assert_eq!(item.iteration_id, Some(84001));
    }

    #[test]
    fn milestones_keep_plain_titles_and_empty_iteration_search_lists_open_ones() {
        let mock = Mock::new(|request| {
            let params: HashMap<_, _> = request.url().query_pairs().into_owned().collect();
            if request.url().path().ends_with("/iterations") {
                assert_eq!(params["state"], "opened");
            }
            Reply::json(json!([
                {"id":5,"title":"v1","state":"active","web_url":"https://gl/acme/app/-/milestones/2"}
            ]))
        });
        let milestones = mock.client().lookup(7, LookupKind::Milestones, "").unwrap();
        assert_eq!(milestones[0].value, "v1");
        assert_eq!(milestones[0].description, "acme/app");
        mock.client()
            .lookup(7, LookupKind::Iterations, " ")
            .unwrap();
    }

    #[test]
    fn identical_iteration_labels_are_made_unique() {
        let mock = Mock::new(|_| {
            Reply::json(json!([
                {"id":1,"iid":1,"start_date":"2026-10-01","due_date":"2026-10-14","web_url":"https://gl/groups/a/-/iterations/1"},
                {"id":2,"iid":1,"start_date":"2026-10-01","due_date":"2026-10-14","web_url":"https://gl/groups/b/-/iterations/1"}
            ]))
        });
        let options = mock.client().lookup(7, LookupKind::Iterations, "").unwrap();
        assert_ne!(options[0].value, options[1].value);
    }

    #[test]
    fn issue_iteration_is_assigned_with_graphql_not_the_rest_update() {
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7" => Reply::json(json!({"path_with_namespace":"acme/platform"})),
            "/api/graphql" => Reply::json(json!({"data":{"issueSetIteration":{"errors":[]}}})),
            other => panic!("unexpected {other}"),
        });
        let client = mock.client();
        for (value, expected) in [
            ("84001", json!("gid://gitlab/Iteration/84001")),
            ("", Value::Null),
        ] {
            client
                .mutate(Mutation::Edit {
                    key: key(ItemKind::Issue),
                    field: "iteration_id".into(),
                    value: value.into(),
                })
                .unwrap();
            let requests = mock.requests.lock().unwrap();
            let post = requests.iter().rev().find(|r| r.method == "POST").unwrap();
            assert_eq!(post.url().path(), "/api/graphql");
            let input = &post.json()["variables"]["input"];
            assert_eq!(input["projectPath"], "acme/platform");
            assert_eq!(input["iid"], "1");
            assert_eq!(input["iterationId"], expected);
            assert!(requests.iter().all(|r| r.method != "PUT"));
        }
    }

    #[test]
    fn graphql_reply_without_a_result_is_not_a_success() {
        let mock = Mock::new(|request| {
            if request.url().path() == "/api/graphql" {
                Reply::json(json!({"data":null}))
            } else {
                Reply::json(json!({"path_with_namespace":"acme/platform"}))
            }
        });
        let error = mock
            .client()
            .mutate(Mutation::Edit {
                key: key(ItemKind::Issue),
                field: "iteration_id".into(),
                value: "5".into(),
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("no issueSetIteration result"), "{error}");
        assert!(error.contains("Request: POST /api/graphql"), "{error}");
        assert!(error.contains("gid://gitlab/Iteration/5"), "{error}");
    }

    #[test]
    fn graphql_iteration_errors_are_reported() {
        let mock = Mock::new(|request| {
            if request.url().path() == "/api/graphql" {
                Reply::json(
                    json!({"data":{"issueSetIteration":{"errors":["Iteration not found"]}}}),
                )
            } else {
                Reply::json(json!({"path_with_namespace":"acme/platform"}))
            }
        });
        let error = mock
            .client()
            .mutate(Mutation::Edit {
                key: key(ItemKind::Issue),
                field: "iteration_id".into(),
                value: "5".into(),
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("Iteration not found"), "{error}");
    }

    #[test]
    fn failed_writes_report_request_and_response_without_secrets() {
        let mock = Mock::new(|_| {
            Reply::bytes(
                400,
                br#"{"message":"400 Bad request - at least one param test-private-secret"}"#
                    .to_vec(),
            )
        });
        let error = mock
            .client()
            .mutate(Mutation::Edit {
                key: key(ItemKind::Issue),
                field: "confidential".into(),
                value: "true".into(),
            })
            .unwrap_err()
            .to_string();
        let first = error.lines().next().unwrap();
        assert!(first.contains("HTTP 400"), "{error}");
        assert!(
            error.contains("Request: PUT /api/v4/projects/7/issues/1"),
            "{error}"
        );
        assert!(error.contains(r#"{"confidential":true}"#), "{error}");
        assert!(error.contains("at least one param"), "{error}");
        assert!(!error.contains("test-private-secret"), "{error}");
    }

    #[test]
    fn autocomplete_ui_debounces_search_and_submits_selected_user_ids() {
        use crate::{
            config::Config,
            ui::{Cronk, Msg, Scope},
        };
        use tui_lipan::{TestBackend, prelude::*};
        let mock = Mock::new(|request| {
            if request.method == "PUT" {
                assert_eq!(request.url().path(), "/api/v4/projects/7/issues/1");
                return Reply::json(json!({}));
            }
            match request.url().path() {
                "/api/v4/user" => {
                    Reply::json(json!({"id":1,"name":"Example","username":"example"}))
                }
                "/api/v4/projects/7/users" => {
                    assert!(
                        request
                            .url()
                            .query_pairs()
                            .any(|(k, v)| k == "search" && v == "Alex")
                    );
                    Reply::json(json!([
                        {"id":41,"name":"Alex","username":"alex-one"},
                        {"id":42,"name":"Alex","username":"alex-two"}
                    ]))
                }
                "/api/v4/projects/7/issues/1" => Reply::json(raw_item(1, "opened")),
                _ => Reply::json(json!([])),
            }
        });
        let mut ui = TestBackend::new_with_app(
            App::new().focus_policy(FocusPolicy::Manual),
            Cronk {
                config: Config {
                    onboarding: false,
                    ..Config::default()
                },
                path: None,
                api: Some(mock.client()),
                demo: false,
            },
            (),
        );
        ui.pump().unwrap();
        let mut item = serde_json::from_value::<ApiItem>(raw_item(1, "opened"))
            .unwrap()
            .into_item(7, ItemKind::Issue);
        item.assignees.push(User {
            id: 1,
            username: "example".into(),
            name: "Example".into(),
            ..User::default()
        });
        ui.state_mut().config.projects = vec![project()];
        ui.state_mut().config.route = Some(item.key.clone());
        ui.state_mut().scope = Scope::Section;
        ui.state_mut().details = Some(Details {
            item,
            ..Details::default()
        });
        ui.dispatch(Msg::Field(3)).unwrap();
        ui.send_key(KeyEvent {
            code: KeyCode::Home,
            mods: KeyMods::NONE,
        })
        .unwrap();
        ui.send_key(KeyEvent {
            code: KeyCode::End,
            mods: KeyMods::SHIFT,
        })
        .unwrap();
        ui.send_paste("@example,   Al").unwrap();
        ui.advance(Duration::from_millis(100));
        ui.send_paste("ex ").unwrap();
        ui.advance(Duration::from_millis(250));
        assert!(
            !mock
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.target.contains("/users?"))
        );
        ui.advance(Duration::from_millis(100));
        for _ in 0..40 {
            ui.settle(Duration::from_millis(25)).unwrap();
            if !ui.state().dialog.as_ref().unwrap().fields[0]
                .completion
                .as_ref()
                .unwrap()
                .pending
            {
                break;
            }
        }
        assert_eq!(
            ui.state().dialog.as_ref().unwrap().fields[0]
                .completion
                .as_ref()
                .unwrap()
                .options
                .len(),
            2
        );
        for code in [KeyCode::Down, KeyCode::Enter] {
            ui.send_key(KeyEvent {
                code,
                mods: KeyMods::NONE,
            })
            .unwrap();
        }
        assert_eq!(
            ui.state().dialog.as_ref().unwrap().fields[0].value(),
            "@example,   @alex-two "
        );
        assert!(
            !mock
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.method == "PUT")
        );
        ui.send_key(KeyEvent {
            code: KeyCode::Enter,
            mods: KeyMods::NONE,
        })
        .unwrap();
        for _ in 0..40 {
            ui.settle(Duration::from_millis(25)).unwrap();
            if !ui.state().mutation_pending {
                break;
            }
        }
        let requests = mock.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.target.contains("/users?"))
                .count(),
            1
        );
        let writes: Vec<_> = requests.iter().filter(|r| r.method == "PUT").collect();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].json(), json!({"assignee_ids":[1,42]}));
    }

    #[test]
    fn item_parsing_retains_assignment_ids_not_iids() {
        let mut value = raw_item(1, "opened");
        value["milestone"] = json!({"id":123,"iid":3,"title":"Release"});
        value["iteration"] = json!({"id":456,"iid":6,"title":"Sprint"});
        value["epic"] = json!({"id":789,"iid":9,"title":"Roadmap","group_id":60});
        let item = serde_json::from_value::<ApiItem>(value)
            .unwrap()
            .into_item(7, ItemKind::Issue);
        assert_eq!(item.milestone_id, Some(123));
        assert_eq!(item.iteration_id, Some(456));
        assert_eq!(item.milestone, "Release");
        assert_eq!(item.iteration, "Sprint");
        assert_eq!(item.epic_id, Some(789));
        assert_eq!(item.epic, "Roadmap");
        // Cache round trips retain the assignment; old caches and Free responses remain valid.
        let cached: WorkItem =
            serde_json::from_value(serde_json::to_value(&item).unwrap()).unwrap();
        assert_eq!(cached, item);
        for epic in [
            None,
            Some(Value::Null),
            Some(json!({"id":789,"iid":9,"title":null})),
        ] {
            let mut value = raw_item(1, "opened");
            if let Some(epic) = epic {
                value["epic"] = epic;
            }
            let item = serde_json::from_value::<ApiItem>(value)
                .unwrap()
                .into_item(7, ItemKind::Issue);
            if item.epic_id.is_some() {
                assert_eq!(item.epic, "Epic &9");
            } else {
                assert!(item.epic.is_empty());
            }
        }
        let old: WorkItem = serde_json::from_value(json!({"title":"cached before epics"})).unwrap();
        assert_eq!(old.epic_id, None);
        assert!(old.epic.is_empty());
    }

    #[derive(Debug)]
    struct Request {
        method: String,
        target: String,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    impl Request {
        fn url(&self) -> Url {
            Url::parse(&format!("http://localhost{}", self.target)).unwrap()
        }
        fn page(&self) -> u64 {
            self.url()
                .query_pairs()
                .find(|(k, _)| k == "page")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(1)
        }
        fn json(&self) -> Value {
            serde_json::from_slice(&self.body).unwrap()
        }
    }

    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        chunked: bool,
    }

    impl Reply {
        fn bytes(status: u16, body: impl Into<Vec<u8>>) -> Self {
            Self {
                status,
                headers: Vec::new(),
                body: body.into(),
                chunked: false,
            }
        }
        fn json(body: Value) -> Self {
            Self::bytes(200, serde_json::to_vec(&body).unwrap())
        }
        fn header(mut self, key: &str, value: &str) -> Self {
            self.headers.push((key.into(), value.into()));
            self
        }
        fn chunked(mut self) -> Self {
            self.chunked = true;
            self
        }
    }

    struct Mock {
        base: String,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
        requests: Arc<Mutex<Vec<Request>>>,
    }

    impl Mock {
        fn new(mut route: impl FnMut(&Request) -> Reply + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let requests = Arc::new(Mutex::new(Vec::new()));
            let flag = stop.clone();
            let log = requests.clone();
            let worker = thread::spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("mock accept: {error}"),
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let request = read_request(&mut stream);
                    let reply = route(&request);
                    log.lock().unwrap().push(request);
                    let mut headers =
                        format!("HTTP/1.1 {} Test\r\nConnection: close\r\n", reply.status);
                    for (key, value) in reply.headers {
                        headers.push_str(&format!("{key}: {value}\r\n"));
                    }
                    if reply.chunked {
                        headers.push_str("Transfer-Encoding: chunked\r\n\r\n");
                    } else {
                        headers.push_str(&format!("Content-Length: {}\r\n\r\n", reply.body.len()));
                    }
                    // Clients deliberately abandon oversized trace/JSON responses.
                    if stream.write_all(headers.as_bytes()).is_ok() {
                        if reply.chunked {
                            let _ = write!(stream, "{:x}\r\n", reply.body.len());
                            let _ = stream.write_all(&reply.body);
                            let _ = stream.write_all(b"\r\n0\r\n\r\n");
                        } else {
                            let _ = stream.write_all(&reply.body);
                        }
                    }
                }
            });
            Self {
                base,
                stop,
                worker: Some(worker),
                requests,
            }
        }
        fn concurrent(route: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let stop = Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let log = requests.clone();
            let route = Arc::new(route);
            let worker = thread::spawn(move || {
                let mut workers = Vec::new();
                while !flag.load(Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(e) => panic!("mock accept: {e}"),
                    };
                    let route = route.clone();
                    let log = log.clone();
                    workers.push(thread::spawn(move || {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let request = read_request(&mut stream);
                        let reply = route(&request);
                        log.lock().unwrap().push(request);
                        let mut headers = format!(
                            "HTTP/1.1 {} Test\r\nConnection: close\r\nContent-Length: {}\r\n",
                            reply.status,
                            reply.body.len()
                        );
                        for (key, value) in reply.headers {
                            headers.push_str(&format!("{key}: {value}\r\n"));
                        }
                        headers.push_str("\r\n");
                        if stream.write_all(headers.as_bytes()).is_ok() {
                            let _ = stream.write_all(&reply.body);
                        }
                    }));
                }
                for worker in workers {
                    worker.join().unwrap();
                }
            });
            Self {
                base,
                stop,
                requests,
                worker: Some(worker),
            }
        }

        fn client(&self) -> GitLab {
            GitLab::new(&self.base, "test-private-secret").unwrap()
        }
        fn legacy_trace_client(&self) -> GitLab {
            let client = self.client();
            client.trace_byte_params.store(2, Ordering::Relaxed);
            client
        }
    }

    impl Drop for Mock {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let result = worker.join();
                if !thread::panicking() {
                    result.unwrap();
                }
            }
        }
    }

    fn read_request(stream: &mut TcpStream) -> Request {
        let mut buffer = Vec::new();
        let boundary = loop {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).unwrap();
            buffer.push(byte[0]);
            assert!(buffer.len() <= 32 * 1024);
            if buffer.ends_with(b"\r\n\r\n") {
                break buffer.len();
            }
        };
        let header = String::from_utf8(buffer[..boundary].to_vec()).unwrap();
        let mut lines = header.split("\r\n");
        let mut first = lines.next().unwrap().split_ascii_whitespace();
        let method = first.next().unwrap().to_owned();
        let target = first.next().unwrap().to_owned();
        let headers: HashMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let length = headers
            .get("content-length")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        assert!(length < 1024 * 1024);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        Request {
            method,
            target,
            headers,
            body,
        }
    }

    fn raw_item(iid: u64, state: &str) -> Value {
        json!({"iid": iid, "title": format!("Fixture {iid}"), "state": state, "description": null,
            "author": {"id": 1, "username": "example", "name": "Example"},
            "updated_at": format!("2026-09-{:02}T00:00:00Z", iid % 28 + 1)})
    }

    fn project() -> Project {
        Project {
            id: 7,
            path: "demo/deep/project".into(),
            visible: true,
            ..Project::default()
        }
    }
    fn key(kind: ItemKind) -> ItemKey {
        ItemKey {
            project: 7,
            iid: 1,
            kind,
        }
    }

    #[test]
    fn persistent_sync_publishes_pages_and_warm_restart_requests_only_deltas() {
        let mock = Mock::new(|request| {
            let url = request.url();
            if url.path().ends_with("/merge_requests") {
                return Reply::json(json!([]));
            }
            assert!(url.path().ends_with("/issues"));
            let delta = url.query_pairs().any(|(k, _)| k == "updated_after");
            if delta {
                let mut item = raw_item(1, "closed");
                item["title"] = json!("Changed after import");
                return Reply::json(json!([item])).header("x-total", "1");
            }
            let page = request.page();
            let rows: Vec<_> = ((page - 1) * 100 + 1..=page * 100)
                .map(|iid| raw_item(iid, "opened"))
                .collect();
            Reply::json(json!(rows)).header("x-total", "1000").header(
                "x-next-page",
                &if page < 10 {
                    (page + 1).to_string()
                } else {
                    String::new()
                },
            )
        });
        let legacy = mock.client();
        assert_eq!(legacy.list_project(&project()).unwrap().len(), 1000);
        assert_eq!(
            mock.requests.lock().unwrap().len(),
            11,
            "uncached all-history baseline"
        );
        mock.requests.lock().unwrap().clear();
        drop(legacy);
        let directory = tempfile::tempdir().unwrap();
        let workspace = directory.path().join("config.toml");
        let mut api = mock.client();
        api.enable_persistence(&workspace).unwrap();
        let mut delivered = 0;
        let items = api
            .sync_project(&project(), false, |status, page| {
                if !page.is_empty() {
                    delivered += page.len();
                    assert_eq!(
                        api.cached_items(7).unwrap().len(),
                        delivered,
                        "UI pages are already durable"
                    );
                    assert_eq!(status.total, Some(1000));
                    if status.page == 1 {
                        assert_eq!(
                            mock.requests
                                .lock()
                                .unwrap()
                                .iter()
                                .filter(|r| r.url().path().ends_with("/issues"))
                                .count(),
                            1,
                            "first page is delivered before second issue request"
                        );
                    }
                }
            })
            .unwrap();
        assert_eq!(items.len(), 1000);
        assert_eq!(delivered, 1000);
        let cold_requests = mock.requests.lock().unwrap().len();
        assert_eq!(cold_requests, 11);
        drop(api);
        let mut api = mock.client();
        api.enable_persistence(&workspace).unwrap();
        assert_eq!(api.cached_items(7).unwrap().len(), 1000);
        let items = api.sync_project(&project(), false, |_, _| {}).unwrap();
        assert_eq!(items.len(), 1000);
        assert_eq!(
            items.iter().find(|i| i.key.iid == 1).unwrap().title,
            "Changed after import"
        );
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len() - cold_requests, 2);
        assert!(
            requests[cold_requests..]
                .iter()
                .all(|r| r.url().query_pairs().any(|(k, _)| k == "updated_after"))
        );
    }

    #[test]
    fn interrupted_network_import_resumes_its_saved_timestamp_boundary() {
        let failed = Arc::new(AtomicBool::new(false));
        let route_flag = failed.clone();
        let mock = Mock::new(move |request| {
            if request.url().path().ends_with("/merge_requests") {
                return Reply::json(json!([]));
            }
            if request.page() == 2 && !route_flag.swap(true, Ordering::SeqCst) {
                return Reply::bytes(503, "temporarily unavailable");
            }
            if request.page() == 1 {
                return Reply::json(json!([raw_item(1, "opened")])).header("x-next-page", "2");
            }
            Reply::json(json!([raw_item(2, "closed")])).header("x-next-page", "")
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut api = mock.client();
        api.enable_persistence(&path).unwrap();
        assert!(api.sync_project(&project(), false, |_, _| {}).is_err());
        assert_eq!(api.cached_items(7).unwrap().len(), 1);
        let restart_index = mock.requests.lock().unwrap().len();
        drop(api);
        let mut api = mock.client();
        api.enable_persistence(&path).unwrap();
        let items = api.sync_project(&project(), false, |_, _| {}).unwrap();
        assert_eq!(items.len(), 2);
        let requests = mock.requests.lock().unwrap();
        let resume = requests[restart_index..]
            .iter()
            .find(|r| r.url().path().ends_with("/issues"))
            .unwrap()
            .url();
        assert!(
            resume
                .query_pairs()
                .any(|(k, v)| k == "updated_before" && v == "2026-09-02T00:00:00Z")
        );
        assert!(!resume.query_pairs().any(|(k, _)| k == "updated_after"));
    }

    #[test]
    fn reconciliation_verifies_missing_items_before_removing_them() {
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7/issues" | "/api/v4/projects/7/merge_requests" => {
                Reply::json(json!([]))
            }
            "/api/v4/projects/7/issues/1" => Reply::json(raw_item(1, "opened")),
            "/api/v4/projects/7/issues/2" => Reply::bytes(404, "gone"),
            other => panic!("unexpected {other}"),
        });
        let dir = tempfile::tempdir().unwrap();
        let mut api = mock.client();
        api.enable_persistence(&dir.path().join("config.toml"))
            .unwrap();
        let cache = api.persistent.as_ref().unwrap();
        let plan = cache.plan(7, ItemKind::Issue, false).unwrap();
        let items: Vec<_> = [1, 2]
            .into_iter()
            .map(|iid| {
                serde_json::from_value::<ApiItem>(raw_item(iid, "opened"))
                    .unwrap()
                    .into_item(7, ItemKind::Issue)
            })
            .collect();
        cache.page(7, ItemKind::Issue, &plan, &items).unwrap();
        cache.finish(7, ItemKind::Issue, &plan, &[]).unwrap();
        let result = api.sync_project(&project(), true, |_, _| {}).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].key.iid, 1);
    }

    #[test]
    fn progressive_details_publish_core_first_and_persist_completed_sections() {
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7/issues/1" => Reply::json(raw_item(1, "opened")),
            "/api/v4/projects/7/issues/1/notes" | "/api/v4/projects/7/issues/1/discussions" => {
                Reply::json(json!([]))
            }
            other => panic!("unexpected {other}"),
        });
        let dir = tempfile::tempdir().unwrap();
        let mut api = mock.client();
        api.enable_persistence(&dir.path().join("config.toml"))
            .unwrap();
        let delivered = Mutex::new(Vec::new());
        let details = api
            .details_progress(&key(ItemKind::Issue), None, |part, details| {
                if part == DetailPart::Core {
                    assert_eq!(mock.requests.lock().unwrap().len(), 1);
                }
                assert!(details.loaded.contains(&part));
                assert!(
                    api.cached_details(&details.item.key)
                        .unwrap()
                        .unwrap()
                        .loaded
                        .contains(&part)
                );
                delivered.lock().unwrap().push(part);
            })
            .unwrap();
        assert_eq!(details.loaded.len(), 3);
        assert_eq!(delivered.lock().unwrap()[0], DetailPart::Core);
        assert_eq!(
            api.cached_details(&details.item.key)
                .unwrap()
                .unwrap()
                .loaded,
            details.loaded
        );
        api.mutate(Mutation::Edit {
            key: details.item.key.clone(),
            field: "title".into(),
            value: "new".into(),
        })
        .unwrap();
        assert!(api.cached_details(&details.item.key).unwrap().is_none());
    }

    #[test]
    fn a_blocked_activity_section_does_not_block_discussions_jobs_or_changes() {
        let (blocked_tx, blocked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        let mock = Mock::concurrent(move |request| match request.url().path() {
            "/api/v4/projects/7/merge_requests/1" => {
                let mut item = raw_item(1, "opened");
                item["head_pipeline"] = json!({"id":4,"status":"running"});
                Reply::json(item)
            }
            "/api/v4/projects/7/merge_requests/1/notes" => {
                blocked_tx.send(()).unwrap();
                release_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap();
                Reply::json(json!([]))
            }
            "/api/v4/projects/7/merge_requests/1/discussions"
            | "/api/v4/projects/7/merge_requests/1/diffs"
            | "/api/v4/projects/7/pipelines/4/jobs" => Reply::json(json!([])),
            other => panic!("unexpected {other}"),
        });
        let api = mock.client();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            api.details_progress(&key(ItemKind::MergeRequest), None, |part, _| {
                tx.send(part).unwrap();
            })
        });
        blocked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut parts = HashSet::new();
        while let Ok(part) = rx.recv_timeout(Duration::from_secs(2)) {
            parts.insert(part);
            if [
                DetailPart::Discussions,
                DetailPart::Pipeline,
                DetailPart::Changes,
            ]
            .iter()
            .all(|p| parts.contains(p))
            {
                break;
            }
        }
        // Always release before assertions, so a sequential-loading regression can't
        // leave blocked workers behind when this test fails.
        release_tx.send(()).unwrap();
        let details = worker.join().unwrap().unwrap();
        assert!(parts.contains(&DetailPart::Discussions));
        assert!(parts.contains(&DetailPart::Pipeline));
        assert!(parts.contains(&DetailPart::Changes));
        assert!(!parts.contains(&DetailPart::Activity));
        assert_eq!(details.loaded.len(), 5);
    }

    #[test]
    fn a_successful_write_cannot_be_followed_by_stale_detail_cache_repopulation() {
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7/issues/1" => Reply::json(raw_item(1, "opened")),
            "/api/v4/projects/7/issues/1/notes" | "/api/v4/projects/7/issues/1/discussions" => {
                Reply::json(json!([]))
            }
            other => panic!("unexpected {other}"),
        });
        let dir = tempfile::tempdir().unwrap();
        let mut api = mock.client();
        api.enable_persistence(&dir.path().join("config.toml"))
            .unwrap();
        let result = api.details_progress(&key(ItemKind::Issue), None, |part, _| {
            if part == DetailPart::Core {
                api.mutate(Mutation::Comment {
                    key: key(ItemKind::Issue),
                    body: "A write during detail loading".into(),
                })
                .unwrap();
            }
        });
        assert!(result.unwrap_err().to_string().contains("superseded"));
        assert!(api.cached_details(&key(ItemKind::Issue)).unwrap().is_none());
    }

    #[test]
    fn failed_optional_refresh_retains_cached_content_with_a_warning() {
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7/issues/1" => Reply::json(raw_item(1, "opened")),
            "/api/v4/projects/7/issues/1/notes" => Reply::bytes(503, "retry later"),
            "/api/v4/projects/7/issues/1/discussions" => Reply::json(json!([])),
            other => panic!("unexpected {other}"),
        });
        let previous = Details {
            notes: vec![crate::model::Note {
                body: "Already cached".into(),
                ..Default::default()
            }],
            loaded: HashSet::from([DetailPart::Activity]),
            ..Default::default()
        };
        let details = mock
            .client()
            .details_progress(&key(ItemKind::Issue), Some(previous), |_, _| {})
            .unwrap();
        assert_eq!(details.notes[0].body, "Already cached");
        assert!(
            details
                .warnings
                .iter()
                .any(|w| w.contains("Notes incomplete"))
        );
    }

    #[test]
    fn url_security_subpaths_and_percent_encoded_project_ids() {
        for url in [
            "http://gitlab.example.com",
            "ftp://localhost",
            "https://user:secret@example.com",
            "https://example.com?token=secret",
            "https://example.com/#secret",
            "http://127.0.0.1.evil.example",
        ] {
            assert!(GitLab::new(url, "token").is_err(), "{url}");
        }
        assert!(GitLab::new("https://example.com", "bad\ntoken").is_err());
        assert!(GitLab::new("https://example.com", "").is_err());
        for url in [
            "http://localhost",
            "http://127.1.2.3",
            "http://[::1]",
            "https://gitlab.example.com/root",
        ] {
            assert!(GitLab::new(url, "token").is_ok(), "{url}");
        }
        let mock = Mock::new(|request| {
            assert_eq!(
                request.target,
                "/enterprise%20root/gitlab/api/v4/projects/demo%2Fnested%2Fproject"
            );
            assert_eq!(request.headers["private-token"], "test-private-secret");
            Reply::json(json!({
                "id": 7,
                "name": "Nested project",
                "path_with_namespace": "demo/nested/project"
            }))
        });
        for suffix in [
            "/enterprise%20root/gitlab",
            "/enterprise%20root/gitlab/api/v4/",
        ] {
            let client =
                GitLab::new(&format!("{}{suffix}", mock.base), "test-private-secret").unwrap();
            let project = client.resolve_project("demo/nested/project").unwrap();
            assert_eq!(project.path, "demo/nested/project");
            assert_eq!(project.alias, "Nested project");
        }
    }

    #[test]
    fn redirects_retry_after_and_json_errors_do_not_leak_secrets() {
        let sink = Mock::new(|_| panic!("redirect must never receive a token"));
        let location = format!("{}/stolen", sink.base);
        let redirect = Mock::new(move |_| {
            Reply::bytes(302, b"test-private-secret".to_vec()).header("Location", &location)
        });
        let error = redirect.client().current_user().unwrap_err().to_string();
        assert!(error.contains("redirects are disabled"));
        assert!(!error.contains("test-private-secret"));
        assert!(sink.requests.lock().unwrap().is_empty());
        let limited = Mock::new(|_| {
            Reply::bytes(429, b"test-private-secret".to_vec()).header("Retry-After", "120")
        });
        let error = limited.client().current_user().unwrap_err().to_string();
        assert!(error.contains("Retry-After: 120") && error.contains("no automatic retry"));
        assert!(!error.contains("test-private-secret"));
        let date = Mock::new(|_| {
            Reply::bytes(503, Vec::new()).header("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT")
        });
        assert!(
            date.client()
                .current_user()
                .unwrap_err()
                .to_string()
                .contains("Wed, 21 Oct 2026")
        );
        let invalid = Mock::new(|_| Reply::json(json!({"id": "test-private-secret"})));
        let error = format!("{:#}", invalid.client().current_user().unwrap_err());
        assert!(error.contains("invalid JSON/schema") && !error.contains("test-private-secret"));
    }

    #[test]
    fn validators_are_shared_and_successful_writes_invalidate_cache() {
        let mut reads = 0;
        let mock = Mock::new(move |request| {
            if request.method == "POST" {
                assert!(request.target.ends_with("/issues/1/notes"));
                assert_eq!(request.json(), json!({"body": "new comment"}));
                return Reply::bytes(201, b"{}".to_vec());
            }
            reads += 1;
            match reads {
                1 | 3 => {
                    assert!(!request.headers.contains_key("if-none-match"));
                    assert!(!request.headers.contains_key("if-modified-since"));
                    Reply::json(json!({"id": 1, "username": "example", "name": "Example"}))
                        .header("ETag", "\"v1\"")
                        .header("Last-Modified", "Wed, 21 Oct 2026 07:28:00 GMT")
                }
                2 => {
                    assert_eq!(request.headers["if-none-match"], "\"v1\"");
                    assert_eq!(
                        request.headers["if-modified-since"],
                        "Wed, 21 Oct 2026 07:28:00 GMT"
                    );
                    Reply::bytes(304, Vec::new())
                }
                _ => panic!("unexpected GET"),
            }
        });
        let client = mock.client();
        assert_eq!(
            client.current_user().unwrap(),
            client.clone().current_user().unwrap()
        );
        client
            .mutate(Mutation::Comment {
                key: key(ItemKind::Issue),
                body: "new comment".into(),
            })
            .unwrap();
        assert_eq!(client.current_user().unwrap().id, 1);
    }

    #[test]
    fn last_modified_without_etag_and_failed_write_retains_cache() {
        let mut read = false;
        let mock = Mock::new(move |request| {
            if request.method == "POST" {
                return Reply::bytes(403, Vec::new());
            }
            if read {
                assert_eq!(
                    request.headers["if-modified-since"],
                    "Wed, 21 Oct 2026 07:28:00 GMT"
                );
                Reply::bytes(304, Vec::new())
            } else {
                read = true;
                Reply::json(json!({"id": 1}))
                    .header("Last-Modified", "Wed, 21 Oct 2026 07:28:00 GMT")
            }
        });
        let client = mock.client();
        client.current_user().unwrap();
        assert!(
            client
                .mutate(Mutation::RetryJob { project: 7, job: 1 })
                .is_err()
        );
        assert_eq!(client.current_user().unwrap().id, 1);
    }

    #[test]
    fn cache_is_bounded_by_bytes_and_entries() {
        let mut cache = Cache::default();
        for i in 0..CACHE_ENTRIES + 10 {
            cache.insert(
                i.to_string(),
                Document {
                    body: Arc::from(&b"{}"[..]),
                    headers: HeaderMap::new(),
                },
            );
        }
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
        assert!(!cache.entries.contains_key("0"));
        for i in 0..4 {
            cache.insert(
                format!("large-{i}"),
                Document {
                    body: Arc::from(vec![0; JSON_LIMIT]),
                    headers: HeaderMap::new(),
                },
            );
        }
        assert!(cache.bytes <= CACHE_BYTES);
        assert!(cache.entries.len() <= CACHE_ENTRIES);
        cache.invalidate();
        assert_eq!(cache.bytes, 0);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn pagination_includes_all_states_and_label_backgrounds() {
        let mock = Mock::new(|request| {
            let url = request.url();
            if url.path().ends_with("/labels") {
                assert!(request.target.contains("include_ancestor_groups=true"));
                return Reply::json(
                    json!([{"name": "bug", "color": "#cc0000", "text_color": "#ffffff"}]),
                )
                .header("X-Next-Page", "");
            }
            assert!(request.target.contains("state=all") && request.target.contains("scope=all"));
            let issue = url.path().ends_with("/issues");
            match (issue, request.page()) {
                (true, 1) => {
                    let mut item = raw_item(1, "opened"); item["labels"] = json!(["bug"]);
                    Reply::json(json!([item])).header("X-Next-Page", "2")
                }
                (true, 2) => Reply::json(json!([raw_item(2, "closed")])).header("X-Next-Page", ""),
                (false, 1) => Reply::json(json!([raw_item(3, "merged")])).header("Link", "</api/v4/projects/7/merge_requests?state=all&scope=all&per_page=100&page=2>; rel=\"next\""),
                (false, 2) => Reply::json(json!([raw_item(4, "closed")])).header("X-Next-Page", ""),
                _ => panic!("unexpected request {request:?}"),
            }
        });
        let items = mock.client().list_project(&project()).unwrap();
        assert_eq!(items.len(), 4);
        assert!(items.iter().any(|i| i.state == "merged"));
        assert_eq!(
            items.iter().find(|i| i.key.iid == 1).unwrap().labels[0].color,
            "#cc0000"
        );
    }

    #[test]
    fn headerless_full_pages_are_probed_and_repeated_pages_fail() {
        let mock = Mock::new(|request| {
            let first = request.page() == 1;
            Reply::json(if first {
                json!((0..100).collect::<Vec<_>>())
            } else {
                json!([100, 101])
            })
        });
        assert_eq!(
            mock.client()
                .all::<u64>(mock.client().url(&["collection"]))
                .unwrap()
                .len(),
            102
        );
        let repeated = Mock::new(|_| Reply::json(json!((0..100).collect::<Vec<_>>())));
        let client = repeated.client();
        assert!(
            client
                .all::<u64>(client.url(&["collection"]))
                .unwrap_err()
                .to_string()
                .contains("repeated page")
        );
    }

    #[test]
    fn pagination_headers_survive_304_and_unsafe_links_are_rejected() {
        let mock = Mock::new(|request| {
            if request.headers.contains_key("if-none-match") {
                return Reply::bytes(304, Vec::new());
            }
            Reply::json(json!([request.page()]))
                .header("ETag", &format!("\"page-{}\"", request.page()))
                .header("X-Next-Page", if request.page() == 1 { "2" } else { "" })
        });
        let client = mock.client();
        for _ in 0..2 {
            assert_eq!(
                client.all::<u64>(client.url(&["collection"])).unwrap(),
                vec![1, 2]
            );
        }
        for link in [
            "<https://evil.invalid/steal>; rel=\"next\"",
            "</api/v4/wrong>; rel=\"next\"",
        ] {
            let link = link.to_owned();
            let mock = Mock::new(move |_| Reply::json(json!([1])).header("Link", &link));
            let client = mock.client();
            assert!(
                client
                    .all::<u64>(client.url(&["collection"]))
                    .unwrap_err()
                    .to_string()
                    .contains("unsafe pagination")
            );
        }
    }

    #[test]
    fn dashboard_enrichment_is_bounded_and_unknown_does_not_mean_zero() {
        let mock = Mock::new(|request| {
            let url = request.url();
            if url.path().ends_with("/issues") {
                return Reply::json(json!([]));
            }
            if url.path().ends_with("/merge_requests") {
                return Reply::json(Value::Array(
                    (1..=30).map(|iid| raw_item(iid, "opened")).collect(),
                ));
            }
            if url.path().ends_with("/discussions") {
                return Reply::bytes(404, Vec::new());
            }
            let iid = url.path().rsplit('/').next().unwrap().parse().unwrap();
            let mut item = raw_item(iid, "opened");
            item["head_pipeline"] = json!({"id": iid + 500, "status": "failed"});
            item["blocking_discussions_resolved"] = json!(false);
            Reply::json(item)
        });
        let items = mock.client().list_project(&project()).unwrap();
        assert_eq!(items.len(), 30);
        assert_eq!(
            items.iter().filter(|item| item.pipeline.is_some()).count(),
            DASHBOARD_ENRICHMENT
        );
        assert!(items.iter().all(|item| item.unresolved.is_none()));
        assert!(
            items
                .iter()
                .take(DASHBOARD_ENRICHMENT)
                .all(|item| item.pipeline.is_some())
        );
        assert_eq!(
            mock.requests.lock().unwrap().len(),
            2 + DASHBOARD_ENRICHMENT + DASHBOARD_DISCUSSION_PAGES
        );
    }

    #[test]
    fn optional_detail_pages_keep_partial_data_and_warn() {
        let mock = Mock::new(|request| {
            let url = request.url();
            match url.path() {
                "/api/v4/projects/7/merge_requests/1" => {
                    let mut item = raw_item(1, "opened");
                    item["head_pipeline"] = json!({"id": 77, "project_id": 8, "status": "running"});
                    item["changes_count"] = json!("3+");
                    Reply::json(item)
                }
                "/api/v4/projects/7/merge_requests/1/notes" => {
                    assert!(request.target.contains("sort=desc"));
                    Reply::json(json!([
                        {"id": 1, "body": "older", "created_at": "2026-01-01T00:00:00Z"},
                        {"id": 2, "body": "newer", "created_at": "2026-01-02T00:00:00Z"}
                    ]))
                }
                "/api/v4/projects/7/merge_requests/1/discussions" => match request.page() {
                    1 => Reply::json(json!([{"id": "thread", "notes": [{"id": 1, "resolvable": true, "resolved": false}]}])).header("X-Next-Page", "2"),
                    _ => Reply::bytes(403, b"private response body".to_vec()),
                },
                "/api/v4/projects/8/pipelines/77/jobs" => match request.page() {
                    1 => Reply::json(json!([{"id": 1, "name": "linux", "status": "running"}])).header("X-Next-Page", "2"),
                    _ => Reply::json(json!([{"id": 2, "name": "windows", "status": "running"}])).header("X-Next-Page", ""),
                },
                "/api/v4/projects/7/merge_requests/1/diffs" => match request.page() {
                    1 => Reply::json(json!([{"new_path": "src/main.rs", "collapsed": true}])).header("X-Next-Page", "2"),
                    _ => Reply::bytes(429, Vec::new()).header("Retry-After", "15"),
                },
                _ => panic!("unexpected request {request:?}"),
            }
        });
        let details = mock.client().details(&key(ItemKind::MergeRequest)).unwrap();
        assert_eq!(details.notes[0].id, 2);
        assert_eq!(details.discussions.len(), 1);
        assert_eq!(details.item.unresolved, None);
        assert_eq!(details.jobs.iter().filter(|j| j.running()).count(), 2);
        assert_eq!(details.diffs.len(), 1);
        assert!(
            details
                .warnings
                .iter()
                .any(|w| w.contains("Discussions incomplete"))
        );
        assert!(
            details
                .warnings
                .iter()
                .any(|w| w.contains("Retry-After: 15"))
        );
        assert!(details.warnings.iter().any(|w| w.contains("collapsed")));
        assert!(
            details
                .warnings
                .iter()
                .all(|w| !w.contains("private response body"))
        );
    }

    #[test]
    fn latest_head_pipeline_is_selected_by_sha_not_list_order() {
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7/merge_requests/1" => {
                let mut item = raw_item(1, "opened");
                item["sha"] = json!("head-sha");
                Reply::json(item)
            }
            "/api/v4/projects/7/merge_requests/1/pipelines" => {
                if request.page() == 1 {
                    Reply::json(json!([{"id": 999, "sha": "old-sha", "status": "success"}, {"id": 1, "sha": "head-sha", "status": "failed"}])).header("X-Next-Page", "2")
                } else {
                    Reply::json(json!([{"id": 2, "sha": "head-sha", "status": "running"}]))
                        .header("X-Next-Page", "")
                }
            }
            "/api/v4/projects/7/pipelines/2" => {
                Reply::json(json!({"id": 2, "sha": "head-sha", "status": "running"}))
            }
            "/api/v4/projects/7/pipelines/2/jobs" => {
                Reply::json(json!([{"id": 1, "status": "running"}]))
            }
            _ => Reply::json(json!([])),
        });
        let details = mock.client().details(&key(ItemKind::MergeRequest)).unwrap();
        assert_eq!(details.item.pipeline.unwrap().id, 2);
        assert_eq!(details.item.unresolved, Some(0));
        assert_eq!(details.jobs.len(), 1);
        assert!(details.warnings.is_empty());
    }

    #[test]
    fn all_mutation_routes_and_typed_edit_bodies() {
        let mock = Mock::new(|_| Reply::bytes(204, Vec::new()));
        let client = mock.client();
        let issue = key(ItemKind::Issue);
        let mr = key(ItemKind::MergeRequest);
        let mutations = vec![
            Mutation::Edit {
                key: issue.clone(),
                field: "assignee_ids".into(),
                value: "1, 2".into(),
            },
            Mutation::Edit {
                key: mr.clone(),
                field: "reviewer_ids".into(),
                value: "".into(),
            },
            Mutation::Edit {
                key: issue.clone(),
                field: "milestone_id".into(),
                value: "4".into(),
            },
            Mutation::Edit {
                key: issue.clone(),
                field: "confidential".into(),
                value: "true".into(),
            },
            Mutation::Comment {
                key: issue.clone(),
                body: "hello".into(),
            },
            Mutation::Reply {
                key: mr.clone(),
                discussion: "thread/id".into(),
                body: "reply".into(),
            },
            Mutation::Resolve {
                key: mr.clone(),
                discussion: "thread".into(),
                resolved: true,
            },
            Mutation::RetryJob {
                project: 7,
                job: 42,
            },
            Mutation::CreateIssue {
                project: 7,
                title: "new".into(),
                description: "body".into(),
            },
            Mutation::CreateMergeRequest {
                project: 7,
                title: "new MR".into(),
                source: "feature".into(),
                target: "main".into(),
                description: "body".into(),
            },
        ];
        for mutation in mutations {
            client.mutate(mutation).unwrap();
        }
        let requests = mock.requests.lock().unwrap();
        assert_eq!(requests.len(), 10);
        assert_eq!(requests[0].method, "PUT");
        assert_eq!(requests[0].json(), json!({"assignee_ids": [1, 2]}));
        assert_eq!(requests[1].json(), json!({"reviewer_ids": []}));
        assert_eq!(requests[2].json(), json!({"milestone_id": 4}));
        assert_eq!(requests[3].json(), json!({"confidential": true}));
        assert!(
            requests[5]
                .target
                .ends_with("/discussions/thread%2Fid/notes")
        );
        assert_eq!(requests[6].json(), json!({"resolved": true}));
        assert!(requests[7].target.ends_with("/jobs/42/retry"));
        assert_eq!(
            requests[8].json(),
            json!({"title": "new", "description": "body"})
        );
        assert_eq!(
            requests[9].json(),
            json!({"title": "new MR", "source_branch": "feature", "target_branch": "main", "description": "body"})
        );
        drop(requests);
        assert!(
            client
                .mutate(Mutation::Edit {
                    key: issue,
                    field: "assignee_ids".into(),
                    value: "secret-not-a-number".into()
                })
                .is_err()
        );
        assert!(
            client
                .mutate(Mutation::Resolve {
                    key: mr,
                    discussion: "..".into(),
                    resolved: true
                })
                .is_err()
        );
        assert_eq!(mock.requests.lock().unwrap().len(), 10);
    }

    #[test]
    fn trace_query_parameters_preserve_byte_offsets_and_parser_state() {
        for chunked in [false, true] {
            let data = Arc::new(Mutex::new(b"a\xc3".to_vec()));
            let server_data = data.clone();
            let mock = Mock::new(move |request| {
                assert_eq!(request.url().path(), "/api/v4/projects/7/jobs/42/trace");
                assert!(!request.headers.contains_key("range"));
                assert_eq!(request.headers["accept-encoding"], "identity");
                let params: HashMap<_, _> = request.url().query_pairs().into_owned().collect();
                let offset: u64 = params["byte_offset"].parse().unwrap();
                let limit: usize = params["byte_limit"].parse().unwrap();
                assert!(limit == 1 || limit == TRACE_BYTES);
                let data = server_data.lock().unwrap();
                let start = offset.min(data.len() as u64) as usize;
                let end = start.saturating_add(limit).min(data.len());
                let reply = Reply::bytes(200, data[start..end].to_vec());
                if chunked { reply.chunked() } else { reply }
            });
            let client = mock.client();
            let first = client.trace(7, 42, 0).unwrap();
            assert_eq!(first.text, "a");
            assert_eq!(first.next_offset, 2);
            data.lock()
                .unwrap()
                .extend_from_slice(b"\xa9\x1b]52;c;secret");
            let second = client.clone().trace(7, 42, first.next_offset).unwrap();
            assert_eq!(second.text, "é");
            data.lock()
                .unwrap()
                .extend_from_slice(b"\x07\x1b[31mRED\x1b[0m\r\n");
            let third = client.trace(7, 42, second.next_offset).unwrap();
            assert_eq!(third.text, "\x1b[31mRED\x1b[0m\n");
            assert_eq!(third.next_offset, data.lock().unwrap().len() as u64);
            assert!(!third.reset);
            let eof = client.trace(7, 42, third.next_offset).unwrap();
            assert!(eof.text.is_empty());
            assert_eq!(eof.next_offset, third.next_offset);
            assert!(!eof.reset);
            // Two capability probes, three reads, then EOF and its preceding-byte check.
            assert_eq!(mock.requests.lock().unwrap().len(), 7);
            data.lock().unwrap().clear();
            data.lock().unwrap().extend_from_slice(b"new\n");
            let reset = client.trace(7, 42, eof.next_offset).unwrap();
            assert!(reset.reset);
            assert_eq!(reset.text, "new\n");
            assert_eq!(reset.next_offset, 4);
        }
    }

    #[test]
    fn trace_ignored_query_parameters_fall_back_without_duplicating_output() {
        // Cover ignoring both parameters, only offset, and only limit, as well
        // as a one-byte full trace which alone looks like a valid limit=1 slice.
        for mode in 0..4 {
            let data = Arc::new(Mutex::new(if mode == 3 {
                b"a".to_vec()
            } else {
                b"old".to_vec()
            }));
            let server_data = data.clone();
            let mock = Mock::new(move |request| {
                let params: HashMap<_, _> = request.url().query_pairs().into_owned().collect();
                let data = server_data.lock().unwrap();
                if params.contains_key("byte_limit") {
                    assert!(!request.headers.contains_key("range"));
                    let offset: u64 = params["byte_offset"].parse().unwrap();
                    let limit: usize = params["byte_limit"].parse().unwrap();
                    let start = if mode == 2 {
                        offset.min(data.len() as u64) as usize
                    } else {
                        0
                    };
                    let end = if mode == 1 {
                        start.saturating_add(limit).min(data.len())
                    } else {
                        data.len()
                    };
                    Reply::bytes(200, data[start..end].to_vec())
                } else {
                    assert!(request.headers.contains_key("range"));
                    Reply::bytes(200, data.clone()).chunked()
                }
            });
            let client = mock.client();
            let first = client.trace(7, 42, 0).unwrap();
            assert_eq!(first.text.as_bytes(), data.lock().unwrap().as_slice());
            data.lock().unwrap().extend_from_slice(b"new\n");
            let second = client.clone().trace(7, 42, first.next_offset).unwrap();
            assert_eq!(second.text, "new\n");
            assert!(!second.reset);
            assert_eq!(second.next_offset, data.lock().unwrap().len() as u64);
            let requests = mock.requests.lock().unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.target.contains("byte_limit"))
                    .count(),
                if mode == 1 || mode == 3 { 2 } else { 1 }
            );
        }
    }

    #[test]
    fn trace_ignored_parameters_can_fall_back_to_partial_content() {
        let mock = Mock::new(|request| {
            if request.target.contains("byte_limit") {
                return Reply::bytes(200, b"oldnew".to_vec());
            }
            if request.headers["range"].starts_with("bytes=0-") {
                Reply::bytes(206, b"old".to_vec()).header("Content-Range", "bytes 0-2/6")
            } else {
                Reply::bytes(206, b"new".to_vec()).header("Content-Range", "bytes 3-5/6")
            }
        });
        let client = mock.client();
        let first = client.trace(7, 42, 0).unwrap();
        assert_eq!(first.text, "old");
        let second = client.trace(7, 42, first.next_offset).unwrap();
        assert_eq!(second.text, "new");
        assert_eq!(second.next_offset, 6);
        assert!(!second.reset);
        assert_eq!(mock.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn trace_empty_probe_does_not_cache_false_support() {
        for supported in [false, true] {
            let data = Arc::new(Mutex::new(Vec::<u8>::new()));
            let server_data = data.clone();
            let mock = Mock::new(move |request| {
                let params: HashMap<_, _> = request.url().query_pairs().into_owned().collect();
                let data = server_data.lock().unwrap();
                if supported && params.contains_key("byte_limit") {
                    let offset: u64 = params["byte_offset"].parse().unwrap();
                    let limit: usize = params["byte_limit"].parse().unwrap();
                    let start = offset.min(data.len() as u64) as usize;
                    Reply::bytes(
                        200,
                        data[start..start.saturating_add(limit).min(data.len())].to_vec(),
                    )
                } else {
                    Reply::bytes(200, data.clone())
                }
            });
            let client = mock.client();
            assert!(client.trace(7, 42, 0).unwrap().text.is_empty());
            assert_eq!(client.trace_byte_params.load(Ordering::Relaxed), 0);
            data.lock().unwrap().extend_from_slice(b"hello");
            assert_eq!(client.trace(7, 42, 0).unwrap().text, "hello");
            assert_eq!(
                client.trace_byte_params.load(Ordering::Relaxed),
                if supported { 1 } else { 2 }
            );
        }
    }

    #[test]
    fn trace_parameter_probe_falls_back_on_400_but_propagates_other_errors() {
        let mock = Mock::new(|request| {
            if request.target.contains("byte_limit") {
                Reply::bytes(400, b"unsupported parameters".to_vec())
            } else {
                Reply::bytes(200, b"legacy".to_vec())
            }
        });
        assert_eq!(mock.client().trace(7, 42, 0).unwrap().text, "legacy");
        let rejects_offset = Mock::new(|request| {
            if request
                .target
                .contains(&format!("byte_offset={}", i64::MAX))
            {
                Reply::bytes(400, Vec::new())
            } else if request.target.contains("byte_limit") {
                Reply::bytes(200, b"l".to_vec())
            } else {
                Reply::bytes(200, b"legacy".to_vec())
            }
        });
        assert_eq!(
            rejects_offset.client().trace(7, 42, 0).unwrap().text,
            "legacy"
        );
        for status in [401, 403, 404, 429, 500] {
            let mock = Mock::new(move |_| Reply::bytes(status, Vec::new()));
            let client = mock.client();
            let error = client.trace(7, 42, 0).unwrap_err();
            assert_eq!(http_status(&error).unwrap().as_u16(), status);
            assert_eq!(client.trace_byte_params.load(Ordering::Relaxed), 0);
            assert_eq!(mock.requests.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn trace_parameter_reads_are_bounded_and_do_not_scan_large_prefixes() {
        let mock = Mock::new(|request| {
            let params: HashMap<_, _> = request.url().query_pairs().into_owned().collect();
            assert_eq!(params["byte_offset"], (TRACE_SKIP_LIMIT + 1).to_string());
            assert_eq!(params["byte_limit"], TRACE_BYTES.to_string());
            Reply::bytes(200, b"tail".to_vec())
        });
        let client = mock.client();
        client.trace_byte_params.store(1, Ordering::Relaxed);
        let (bytes, start, reset) = client
            .trace_bytes(
                &client.url(&["projects", "7", "jobs", "42", "trace"]),
                TRACE_SKIP_LIMIT + 1,
                false,
            )
            .unwrap();
        assert_eq!(bytes, b"tail");
        assert_eq!(start, TRACE_SKIP_LIMIT + 1);
        assert!(!reset);
        let oversized = Mock::new(|_| Reply::bytes(200, vec![b'x'; TRACE_BYTES + 1]));
        let client = oversized.client();
        client.trace_byte_params.store(1, Ordering::Relaxed);
        assert!(
            client
                .trace(7, 42, 0)
                .unwrap_err()
                .to_string()
                .contains("exceeds byte_limit")
        );
        let compressed =
            Mock::new(|_| Reply::bytes(200, b"gzip".to_vec()).header("Content-Encoding", "gzip"));
        assert!(
            compressed
                .client()
                .trace(7, 42, 0)
                .unwrap_err()
                .to_string()
                .contains("compressed trace")
        );
    }

    #[test]
    fn trace_byte_offsets_unicode_and_split_osc_sequences() {
        let chunks = [
            b"a\xc3".to_vec(),
            b"\xa9\x1b]52;c;secret".to_vec(),
            b"\x07\x1b[31mRED\x1b[0m\r\n".to_vec(),
        ];
        let total: usize = chunks.iter().map(Vec::len).sum();
        let mut index = 0;
        let mut offset = 0;
        let mock = Mock::new(move |request| {
            assert_eq!(
                request.headers["range"],
                format!("bytes={offset}-{}", offset + TRACE_BYTES - 1)
            );
            assert_eq!(request.headers["accept-encoding"], "identity");
            let bytes = chunks[index].clone();
            index += 1;
            let header = format!("bytes {offset}-{}/{total}", offset + bytes.len() - 1);
            offset += bytes.len();
            Reply::bytes(206, bytes).header("Content-Range", &header)
        });
        let client = mock.legacy_trace_client();
        let first = client.trace(7, 42, 0).unwrap();
        assert_eq!(first.text, "a");
        assert_eq!(first.next_offset, 2);
        let second = client.clone().trace(7, 42, first.next_offset).unwrap();
        assert_eq!(second.text, "é");
        let third = client.trace(7, 42, second.next_offset).unwrap();
        assert_eq!(third.text, "\x1b[31mRED\x1b[0m\n");
        assert_eq!(third.next_offset, total as u64);
        assert!(!third.reset);
    }

    #[test]
    fn trace_ignored_ranges_skip_prefix_and_reset_shorter_logs() {
        let mut first = true;
        let mock = Mock::new(move |_| {
            let body = if first {
                b"old".to_vec()
            } else {
                b"old\xc3\xa9\x1b[32mnew\x1b[0m\n".to_vec()
            };
            first = false;
            Reply::bytes(200, body)
        });
        let client = mock.legacy_trace_client();
        assert_eq!(client.trace(7, 42, 0).unwrap().text, "old");
        let chunk = client.trace(7, 42, 3).unwrap();
        assert_eq!(chunk.text, "é\x1b[32mnew\x1b[0m\n");
        assert_eq!(chunk.next_offset, 18);
        assert!(!chunk.reset);
        let mut first = true;
        let short = Mock::new(move |_| {
            let body = if first {
                vec![b'x'; 100]
            } else {
                b"new\n".to_vec()
            };
            first = false;
            Reply::bytes(200, body)
        });
        let client = short.legacy_trace_client();
        client.trace(7, 42, 0).unwrap();
        let chunk = client.trace(7, 42, 100).unwrap();
        assert!(chunk.reset);
        assert_eq!(chunk.text, "new\n");
        assert_eq!(chunk.next_offset, 4);
        let mut first = true;
        let chunked = Mock::new(move |_| {
            let body = if first {
                vec![b'x'; 100]
            } else {
                b"new\n".to_vec()
            };
            first = false;
            Reply::bytes(200, body).chunked()
        });
        let client = chunked.legacy_trace_client();
        client.trace(7, 42, 0).unwrap();
        let chunk = client.trace(7, 42, 100).unwrap();
        assert!(chunk.reset);
        assert_eq!(chunk.text, "new\n");
        assert_eq!(chunk.next_offset, 4);
    }

    #[test]
    fn trace_416_at_eof_and_after_truncation() {
        let mock = Mock::new(|request| {
            if request.headers["range"].starts_with("bytes=0-") {
                Reply::bytes(206, b"new".to_vec()).header("Content-Range", "bytes 0-2/3")
            } else {
                Reply::bytes(416, Vec::new()).header("Content-Range", "bytes */3")
            }
        });
        let client = mock.legacy_trace_client();
        client.trace(7, 42, 0).unwrap();
        let eof = client.trace(7, 42, 3).unwrap();
        assert!(eof.text.is_empty());
        assert_eq!(eof.next_offset, 3);
        assert!(!eof.reset);
        let (bytes, start, reset) = client
            .trace_bytes(
                &client.url(&["projects", "7", "jobs", "42", "trace"]),
                9,
                false,
            )
            .unwrap();
        assert!(reset);
        assert_eq!(bytes, b"new");
        assert_eq!(start, 0);
        let bad = Mock::new(|_| Reply::bytes(416, Vec::new()));
        assert!(
            bad.legacy_trace_client()
                .trace(7, 42, 1)
                .unwrap_err()
                .to_string()
                .contains("416 without")
        );
    }

    #[test]
    fn trace_response_caps_and_range_validation() {
        let large = Mock::new(|_| Reply::bytes(200, vec![b'a'; TRACE_BYTES * 2]));
        let chunk = large.legacy_trace_client().trace(7, 42, 0).unwrap();
        assert_eq!(chunk.text.len(), TRACE_BYTES);
        assert_eq!(chunk.next_offset, TRACE_BYTES as u64);
        let large_206 = Mock::new(|_| {
            Reply::bytes(206, vec![b'b'; TRACE_BYTES * 2]).header(
                "Content-Range",
                &format!("bytes 0-{}/{}", TRACE_BYTES * 2 - 1, TRACE_BYTES * 2),
            )
        });
        assert_eq!(
            large_206
                .legacy_trace_client()
                .trace(7, 42, 0)
                .unwrap()
                .text
                .len(),
            TRACE_BYTES
        );
        let ignored = Mock::new(|_| Reply::bytes(200, vec![b'a'; 16]).chunked());
        let client = ignored.legacy_trace_client();
        assert!(
            client
                .trace_bytes(
                    &client.url(&["projects", "7", "jobs", "42", "trace"]),
                    TRACE_SKIP_LIMIT + 1,
                    false
                )
                .unwrap_err()
                .to_string()
                .contains("scan limit")
        );
        let invalid = Mock::new(|_| {
            Reply::bytes(206, b"bad".to_vec()).header("Content-Range", "bytes 1-3/4")
        });
        assert!(
            invalid
                .legacy_trace_client()
                .trace(7, 42, 0)
                .unwrap_err()
                .to_string()
                .contains("wrong byte offset")
        );
        let short =
            Mock::new(|_| Reply::bytes(206, b"x".to_vec()).header("Content-Range", "bytes 0-3/4"));
        assert!(
            short
                .legacy_trace_client()
                .trace(7, 42, 0)
                .unwrap_err()
                .to_string()
                .contains("shorter than")
        );
        let encoded =
            Mock::new(|_| Reply::bytes(200, b"gzip".to_vec()).header("Content-Encoding", "gzip"));
        assert!(
            encoded
                .legacy_trace_client()
                .trace(7, 42, 0)
                .unwrap_err()
                .to_string()
                .contains("compressed trace")
        );
    }

    #[test]
    fn sanitizer_handles_all_split_boundaries_and_growing_utf8() {
        let bytes = "A\u{1b}]0;title\u{1b}\\B\u{1b}]8;;https://demo.invalid\u{7}link\u{1b}]8;;\u{7}\u{1b}Pprivate\u{1b}\\\u{1b}[31mC\u{1b}[0m\u{9d}secret\u{9c}é\n\t\r\u{0}\u{7}".as_bytes();
        for split in 0..=bytes.len() {
            let mut sanitizer = Sanitizer::default();
            let mut text = sanitizer.feed(&bytes[..split]);
            text.push_str(&sanitizer.feed(&bytes[split..]));
            assert_eq!(text, "ABlink\x1b[31mC\x1b[0mé\n\t", "split {split}");
        }
        let mut sanitizer = Sanitizer::default();
        assert_eq!(sanitizer.feed(b"\xc3"), "");
        assert_eq!(sanitizer.feed(b""), "");
        assert_eq!(sanitizer.feed(b"\xa9"), "é");
        assert_eq!(
            sanitizer.feed(b"\x9dhidden\x07ok\x9b31mred\x9b0m"),
            "ok\x1b[31mred\x1b[0m"
        );
    }

    #[test]
    fn sanitizer_preserves_sgr_but_rejects_commands_and_unbounded_parameters() {
        let input = "start\x1b[2J\x1b[?25l\x1b[?31m\x1b[999999999999999999999m\x1b[1;91;48;5;235mRED\x1b[m\x1b]52;c;CLIPBOARD\x07\x1bPPRIVATE\x1b\\end";
        for split in 0..=input.len() {
            let mut sanitizer = Sanitizer::default();
            let mut text = sanitizer.feed(&input.as_bytes()[..split]);
            text.push_str(&sanitizer.feed(&input.as_bytes()[split..]));
            assert_eq!(
                text, "start\x1b[1;91;48;5;235mRED\x1b[0mend",
                "split {split}"
            );
        }
        let mut sanitizer = Sanitizer::default();
        assert!(
            sanitizer
                .feed(format!("\x1b[{}", "1".repeat(20_000)).as_bytes())
                .is_empty()
        );
        assert!(sanitizer.csi.len() <= crate::ansi::SGR_LIMIT);
        assert_eq!(sanitizer.feed(b"mstill visible"), "still visible");
        assert_eq!(
            sanitizer.feed(b"\x1b[38:2::12:34:56mRGB\x1b[0m"),
            "\x1b[38;2;12;34;56mRGB\x1b[0m"
        );
    }

    #[test]
    fn disabled_issues_do_not_hide_healthy_merge_requests() {
        for status in [403, 404] {
            let mock = Mock::new(move |request| match request.url().path() {
                "/api/v4/projects/7/issues" => Reply::bytes(status, Vec::new()),
                "/api/v4/projects/7" => {
                    Reply::json(json!({"id": 7, "issues_access_level": "disabled"}))
                }
                "/api/v4/projects/7/merge_requests" => Reply::json(json!([raw_item(1, "merged")])),
                _ => panic!("unexpected request {request:?}"),
            });
            let items = mock.client().list_project(&project()).unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].key.kind, ItemKind::MergeRequest);
        }
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7/issues" => Reply::json(json!([raw_item(1, "opened")])),
            "/api/v4/projects/7/merge_requests" => Reply::bytes(403, Vec::new()),
            "/api/v4/projects/7" => Reply::json(json!({"id": 7, "merge_requests_enabled": false})),
            _ => panic!("unexpected request {request:?}"),
        });
        assert_eq!(
            mock.client().list_project(&project()).unwrap()[0].key.kind,
            ItemKind::Issue
        );
    }

    #[test]
    fn feature_permissions_and_failed_verification_are_not_empty_collections() {
        for features in [
            json!({"id": 7}),
            json!({"id": 7, "issues_access_level": "private"}),
            json!({"id": 7, "issues_access_level": "enabled", "issues_enabled": false}),
            json!({"id": 8, "issues_enabled": false}),
        ] {
            let mock = Mock::new(move |request| {
                if request.url().path().ends_with("/issues") {
                    Reply::bytes(403, Vec::new())
                } else {
                    Reply::json(features.clone())
                }
            });
            assert!(mock.client().list_project(&project()).is_err());
        }
        let mock = Mock::new(|request| {
            if request.url().path().ends_with("/issues") {
                Reply::bytes(404, Vec::new())
            } else {
                Reply::bytes(429, Vec::new()).header("Retry-After", "90")
            }
        });
        let error = mock
            .client()
            .list_project(&project())
            .unwrap_err()
            .to_string();
        assert!(error.contains("Retry-After: 90"));
    }

    #[test]
    fn dashboard_counts_discussion_threads_even_when_pipeline_is_supplied() {
        let mock = Mock::new(|request| {
            let mut item = raw_item(1, "opened");
            item["head_pipeline"] = json!({"id": 77, "status": "success"});
            item["blocking_discussions_resolved"] = json!(true);
            match request.url().path() {
                "/api/v4/projects/7/issues" => Reply::json(json!([])),
                "/api/v4/projects/7/merge_requests" => Reply::json(json!([item])),
                "/api/v4/projects/7/merge_requests/1" => Reply::json(item),
                "/api/v4/projects/7/merge_requests/1/discussions" => {
                    if request.page() == 1 {
                        Reply::json(json!([{"id": "one", "notes": [
                            {"resolvable": true, "resolved": false},
                            {"resolvable": true, "resolved": false}
                        ]}]))
                        .header("X-Next-Page", "2")
                    } else {
                        Reply::json(json!([
                            {"id": "two", "notes": [{"resolvable": true, "resolved": false}]},
                            {"id": "resolved", "notes": [{"resolvable": true, "resolved": true}]}
                        ]))
                        .header("X-Next-Page", "")
                    }
                }
                _ => panic!("unexpected request {request:?}"),
            }
        });
        let items = mock.client().list_project(&project()).unwrap();
        assert_eq!(items[0].unresolved, Some(2));
        assert_eq!(items[0].pipeline.as_ref().unwrap().id, 77);
    }

    #[test]
    fn dashboard_budget_and_unsupported_enrichment_never_fabricate_counts() {
        for flag in [false, true] {
            let mut raw = raw_item(1, "opened");
            raw["blocking_discussions_resolved"] = json!(flag);
            assert_eq!(
                serde_json::from_value::<ApiItem>(raw)
                    .unwrap()
                    .into_item(7, ItemKind::MergeRequest)
                    .unresolved,
                None
            );
        }
        for status in [403, 404, 405, 501] {
            let mock = Mock::new(move |request| {
                if request.url().path().ends_with("/issues") {
                    Reply::json(json!([]))
                } else if request.url().path().ends_with("/merge_requests") {
                    Reply::json(json!([raw_item(1, "opened")]))
                } else {
                    Reply::bytes(status, Vec::new())
                }
            });
            let items = mock.client().list_project(&project()).unwrap();
            assert_eq!(items.len(), 1);
            assert!(items[0].pipeline.is_none() && items[0].unresolved.is_none());
        }
        let mock = Mock::new(|request| {
            match request.url().path() {
            "/api/v4/projects/7/issues" => Reply::json(json!([])),
            "/api/v4/projects/7/merge_requests" => Reply::json(json!([raw_item(1, "opened")])),
            "/api/v4/projects/7/merge_requests/1" => Reply::json(raw_item(1, "opened")),
            _ => Reply::json(json!([{"id": request.page().to_string(), "notes": [{"resolvable": true, "resolved": false}]}]))
                .header("X-Next-Page", &(request.page() + 1).to_string()),
        }
        });
        let items = mock.client().list_project(&project()).unwrap();
        assert_eq!(items[0].unresolved, None);
        assert_eq!(
            mock.requests.lock().unwrap().len(),
            3 + DASHBOARD_DISCUSSION_PAGES
        );
    }

    #[test]
    fn dashboard_discussion_rate_limits_are_still_surfaced() {
        let mock = Mock::new(|request| match request.url().path() {
            "/api/v4/projects/7/issues" => Reply::json(json!([])),
            "/api/v4/projects/7/merge_requests" => Reply::json(json!([raw_item(1, "opened")])),
            "/api/v4/projects/7/merge_requests/1" => Reply::json(raw_item(1, "opened")),
            _ => Reply::bytes(429, Vec::new()).header("Retry-After", "60"),
        });
        assert!(
            mock.client()
                .list_project(&project())
                .unwrap_err()
                .to_string()
                .contains("Retry-After: 60")
        );
    }

    #[test]
    fn trace_parser_is_scoped_to_project_and_job_and_restarts_after_eviction() {
        let mut seen = HashMap::<String, usize>::new();
        let mock = Mock::new(move |request| {
            let seen = seen.entry(request.target.clone()).or_default();
            *seen += 1;
            if request.target.contains("/projects/7/") && request.target.contains("/jobs/42/") {
                if *seen == 1 {
                    Reply::bytes(206, b"\x1b]52;c;".to_vec())
                        .header("Content-Range", "bytes 0-6/20")
                } else {
                    assert!(request.headers["range"].starts_with("bytes=0-"));
                    Reply::bytes(200, b"\x1b]52;c;PRIVATE\x07safe".to_vec())
                }
            } else {
                Reply::bytes(200, b"visible".to_vec())
            }
        });
        let client = mock.legacy_trace_client();
        let hidden = client.trace(7, 42, 0).unwrap();
        assert!(hidden.text.is_empty());
        assert_eq!(hidden.next_offset, 7);
        assert_eq!(client.trace(8, 42, 0).unwrap().text, "visible");
        assert_eq!(client.trace(7, 43, 0).unwrap().text, "visible");
        // Evict the idle original job without sharing its OSC state with any other job.
        for job in 100..100 + TRACE_STATES as u64 {
            client.trace(7, job, 0).unwrap();
        }
        assert_eq!(client.traces.lock().unwrap().len(), TRACE_STATES);
        let resumed = client.trace(7, 42, hidden.next_offset).unwrap();
        assert!(resumed.reset);
        assert_eq!(resumed.text, "safe");
        assert_eq!(resumed.next_offset, 19);
    }

    #[test]
    fn nonzero_trace_offset_without_state_does_not_expose_osc_payload() {
        let mock = Mock::new(|request| {
            assert!(request.headers["range"].starts_with("bytes=0-"));
            Reply::bytes(200, b"\x1b]52;c;secret\x07safe".to_vec())
        });
        let chunk = mock.legacy_trace_client().trace(7, 42, 7).unwrap();
        assert!(chunk.reset);
        assert_eq!(chunk.text, "safe");
        assert_eq!(chunk.next_offset, 18);
    }

    #[test]
    fn trace_job_lock_does_not_block_another_jobs_network_request() {
        let mock = Mock::new(|request| {
            if request.target.contains("/jobs/42/") {
                if request.headers["range"].starts_with("bytes=0-") {
                    Reply::bytes(206, b"\x1b]52;c;".to_vec())
                        .header("Content-Range", "bytes 0-6/18")
                } else {
                    Reply::bytes(206, b"secret\x07safe".to_vec())
                        .header("Content-Range", "bytes 7-17/18")
                }
            } else {
                Reply::bytes(200, b"other job".to_vec())
            }
        });
        let client = mock.legacy_trace_client();
        client.trace(7, 42, 0).unwrap();
        let state = client.traces.lock().unwrap()[0].clone();
        let guard = state.cursor.lock().unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let same_client = client.clone();
        let same = thread::spawn(move || {
            started_tx.send(()).unwrap();
            same_client.trace(7, 42, 7)
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let other = thread::spawn(move || {
            done_tx.send(client.trace(7, 43, 0)).unwrap();
        });
        let result = done_rx.recv_timeout(Duration::from_secs(5));
        // Release the lock before any assertion so a regression cannot leave blocked workers.
        let requests_before_unlock = mock.requests.lock().unwrap().len();
        drop(guard);
        let same = same.join().unwrap().unwrap();
        other.join().unwrap();
        assert_eq!(result.unwrap().unwrap().text, "other job");
        assert_eq!(requests_before_unlock, 2);
        assert_eq!(same.text, "safe");
        assert_eq!(same.next_offset, 18);
        assert!(!same.reset);
    }

    #[test]
    fn json_response_limit_is_explicit() {
        let mock = Mock::new(|_| Reply::bytes(200, vec![b' '; JSON_LIMIT + 1]));
        assert!(
            mock.client()
                .current_user()
                .unwrap_err()
                .to_string()
                .contains("safety limit")
        );
    }
}
