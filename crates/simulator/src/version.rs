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
        let _ = text;
        unimplemented!("task 6.1: version parsing")
    }

    /// The components, most significant first.
    pub fn components(&self) -> &[u64] {
        &self.0
    }

    /// The first component, such as Xcode's major version.
    pub fn major(&self) -> u64 {
        self.0.first().copied().unwrap_or(0)
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
        let _ = other;
        unimplemented!("task 6.1: version comparison")
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
    fn display_round_trips_and_major_is_the_first_component() {
        assert_eq!(v("1171.7").to_string(), "1171.7");
        assert_eq!(v("27.0").major(), 27);
        assert_eq!(v("26.5").components(), &[26, 5]);
    }
}
