//! Local AND queries: words/quoted phrases search titles and descriptions;
//! `field:value` restricts a field and `-field:value` negates that restriction.
//! Double quotes preserve spaces and colons; inside quotes, `\"` and `\\` escape
//! a quote and a backslash. Repeated fields are independent AND conditions.
//!
//! Matching is case-insensitive. Labels, states, usernames, and pipeline statuses
//! are exact; projects (path or alias), iterations, and milestones are substring
//! matches. User fields accept `@me` and optional `@` prefixes on usernames.

use crate::model::{ItemKind, Project, WorkItem};

#[derive(Clone, Debug, Default)]
pub struct Query {
    terms: Vec<Term>,
}

#[derive(Clone, Debug)]
struct Term {
    negated: bool,
    predicate: Predicate,
}

#[derive(Clone, Debug)]
enum Predicate {
    Text(String),
    Project(String),
    Label(String),
    State(String),
    Assignee(String),
    Author(String),
    Reviewer(String),
    Iteration(String),
    Milestone(String),
    Pipeline(String),
    Draft(bool),
    Kind(ItemKind),
}

impl Query {
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut terms = Vec::new();
        for (text, separator) in tokenize(source)? {
            let (negated, predicate) = if let Some(separator) = separator {
                let field = &text[..separator];
                let negated = field.starts_with('-');
                let field = field.strip_prefix('-').unwrap_or(field).to_lowercase();
                let value = text[separator + 1..].to_lowercase();
                if value.trim().is_empty() {
                    return Err(format!("field '{field}' requires a value"));
                }
                let predicate = match field.as_str() {
                    "project" => Predicate::Project(value),
                    "label" => Predicate::Label(value),
                    "state" | "status" => Predicate::State(value),
                    "assignee" => Predicate::Assignee(value),
                    "author" => Predicate::Author(value),
                    "reviewer" => Predicate::Reviewer(value),
                    "iteration" => Predicate::Iteration(value),
                    "milestone" => Predicate::Milestone(value),
                    "pipeline" => Predicate::Pipeline(value),
                    "draft" => Predicate::Draft(match value.as_str() {
                        "true" => true,
                        "false" => false,
                        _ => return Err("draft must be true or false".into()),
                    }),
                    "kind" => Predicate::Kind(match value.as_str() {
                        "issue" | "issues" => ItemKind::Issue,
                        "mr" | "merge_request" | "merge_requests" | "merge-request"
                        | "merge-requests" => ItemKind::MergeRequest,
                        _ => return Err("kind must be issue or merge_request (mr)".into()),
                    }),
                    _ => return Err(format!("unknown query field '{field}'")),
                };
                (negated, predicate)
            } else {
                if text.trim().is_empty() {
                    return Err("empty text term".into());
                }
                (false, Predicate::Text(text.to_lowercase()))
            };
            terms.push(Term { negated, predicate });
        }
        Ok(Self { terms })
    }

    /// An empty query matches everything. `project` must describe the item's
    /// project; `current_user` is its GitLab username (empty if unknown).
    pub fn matches(&self, item: &WorkItem, project: &Project, current_user: &str) -> bool {
        self.terms.iter().all(|term| {
            let matches = match &term.predicate {
                Predicate::Text(value) => {
                    contains(&item.title, value) || contains(&item.description, value)
                }
                Predicate::Project(value) => {
                    contains(&project.path, value) || contains(&project.alias, value)
                }
                Predicate::Label(value) => {
                    item.labels.iter().any(|label| equal(&label.name, value))
                }
                Predicate::State(value) => equal(&item.state, value),
                Predicate::Assignee(value) => item
                    .assignees
                    .iter()
                    .any(|user| user_matches(&user.username, value, current_user)),
                Predicate::Author(value) => {
                    user_matches(&item.author.username, value, current_user)
                }
                Predicate::Reviewer(value) => item
                    .reviewers
                    .iter()
                    .any(|user| user_matches(&user.username, value, current_user)),
                Predicate::Iteration(value) => contains(&item.iteration, value),
                Predicate::Milestone(value) => contains(&item.milestone, value),
                Predicate::Pipeline(value) => item
                    .pipeline
                    .as_ref()
                    .is_some_and(|pipeline| equal(&pipeline.status, value)),
                Predicate::Draft(value) => item.draft == *value,
                Predicate::Kind(value) => item.key.kind == *value,
            };
            matches != term.negated
        })
    }
}

// `value` has already been lowercased by the parser.
fn contains(text: &str, value: &str) -> bool {
    text.to_lowercase().contains(value)
}

fn equal(text: &str, value: &str) -> bool {
    text.to_lowercase() == value
}

fn user_matches(username: &str, value: &str, current_user: &str) -> bool {
    let expected = if value == "@me" { current_user } else { value };
    let expected = expected.strip_prefix('@').unwrap_or(expected);
    !expected.is_empty() && username.to_lowercase() == expected.to_lowercase()
}

/// Keep the first *unquoted* colon's byte offset, so a quoted free-text phrase
/// such as `"state:opened"` is not accidentally interpreted as a field filter.
fn tokenize(source: &str) -> Result<Vec<(String, Option<usize>)>, String> {
    let mut chars = source.chars().peekable();
    let mut tokens = Vec::new();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut text = String::new();
        let mut separator = None;
        let mut quoted = false;
        while let Some(&c) = chars.peek() {
            if !quoted && c.is_whitespace() {
                break;
            }
            chars.next();
            match c {
                '"' => quoted = !quoted,
                '\\' if quoted && chars.peek().is_some_and(|c| matches!(c, '"' | '\\')) => {
                    text.push(chars.next().expect("peeked escape character"));
                }
                ':' if !quoted && separator.is_none() => {
                    separator = Some(text.len());
                    text.push(c);
                }
                _ => text.push(c),
            }
        }
        if quoted {
            return Err("unclosed double quote".into());
        }
        tokens.push((text, separator));
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemKey, Label, Pipeline, User};

    fn fixture() -> (WorkItem, Project) {
        let project = Project {
            id: 42,
            path: "Platform/Control-Plane".into(),
            alias: "Core Services".into(),
            visible: true,
        };
        let user = |username: &str, name: &str| User {
            username: username.into(),
            name: name.into(),
            ..User::default()
        };
        let item = WorkItem {
            key: ItemKey {
                project: 42,
                iid: 7,
                kind: ItemKind::MergeRequest,
            },
            title: "Fix OAuth Login".into(),
            description: "Handle expired sessions. Café support; can't reproduce state:opened."
                .into(),
            state: "opened".into(),
            labels: ["Bug", "Needs Review", "team::backend"]
                .into_iter()
                .map(|name| Label {
                    name: name.into(),
                    ..Label::default()
                })
                .collect(),
            author: user("alice", "Alice Example"),
            assignees: vec![user("bob", "Bob Example"), user("carol", "Carol Example")],
            reviewers: vec![user("alice", "Alice Example"), user("dave", "Dave Example")],
            milestone: "Release 1.2".into(),
            iteration: "Sprint 12".into(),
            draft: false,
            pipeline: Some(Pipeline {
                status: "failed".into(),
                ..Pipeline::default()
            }),
            ..WorkItem::default()
        };
        (item, project)
    }

    fn matches(source: &str) -> bool {
        let (item, project) = fixture();
        Query::parse(source)
            .unwrap()
            .matches(&item, &project, "alice")
    }

    #[test]
    fn empty_queries_match_everything() {
        for source in ["", " ", "\t\n\r\u{2003}"] {
            assert!(matches(source));
        }
        assert!(Query::default().matches(&WorkItem::default(), &Project::default(), ""));
    }

    #[test]
    fn free_text_is_case_insensitive_substring_and_searches_title_and_description() {
        for source in [
            "oauth",
            "AUTH",
            "expired",
            "sessions",
            "CAFÉ",
            "can't",
            "login expired",
            "\nlogin\tEXPIRED\n",
        ] {
            assert!(matches(source), "{source}");
        }
        for source in ["missing", "login missing", "alice", "Platform"] {
            assert!(!matches(source), "{source}");
        }
    }

    #[test]
    fn double_quoted_phrases_preserve_whitespace_and_colons() {
        for source in [
            r#""oauth login""#,
            r#""expired sessions""#,
            r#""state:opened""#,
            r#"label:"needs review""#,
        ] {
            assert!(matches(source), "{source}");
        }
        assert!(!matches(r#""login oauth""#));
        assert!(!matches(r#""expired  sessions""#));
        assert!(!matches(r#""unknown:value""#));
    }

    #[test]
    fn project_matches_path_or_alias_substrings_not_only_display_name() {
        for source in [
            "project:platform",
            "project:CONTROL",
            "project:services",
            r#"project:"core services""#,
            "project:plane project:core",
        ] {
            assert!(matches(source), "{source}");
        }
        assert!(!matches("project:other"));
        assert!(matches("-project:other"));
        assert!(!matches("-project:platform"));
    }

    #[test]
    fn labels_are_exact_and_repeated_labels_are_anded() {
        assert!(matches(
            r#"label:BUG label:"Needs Review" label:team::backend"#
        ));
        assert!(matches(r#"label:"team::backend" label:bug label:bug"#));
        for source in [
            "label:bu",
            "label:review",
            "label:backend",
            "label:bug label:missing",
            "label:bug -label:bug",
        ] {
            assert!(!matches(source), "{source}");
        }
        assert!(matches("label:bug -label:wontfix"));
        assert!(!matches(r#"-label:"needs review""#));
    }

    #[test]
    fn state_and_status_are_exact_aliases() {
        for source in [
            "state:OPENED",
            "STATUS:opened",
            "state:opened status:opened",
            "-state:closed",
        ] {
            assert!(matches(source), "{source}");
        }
        for source in [
            "state:open",
            "state:closed",
            "status:pen",
            "state:opened status:closed",
        ] {
            assert!(!matches(source), "{source}");
        }
    }

    #[test]
    fn all_user_fields_match_exact_usernames_not_display_names() {
        for source in [
            "author:ALICE",
            "author:@alice",
            "assignee:bob",
            "assignee:carol",
            "reviewer:dave",
            "reviewer:alice",
            "assignee:bob assignee:carol",
        ] {
            assert!(matches(source), "{source}");
        }
        for source in [
            "author:ali",
            "assignee:bo",
            "reviewer:dav",
            r#"author:"Alice Example""#,
            "assignee:alice",
            "reviewer:bob",
        ] {
            assert!(!matches(source), "{source}");
        }
    }

    #[test]
    fn me_resolves_dynamically_in_all_user_fields_and_negations() {
        let (item, project) = fixture();
        for (source, yes, no) in [
            ("author:@me", "ALICE", "bob"),
            ("assignee:@ME", "bob", "alice"),
            ("reviewer:@me", "dave", "bob"),
            ("-author:@me", "bob", "alice"),
            ("-assignee:@me", "alice", "bob"),
            ("-reviewer:@me", "bob", "dave"),
        ] {
            let query = Query::parse(source).unwrap();
            assert!(query.matches(&item, &project, yes), "{source}");
            assert!(!query.matches(&item, &project, no), "{source}");
        }
        assert!(
            Query::parse("author:@me")
                .unwrap()
                .matches(&item, &project, "@Alice")
        );
    }

    #[test]
    fn unknown_current_user_does_not_match_empty_or_literal_me_users() {
        let mut item = WorkItem::default();
        let project = Project::default();
        for username in ["", "@me", "me"] {
            item.author.username = username.into();
            for current_user in ["", "@"] {
                assert!(!Query::parse("author:@me").unwrap().matches(
                    &item,
                    &project,
                    current_user
                ));
                assert!(Query::parse("-author:@me").unwrap().matches(
                    &item,
                    &project,
                    current_user
                ));
            }
        }
    }

    #[test]
    fn iteration_and_milestone_match_case_insensitive_title_substrings() {
        for source in [
            r#"iteration:"SPRINT 12""#,
            "iteration:12",
            "milestone:RELEASE",
            "milestone:1.2",
            "iteration:sprint milestone:release",
        ] {
            assert!(matches(source), "{source}");
        }
        assert!(!matches("iteration:13"));
        assert!(!matches("milestone:1.3"));
        assert!(matches("-iteration:13 -milestone:1.3"));
    }

    #[test]
    fn pipeline_matches_exact_status_and_missing_pipeline_only_matches_negation() {
        assert!(matches("pipeline:FAILED"));
        assert!(!matches("pipeline:fail"));
        assert!(!matches("pipeline:success"));
        let (mut item, project) = fixture();
        item.pipeline = None;
        assert!(
            !Query::parse("pipeline:failed")
                .unwrap()
                .matches(&item, &project, "alice")
        );
        assert!(
            Query::parse("-pipeline:failed")
                .unwrap()
                .matches(&item, &project, "alice")
        );
    }

    #[test]
    fn draft_boolean_and_negation_match_both_states() {
        let (mut item, project) = fixture();
        for draft in [false, true] {
            item.draft = draft;
            for (source, expected) in [
                ("draft:true", draft),
                ("draft:FALSE", !draft),
                ("-draft:true", !draft),
                ("-draft:false", draft),
            ] {
                assert_eq!(
                    Query::parse(source)
                        .unwrap()
                        .matches(&item, &project, "alice"),
                    expected,
                    "{source}"
                );
            }
        }
        assert!(matches(r#"draft:"false""#));
    }

    #[test]
    fn kind_uses_the_shared_enum_and_accepts_mr_aliases() {
        for kind in [
            "mr",
            "MR",
            "merge_request",
            "merge_requests",
            "merge-request",
            "merge-requests",
        ] {
            assert!(matches(&format!("kind:{kind}")), "{kind}");
        }
        assert!(!matches("kind:issue"));
        assert!(matches("-kind:issues"));
        let (mut item, project) = fixture();
        item.key.kind = ItemKind::Issue;
        for source in ["kind:issue", "kind:issues", "-kind:mr"] {
            assert!(
                Query::parse(source)
                    .unwrap()
                    .matches(&item, &project, "alice")
            );
        }
    }

    #[test]
    fn complex_query_combines_every_field_with_and_semantics() {
        let query = r#"FIX "expired sessions" project:platform label:bug label:"needs review" state:opened status:opened assignee:bob author:@me reviewer:@me iteration:"sprint 12" milestone:1.2 pipeline:failed draft:false kind:mr -label:wontfix -assignee:alice"#;
        assert!(matches(query));
        assert!(!matches(&format!("{query} label:missing")));
        assert!(!matches(&format!("{query} -pipeline:failed")));
    }

    #[test]
    fn unknown_fields_invalid_booleans_and_kinds_are_errors_even_when_negated() {
        for source in [
            "unknown:value",
            "-unknown:value",
            "labels:bug",
            "title:fix",
            ":value",
            "-:value",
            "draft:yes",
            "draft:no",
            "draft:1",
            "draft:0",
            "-draft:maybe",
            r#"draft:" true ""#,
            "kind:ticket",
            "-kind:commit",
        ] {
            assert!(Query::parse(source).is_err(), "{source}");
        }
        assert!(
            Query::parse("unknown:value")
                .unwrap_err()
                .contains("unknown query field")
        );
        assert!(
            Query::parse("draft:yes")
                .unwrap_err()
                .contains("true or false")
        );
    }

    #[test]
    fn unclosed_quotes_are_errors_in_text_fields_and_negated_fields() {
        for source in [
            "\"",
            "\"unfinished",
            "label:\"needs review",
            "-label:\"needs review",
            "text \"unfinished",
            "label:\"escaped\\\"",
        ] {
            assert!(
                Query::parse(source).unwrap_err().contains("unclosed"),
                "{source}"
            );
        }
    }

    #[test]
    fn empty_values_and_terms_are_rejected() {
        for source in [
            "label:",
            "-label:",
            "label: ",
            r#"label:"""#,
            r#"label:"   ""#,
            r#""""#,
            r#"" ""#,
            "project: label:bug",
        ] {
            assert!(Query::parse(source).is_err(), "{source}");
        }
    }

    #[test]
    fn escaped_quotes_and_backslashes_in_quoted_values_are_preserved() {
        let (mut item, project) = fixture();
        item.labels = vec![Label {
            name: "say \"hello\" at C:\\work".into(),
            ..Label::default()
        }];
        let query = Query::parse(r#"label:"say \"hello\" at C:\\work""#).unwrap();
        assert!(query.matches(&item, &project, "alice"));
        item.description = r"Keep C:\work\new literal".into();
        assert!(
            Query::parse(r#""C:\work\new""#)
                .unwrap()
                .matches(&item, &project, "alice")
        );
    }

    #[test]
    fn quoted_free_text_is_not_reinterpreted_as_fields_or_negation() {
        let (mut item, project) = fixture();
        item.title = "-label:bug unknown:value -dash".into();
        for source in [r#""-label:bug""#, r#""unknown:value""#, "-dash"] {
            assert!(
                Query::parse(source)
                    .unwrap()
                    .matches(&item, &project, "alice")
            );
        }
    }

    #[test]
    fn unicode_before_colons_cannot_break_byte_boundaries() {
        assert!(Query::parse("étiquette:bug").is_err());
        let (mut item, project) = fixture();
        item.labels = vec![Label {
            name: "Équipe::核心".into(),
            ..Label::default()
        }];
        assert!(
            Query::parse("label:équipe::核心")
                .unwrap()
                .matches(&item, &project, "alice")
        );
    }

    #[test]
    fn missing_optional_fields_fail_positive_terms_and_pass_negative_terms() {
        let item = WorkItem::default();
        let project = Project::default();
        for field in [
            "label",
            "assignee",
            "reviewer",
            "iteration",
            "milestone",
            "pipeline",
            "project",
        ] {
            assert!(
                !Query::parse(&format!("{field}:anything"))
                    .unwrap()
                    .matches(&item, &project, "")
            );
            assert!(
                Query::parse(&format!("-{field}:anything"))
                    .unwrap()
                    .matches(&item, &project, "")
            );
        }
    }
}
