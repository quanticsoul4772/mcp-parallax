//! Measurement for Jev (064): confusion counts, interval bounds, threshold
//! choice, splits, bootstrap and the promotion rule (plan §7). Pure, std only.

use crate::jev::policy::Band;

/// What Jev decided for one item. Excluded, missing and middle-band items
/// are undecided: they go to the big model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The yes band.
    Yes,
    /// The no band (for a skip gate: no harm, skip the big model).
    No,
    /// Middle band, missing, rejected or excluded.
    Undecided,
}

impl From<Band> for Decision {
    fn from(band: Band) -> Self {
        match band {
            Band::Yes => Self::Yes,
            Band::No => Self::No,
            Band::Middle | Band::Missing => Self::Undecided,
        }
    }
}

impl From<Option<Band>> for Decision {
    fn from(band: Option<Band>) -> Self {
        band.map_or(Self::Undecided, Self::from)
    }
}

/// Counts of Jev decisions against labels (label `true` = positive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Confusion {
    /// Yes on a positive.
    pub tp: u32,
    /// Yes on a negative.
    pub fp: u32,
    /// No on a negative.
    pub tn: u32,
    /// No on a positive.
    pub fn_: u32,
    /// Undecided on a positive.
    pub undecided_positive: u32,
    /// Undecided on a negative.
    pub undecided_negative: u32,
}

fn ratio(numerator: u32, denominator: u32) -> Option<f64> {
    (denominator > 0).then(|| f64::from(numerator) / f64::from(denominator))
}

impl Confusion {
    /// Count `(decision, label)` pairs.
    #[must_use]
    pub fn from_pairs(pairs: &[(Decision, bool)]) -> Self {
        let mut c = Self::default();
        for (decision, positive) in pairs {
            match (decision, positive) {
                (Decision::Yes, true) => c.tp += 1,
                (Decision::Yes, false) => c.fp += 1,
                (Decision::No, false) => c.tn += 1,
                (Decision::No, true) => c.fn_ += 1,
                (Decision::Undecided, true) => c.undecided_positive += 1,
                (Decision::Undecided, false) => c.undecided_negative += 1,
            }
        }
        c
    }

    /// The same decision for every label (the always-yes / always-no
    /// baselines).
    #[must_use]
    pub fn always(decision: Decision, labels: &[bool]) -> Self {
        let pairs: Vec<(Decision, bool)> = labels.iter().map(|l| (decision, *l)).collect();
        Self::from_pairs(&pairs)
    }

    /// tp / (tp + fp); `None` when nothing was a yes.
    #[must_use]
    pub fn precision(&self) -> Option<f64> {
        ratio(self.tp, self.tp + self.fp)
    }

    /// tp / (tp + fn) over decided positives; `None` when there are none.
    #[must_use]
    pub fn recall(&self) -> Option<f64> {
        ratio(self.tp, self.tp + self.fn_)
    }

    /// Every positive, decided or not.
    #[must_use]
    pub const fn positives(&self) -> u32 {
        self.tp + self.fn_ + self.undecided_positive
    }

    /// Items Jev would skip (the no band).
    #[must_use]
    pub const fn skips(&self) -> u32 {
        self.tn + self.fn_
    }
}

/// Skip-gate recall over harmful positives: only a positive Jev would skip is
/// a miss; undecided ones go to the big model. `None` without positives.
#[must_use]
pub fn skip_recall(c: &Confusion) -> Option<f64> {
    ratio(c.positives() - c.fn_, c.positives())
}

/// Wilson score interval for `successes` of `n` at normal quantile `z`.
#[must_use]
pub fn wilson(successes: u32, n: u32, z: f64) -> Option<(f64, f64)> {
    if n == 0 || successes > n {
        return None;
    }
    let n_f = f64::from(n);
    let p = f64::from(successes) / n_f;
    let z2 = z * z;
    let denominator = 1.0 + z2 / n_f;
    let center = (p + z2 / (2.0 * n_f)) / denominator;
    let half = z * (p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)).sqrt() / denominator;
    Some(((center - half).max(0.0), (center + half).min(1.0)))
}

/// ln(n!) by summation (n here is a sample size, at most a few thousand).
fn ln_factorial(n: u32) -> f64 {
    (2..=n).map(|i| f64::from(i).ln()).sum()
}

/// P(X >= k) for X ~ Binomial(n, p), summed in log space.
fn upper_tail(k: u32, n: u32, p: f64) -> f64 {
    if k == 0 {
        return 1.0;
    }
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let ln_n = ln_factorial(n);
    (k..=n)
        .map(|i| {
            let ln_choose = ln_n - ln_factorial(i) - ln_factorial(n - i);
            f64::mul_add(
                f64::from(n - i),
                (1.0 - p).ln(),
                f64::mul_add(f64::from(i), p.ln(), ln_choose),
            )
            .exp()
        })
        .sum()
}

/// The p in [0, 1] where the monotone `f(p)` crosses `target`, by bisection.
fn bisect(increasing: bool, target: f64, f: impl Fn(f64) -> f64) -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..100 {
        let mid = f64::midpoint(lo, hi);
        let above = f(mid) > target;
        if above == increasing {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    f64::midpoint(lo, hi)
}

/// One-sided exact (Clopper-Pearson) lower bound at level `alpha`.
#[must_use]
pub fn clopper_pearson_lower(successes: u32, n: u32, alpha: f64) -> Option<f64> {
    if n == 0 || successes > n {
        return None;
    }
    if successes == 0 {
        return Some(0.0);
    }
    Some(bisect(true, alpha, |p| upper_tail(successes, n, p)))
}

/// Two-sided exact (Clopper-Pearson) interval at level `alpha`.
#[must_use]
pub fn clopper_pearson(successes: u32, n: u32, alpha: f64) -> Option<(f64, f64)> {
    let lower = clopper_pearson_lower(successes, n, alpha / 2.0)?;
    let upper = if successes == n {
        1.0
    } else {
        // P(X <= k) = 1 - P(X >= k + 1), decreasing in p.
        bisect(false, alpha / 2.0, |p| {
            1.0 - upper_tail(successes + 1, n, p)
        })
    };
    Some((lower, upper))
}

/// The distinct observed scores, in hundredths.
#[must_use]
pub fn threshold_grid(scores: &[f64]) -> Vec<u32> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped to 0..=100
    let mut grid: Vec<u32> = scores
        .iter()
        .filter(|s| s.is_finite())
        .map(|s| (s * 100.0).round().clamp(0.0, 100.0) as u32)
        .collect();
    grid.sort_unstable();
    grid.dedup();
    grid
}

/// The skip threshold, by the rule stated in advance (plan 5.4).
///
/// Skip when P(safe) >= t, and take the smallest t on the grid whose
/// one-sided recall lower bound over harmful items meets `target`. `None`
/// when no t does, or there are no harmful items.
#[must_use]
pub fn choose_skip_threshold(scored: &[(f64, bool)], target: f64, alpha: f64) -> Option<f64> {
    let positives = u32::try_from(scored.iter().filter(|(_, harmful)| *harmful).count()).ok()?;
    if positives == 0 {
        return None;
    }
    let observed: Vec<f64> = scored.iter().map(|(p, _)| *p).collect();
    threshold_grid(&observed).into_iter().find_map(|grid| {
        let t = f64::from(grid) / 100.0;
        let misses = scored
            .iter()
            .filter(|(p, harmful)| *harmful && *p >= t)
            .count();
        let misses = u32::try_from(misses).ok()?;
        let bound = clopper_pearson_lower(positives - misses, positives, alpha)?;
        (bound >= target).then_some(t)
    })
}

/// Which set an item belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// Used to choose thresholds.
    Tune,
    /// Sealed; looked at once per question version.
    Confirm,
}

/// A deterministic split by item id: the same id always lands in the same set.
#[must_use]
pub fn split(id: &str, confirm_percent: u64) -> Split {
    if crate::checkpoint::fnv1a64(id.as_bytes()) % 100 < confirm_percent {
        Split::Confirm
    } else {
        Split::Tune
    }
}

/// One look at the confirm set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmLook {
    /// The question looked at.
    pub question: String,
    /// Its wording version.
    pub version: u32,
    /// Why it was looked at.
    pub reason: String,
}

/// Every look at the confirm set, so repeated looks are visible.
#[derive(Debug, Clone, Default)]
pub struct ConfirmLog {
    looks: Vec<ConfirmLook>,
}

impl ConfirmLog {
    /// Record a look.
    pub fn look(&mut self, question: &str, version: u32, reason: &str) {
        self.looks.push(ConfirmLook {
            question: question.into(),
            version,
            reason: reason.into(),
        });
    }

    /// All looks, in order.
    #[must_use]
    pub fn looks(&self) -> &[ConfirmLook] {
        &self.looks
    }

    /// How many looks one question version has had.
    #[must_use]
    pub fn looks_for(&self, question: &str, version: u32) -> usize {
        self.looks
            .iter()
            .filter(|l| l.question == question && l.version == version)
            .count()
    }
}

/// xorshift64*: a small fixed-seed generator, so bootstraps repeat exactly.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Percentile bootstrap (2.5 / 97.5) of `stat`, resampling whole clusters
/// with a fixed seed. `None` without clusters or without any defined value.
pub fn cluster_bootstrap(
    clusters: &[Vec<(Decision, bool)>],
    stat: impl Fn(&Confusion) -> Option<f64>,
    iterations: usize,
    seed: u64,
) -> Option<(f64, f64)> {
    if clusters.is_empty() {
        return None;
    }
    let mut rng = Rng(if seed == 0 {
        0x9e37_79b9_7f4a_7c15
    } else {
        seed
    });
    let len = clusters.len() as u64;
    let mut values: Vec<f64> = (0..iterations)
        .filter_map(|_| {
            let mut pairs = Vec::new();
            for _ in 0..clusters.len() {
                #[allow(clippy::cast_possible_truncation)] // index < clusters.len()
                let pick = (rng.next() % len) as usize;
                pairs.extend_from_slice(&clusters[pick]);
            }
            stat(&Confusion::from_pairs(&pairs))
        })
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )] // indices within values.len()
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    Some((at(0.025), at(0.975)))
}

/// The confirm-set evidence for one skip-gate question.
#[derive(Debug, Clone, Copy)]
pub struct PromotionInput {
    /// Decisions against labels on the confirm set.
    pub confusion: Confusion,
    /// Clusters (runs or sessions) the items came from.
    pub clusters: usize,
    /// The recall target.
    pub target_recall: f64,
    /// One-sided level for the recall bound.
    pub alpha: f64,
    /// Harmful positives needed (59 for 95% with no misses).
    pub min_positives: u32,
}

/// Whether a question may leave shadow, and every reason it may not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Promotion {
    /// All criteria passed.
    pub promote: bool,
    /// One line per failed criterion.
    pub reasons: Vec<String>,
}

/// Plan 7 for a skip gate: enough positives, the recall lower bound meets
/// the target, it skips something (beats the never-skip baseline), and the
/// clusters are reported.
#[must_use]
pub fn promotion_decision(input: &PromotionInput) -> Promotion {
    let c = &input.confusion;
    let mut reasons = Vec::new();
    if c.positives() < input.min_positives {
        reasons.push(format!(
            "{} harmful positives; at least {} needed",
            c.positives(),
            input.min_positives
        ));
    }
    let bound = clopper_pearson_lower(c.positives() - c.fn_, c.positives(), input.alpha);
    match bound {
        Some(b) if b >= input.target_recall => {}
        Some(b) => reasons.push(format!(
            "recall lower bound {b:.4} is below the target {}",
            input.target_recall
        )),
        None => reasons.push("recall lower bound undefined: no harmful positives".into()),
    }
    if c.skips() == 0 {
        reasons.push("nothing skipped: does not beat the never-skip baseline".into());
    }
    if input.clusters == 0 {
        reasons.push("no clusters reported".into());
    }
    Promotion {
        promote: reasons.is_empty(),
        reasons,
    }
}

#[cfg(test)]
mod tests;
