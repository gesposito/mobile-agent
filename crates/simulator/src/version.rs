//! Dotted numeric versions, such as CoreSimulator's "1171.7", a runtime's
//! "26.5" or Xcode's "27.0", compared numerically component by component.

use std::cmp::Ordering;
use std::fmt;

/// A version whose missing trailing components count as zero, so "26.5"
/// equals "26.5.0" and "1155.10" is above "1155.4".
#[derive(Clone, Debug)]
pub struct Version(Vec<u64>);

impl Version {
    /// Parses digits separated by dots; `None` for anything else.
    pub fn parse(text: &str) -> Option<Version> {
        text.split('.')
            .map(|part| {
                if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                part.parse::<u64>().ok()
            })
            .collect::<Option<Vec<u64>>>()
            .map(Version)
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Version) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Version) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Version) -> Ordering {
        let length = self.0.len().max(other.0.len());
        let component =
            |version: &Version, index: usize| version.0.get(index).copied().unwrap_or(0);
        (0..length)
            .map(|index| component(self, index).cmp(&component(other, index)))
            .find(|ordering| ordering.is_ne())
            .unwrap_or(Ordering::Equal)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for component in &self.0 {
            if !first {
                f.write_str(".")?;
            }
            write!(f, "{component}")?;
            first = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap_or_else(|| panic!("{text:?} parses"))
    }

    #[test]
    fn components_compare_numerically() {
        assert!(
            v("1155.10") > v("1155.4"),
            "10 is above 4, unlike the strings"
        );
        assert!(v("1171.7") > v("1155.4"));
        assert!(v("1155.3") < v("1155.4"));
        assert!(v("27.0") > v("26.5"));
        assert!(v("1200") > v("1155.4"));
    }

    #[test]
    fn missing_components_count_as_zero() {
        assert_eq!(v("26.5"), v("26.5.0"));
        assert_eq!(v("27"), v("27.0"));
        assert!(v("27.0.1") > v("27"));
    }

    #[test]
    fn anything_but_dotted_digits_is_refused() {
        for text in ["", "27.", ".5", "27..1", "27.0b", "iOS 26.5", "-1"] {
            assert_eq!(Version::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn display_round_trips() {
        assert_eq!(v("1171.7").to_string(), "1171.7");
        assert_eq!(v("26.5.0").to_string(), "26.5.0");
    }
}
