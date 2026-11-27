//! Which research claims are never sent to Jev (064): numbers, dates and
//! claims with a source flagged by research's injection screen.

/// Why a research claim is not sent to Jev (064): Jev is weak at numbers
/// and dates, and a flagged source may carry injected instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// A digit (of any script, which covers version strings) or a number word.
    Number,
    /// A month, a weekday or a relative date word.
    Date,
    /// A source flagged by research's injection screen (063).
    FlaggedSource,
}

impl Exclusion {
    /// The recorded reason.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Date => "date",
            Self::FlaggedSource => "flagged_source",
        }
    }
}

pub(crate) const NUMBER_WORDS: [&str; 26] = [
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "hundred",
    "thousand",
    "million",
    "billion",
    "once",
    "twice",
];

/// Months (full and short; "may" and "mar" are left out because "may" is an
/// ordinary verb), weekdays, and single relative date words.
pub(crate) const DATE_WORDS: [&str; 32] = [
    "january",
    "february",
    "march",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "jan",
    "feb",
    "apr",
    "jun",
    "jul",
    "aug",
    "sep",
    "sept",
    "oct",
    "nov",
    "dec",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "today",
    "yesterday",
    "tomorrow",
];

/// Whether a claim is kept away from Jev, and why. Words are split on
/// anything that isn't a letter and compared without case.
#[must_use]
pub fn claim_exclusion(claim: &str, any_source_flagged: bool) -> Option<Exclusion> {
    if any_source_flagged {
        return Some(Exclusion::FlaggedSource);
    }
    if claim.chars().any(char::is_numeric) {
        return Some(Exclusion::Number);
    }
    let words: Vec<String> = claim
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.iter().any(|w| NUMBER_WORDS.contains(&w.as_str())) {
        return Some(Exclusion::Number);
    }
    let relative = words.windows(2).any(|pair| {
        matches!(pair[0].as_str(), "last" | "next")
            && matches!(pair[1].as_str(), "week" | "month" | "year")
    });
    if relative || words.iter().any(|w| DATE_WORDS.contains(&w.as_str())) {
        return Some(Exclusion::Date);
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{claim_exclusion, Exclusion};

    #[test]
    fn claims_with_numbers_dates_or_versions_are_excluded() {
        for claim in [
            "Sales rose in 2018.",
            "Growth was 3.5% overall.",
            "It shipped as v1.14.",
            "The job failed three times.",
            "The launch was in March.",
            "Revenue fell in Q3.",
            "About 1,000 users signed up.",
            "The count was \u{0663}\u{0664} users.",
            "The office closes on Tuesday.",
            "It happened yesterday.",
            "Prices change next month.",
            "It ran twice before stopping.",
            "The release slipped to Sept.",
        ] {
            assert!(claim_exclusion(claim, false).is_some(), "{claim}");
        }
    }

    #[test]
    fn plain_claims_pass() {
        for claim in [
            "Rust has no garbage collector.",
            "The tool may fail on large inputs.",
            "Marching bands are loud.",
            "Someone wrote the parser in Go.",
            "Monads compose effects.",
        ] {
            assert_eq!(claim_exclusion(claim, false), None, "{claim}");
        }
    }

    #[test]
    fn a_claim_with_a_flagged_source_is_excluded() {
        assert_eq!(
            claim_exclusion("Rust has no garbage collector.", true),
            Some(Exclusion::FlaggedSource)
        );
    }

    #[test]
    fn exclusions_name_their_reason() {
        assert_eq!(claim_exclusion("In 2018.", false), Some(Exclusion::Number));
        assert_eq!(
            claim_exclusion("Three of them.", false),
            Some(Exclusion::Number)
        );
        assert_eq!(claim_exclusion("On Friday.", false), Some(Exclusion::Date));
        assert_eq!(Exclusion::Number.as_str(), "number");
        assert_eq!(Exclusion::Date.as_str(), "date");
        assert_eq!(Exclusion::FlaggedSource.as_str(), "flagged_source");
    }
}
