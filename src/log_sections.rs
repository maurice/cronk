//! GitLab's custom section metadata, kept separate from executable terminal controls.
//! https://docs.gitlab.com/ci/jobs/job_logs/#create-custom-collapsible-sections
pub(crate) const PREFIX: &str = "\x1b[0K";

pub(crate) struct Marker<'a> {
    pub start: bool,
    pub timestamp: u64,
    pub name: &'a str,
    pub collapsed: bool,
}

pub(crate) fn marker(text: &str) -> Option<Marker<'_>> {
    let (start, rest) = if let Some(rest) = text.strip_prefix("section_start:") {
        (true, rest)
    } else {
        (false, text.strip_prefix("section_end:")?)
    };
    let (timestamp, rest) = rest.split_once(':')?;
    if timestamp.is_empty() || !timestamp.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (name, options) = rest.split_once('[').unwrap_or((rest, ""));
    if rest.contains('[') && (!start || !options.ends_with(']')) {
        return None;
    }
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return None;
    }
    Some(Marker {
        start,
        timestamp: timestamp.parse().ok()?,
        name,
        collapsed: options
            .trim_end_matches(']')
            .split(',')
            .any(|s| s == "collapsed=true"),
    })
}
