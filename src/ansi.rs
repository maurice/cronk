//! Bounded SGR handling shared by trace sanitization and viewport style checkpoints.
//! No terminal commands are executed: retained SGR is parsed into widget styles.

pub(crate) const SGR_LIMIT: usize = 128;

/// Normalize numeric SGR parameters, rejecting intermediates/private markers and
/// numbers that could overflow downstream parsers. Support the optional zero/empty
/// color-space slot in colon-form truecolor sequences.
pub(crate) fn normalize_sgr(params: &str) -> Option<String> {
    if params.len() > SGR_LIMIT
        || !params
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
    {
        return None;
    }
    let mut tokens = Vec::new();
    for group in params.split(';') {
        let mut parts: Vec<_> = group.split(':').collect();
        if parts.len() == 6 && matches!(parts[0], "38" | "48") && parts[1] == "2" {
            if !matches!(parts[2], "" | "0") {
                return None;
            }
            parts.remove(2);
        }
        for part in parts {
            if part.len() > 5 {
                return None;
            }
            tokens.push(if part.is_empty() {
                0
            } else {
                part.parse::<u32>().ok()?
            });
            if tokens.len() > 32 {
                return None;
            }
        }
    }
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], 38 | 48) {
            match tokens.get(i + 1) {
                Some(5) => {
                    if *tokens.get(i + 2)? > 255 {
                        return None;
                    }
                    i += 3;
                }
                Some(2) => {
                    for value in tokens.get_mut(i + 2..i + 5)? {
                        *value = (*value).min(255);
                    }
                    i += 5;
                }
                _ => return None,
            }
        } else {
            i += 1;
        }
    }
    Some(
        tokens
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(";"),
    )
}

#[derive(Default)]
pub(crate) struct SgrState {
    fg: Option<String>,
    bg: Option<String>,
    attrs: u16,
}

impl SgrState {
    pub fn apply(&mut self, params: &str) {
        let Some(params) = normalize_sgr(params) else {
            return;
        };
        let tokens: Vec<u32> = params.split(';').filter_map(|n| n.parse().ok()).collect();
        let mut i = 0;
        while i < tokens.len() {
            let code = tokens[i];
            match code {
                0 => *self = Self::default(),
                1 | 2 | 3 | 4 | 7 | 9 => self.attrs |= 1 << code,
                22 => self.attrs &= !((1 << 1) | (1 << 2)),
                23 => self.attrs &= !(1 << 3),
                24 => self.attrs &= !(1 << 4),
                27 => self.attrs &= !(1 << 7),
                29 => self.attrs &= !(1 << 9),
                30..=37 | 90..=97 => self.fg = Some(code.to_string()),
                40..=47 | 100..=107 => self.bg = Some(code.to_string()),
                39 => self.fg = None,
                49 => self.bg = None,
                38 | 48 => {
                    let count = match tokens.get(i + 1) {
                        Some(5) => 3,
                        Some(2) => 5,
                        _ => 2,
                    };
                    if let Some(color) = tokens.get(i..i + count)
                        && count > 2
                        && color[2..].iter().all(|&n| n <= 255)
                    {
                        let value = color
                            .iter()
                            .map(u32::to_string)
                            .collect::<Vec<_>>()
                            .join(";");
                        if code == 38 {
                            self.fg = Some(value);
                        } else {
                            self.bg = Some(value);
                        }
                    }
                    i += count - 1;
                }
                _ => {}
            }
            i += 1;
        }
    }

    pub fn prefix(&self) -> String {
        let mut params = Vec::new();
        for code in [1, 2, 3, 4, 7, 9] {
            if self.attrs & (1 << code) != 0 {
                params.push(code.to_string());
            }
        }
        params.extend(self.fg.iter().chain(self.bg.iter()).cloned());
        if params.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", params.join(";"))
        }
    }
}

/// End (exclusive) of a CSI sequence beginning at an ESC byte. None means the
/// sequence is incomplete; indices are ASCII boundaries even in Unicode logs.
pub(crate) fn csi_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(start..start + 2)? != b"\x1b[" {
        return None;
    }
    bytes[start + 2..]
        .iter()
        .position(|b| (b'@'..=b'~').contains(b))
        .map(|n| start + 3 + n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_supports_extended_colors_and_rejects_invalid_parameters() {
        assert_eq!(
            normalize_sgr("38:2::12:34:56;48:5:235").as_deref(),
            Some("38;2;12;34;56;48;5;235")
        );
        assert_eq!(
            normalize_sgr("38;2;999;20;3").as_deref(),
            Some("38;2;255;20;3")
        );
        for invalid in [
            "99999999999999999",
            "?31",
            "31 ",
            "38;2;12",
            "48;5;256",
            "38;7;1",
        ] {
            assert!(normalize_sgr(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn checkpoints_track_default_colors_and_independent_attribute_resets() {
        let mut sgr = SgrState::default();
        sgr.apply("1;38;5;42;48;2;1;2;3");
        assert_eq!(sgr.prefix(), "\x1b[1;38;5;42;48;2;1;2;3m");
        sgr.apply("22;39");
        assert_eq!(sgr.prefix(), "\x1b[48;2;1;2;3m");
        sgr.apply("49");
        assert!(sgr.prefix().is_empty());
        sgr.apply("91;7");
        sgr.apply("0");
        assert!(sgr.prefix().is_empty());
    }
}
