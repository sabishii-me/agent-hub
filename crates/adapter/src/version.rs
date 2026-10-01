//! The plugin/host version boundary (ADR-0008): a back-end plugin declares a
//! `minHubVersion` and the host ACTIVATES it only when the host's own version is at
//! least that. One lower bound, no upper bound, no compatibility matrix. A plugin
//! below the requirement is NOT activated and the reason is reported.

/// The hub's own version, from the crate metadata (single source: `Cargo.toml`).
pub const HUB_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parse a dotted numeric version (`MAJOR.MINOR.PATCH`) into a comparable tuple. A
/// non-numeric or missing segment is 0, so `1.2` == `1.2.0`. A leading `v` is
/// ignored. This is deliberately small: the plugin model has ONE lower bound.
pub fn parse(v: &str) -> (u64, u64, u64) {
    let v = v.trim().trim_start_matches('v');
    let mut it = v.split('.').map(|s| {
        // Take the leading digits of a segment (`1-beta` -> 1).
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse::<u64>().unwrap_or(0)
    });
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Whether `actual` satisfies a `required` minimum (actual >= required).
pub fn satisfies(required: &str, actual: &str) -> bool {
    parse(actual) >= parse(required)
}

/// The refusal reason for a plugin that requires a NEWER hub, or `None` when it is
/// acceptable (no requirement, or the hub is new enough). The message names both
/// versions so the reason is self-explanatory (ADR-0002/0008).
pub fn refusal(min_hub_version: Option<&str>) -> Option<String> {
    let required = min_hub_version?;
    if satisfies(required, HUB_VERSION) {
        None
    } else {
        Some(format!(
            "needs hub >= {required}, this hub is {HUB_VERSION}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotted_versions() {
        assert_eq!(parse("1.2.3"), (1, 2, 3));
        assert_eq!(parse("v1.2"), (1, 2, 0));
        assert_eq!(parse("1.2.3-beta"), (1, 2, 3));
        assert_eq!(parse("garbage"), (0, 0, 0));
    }

    #[test]
    fn compares_by_segments() {
        assert!(satisfies("0.1.0", "0.1.0"));
        assert!(satisfies("0.1.0", "0.2.0"));
        assert!(!satisfies("0.2.0", "0.1.0"));
        assert!(satisfies("1.0.0", "1.0.1"));
        assert!(!satisfies("1.1.0", "1.0.9"));
    }

    #[test]
    fn a_newer_requirement_is_refused_with_a_named_reason() {
        assert!(refusal(None).is_none());
        assert!(refusal(Some("0.1.0")).is_none());
        let r = refusal(Some("9.9.9")).expect("refused");
        assert!(r.contains("9.9.9") && r.contains(HUB_VERSION), "{r}");
    }
}
