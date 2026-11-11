//! Stage tiers: how a stage is reviewed. The plan proposes one tier per
//! stage, gate 2 settles them, and a skipped tier is refused on a stage
//! that carries a risk.

use crate::run::gate::PlanStage;

/// How a stage is reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// A review of its own.
    Own,
    /// Reviewed with the batch it closes.
    Batched,
    /// No review before gate 3.
    Skipped,
}

impl Tier {
    /// The tier a word names.
    #[must_use]
    pub fn parse(word: &str) -> Option<Tier> {
        match word {
            "own" => Some(Tier::Own),
            "batched" => Some(Tier::Batched),
            "skipped" => Some(Tier::Skipped),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Own => "own",
            Tier::Batched => "batched",
            Tier::Skipped => "skipped",
        }
    }
}

/// What makes a stage unfit to go unreviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    HostWrites,
    PersistedState,
    PublicSurface,
}

impl Risk {
    /// The risk a word names.
    #[must_use]
    pub fn parse(word: &str) -> Option<Risk> {
        match word {
            "host-writes" => Some(Risk::HostWrites),
            "persisted-state" => Some(Risk::PersistedState),
            "public-surface" => Some(Risk::PublicSurface),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::HostWrites => "host-writes",
            Risk::PersistedState => "persisted-state",
            Risk::PublicSurface => "public-surface",
        }
    }
}

/// Why tiers were refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TierRefused {
    /// A skipped stage that carries a risk.
    Risky { stage: u32, risk: Risk },
    /// An override item whose stage id is malformed.
    UnknownStage(String),
    /// An override item whose tier is no tier.
    UnknownTier(String),
}

/// Why a stage got its own review beyond its plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Promotion {
    SpecChange,
    BatchBlocker,
}

/// A stage's planned tier and the one that applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageTier {
    pub planned: Tier,
    pub effective: Tier,
    pub promoted: Option<Promotion>,
}

/// Where a review falls after a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewPoint {
    /// This stage is reviewed alone.
    Stage(u32),
    /// A batch closes here; its stages, oldest first.
    Batch(Vec<u32>),
    /// No review before gate 3.
    None,
}

/// `--tiers <stage>=<tier>,…` as pairs, in order.
///
/// # Errors
///
/// [`TierRefused::UnknownStage`] for an item whose stage is no `s<n>`,
/// [`TierRefused::UnknownTier`] for one whose tier is no tier word.
pub fn parse_overrides(v: &str) -> Result<Vec<(u32, Tier)>, TierRefused> {
    v.split(',')
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .map(|item| {
            let (stage, word) = item
                .split_once('=')
                .ok_or_else(|| TierRefused::UnknownStage(item.to_owned()))?;
            let n = stage
                .trim()
                .strip_prefix('s')
                .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| TierRefused::UnknownStage(item.to_owned()))?;
            let tier = Tier::parse(word.trim())
                .ok_or_else(|| TierRefused::UnknownTier(word.trim().to_owned()))?;
            Ok((n, tier))
        })
        .collect()
}

/// Each stage's tier: the plan's (`own` when it names none), then the
/// overrides in order, the last one winning; an override naming no stage of
/// the plan is ignored.
///
/// # Errors
///
/// [`TierRefused::Risky`] for a stage that ends skipped with a risk.
pub fn settle(stages: &[PlanStage], over: &[(u32, Tier)]) -> Result<Vec<(u32, Tier)>, TierRefused> {
    let mut out = Vec::with_capacity(stages.len());
    for s in stages {
        let tier = over
            .iter()
            .filter(|(n, _)| *n == s.n)
            .map(|(_, t)| *t)
            .next_back()
            .or(s.tier)
            .unwrap_or(Tier::Own);
        if tier == Tier::Skipped
            && let Some(&risk) = s.risk.first()
        {
            return Err(TierRefused::Risky { stage: s.n, risk });
        }
        out.push((s.n, tier));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(n: u32, tier: Option<Tier>, risk: &[Risk]) -> PlanStage {
        PlanStage {
            n,
            title: format!("t{n}"),
            files: Vec::new(),
            blocked_by: Vec::new(),
            tier,
            risk: risk.to_vec(),
        }
    }

    /// GA-R-070 — the words round trip through `parse` and `as_str`.
    #[test]
    fn ut_tier_and_risk_words_round_trip() {
        for t in [Tier::Own, Tier::Batched, Tier::Skipped] {
            assert_eq!(Tier::parse(t.as_str()), Some(t));
        }
        for r in [Risk::HostWrites, Risk::PersistedState, Risk::PublicSurface] {
            assert_eq!(Risk::parse(r.as_str()), Some(r));
        }
        assert_eq!(Tier::parse("never"), None);
        assert_eq!(Risk::parse("none"), None);
    }

    /// GA-R-070 — overrides parse in order; a bad stage or tier is named.
    #[test]
    fn ut_parse_overrides_reads_pairs_and_names_the_bad_item() {
        assert_eq!(
            parse_overrides("s2=batched, s3=skipped,s2=own").unwrap(),
            [(2, Tier::Batched), (3, Tier::Skipped), (2, Tier::Own)]
        );
        assert_eq!(
            parse_overrides("2=own"),
            Err(TierRefused::UnknownStage("2=own".to_owned()))
        );
        assert_eq!(
            parse_overrides("s2"),
            Err(TierRefused::UnknownStage("s2".to_owned()))
        );
        assert_eq!(
            parse_overrides("s2=maybe"),
            Err(TierRefused::UnknownTier("maybe".to_owned()))
        );
    }

    /// GA-R-070, GA-E-006 — the plan's tier, `own` when absent, then the
    /// overrides, the last winning; an override of no stage is ignored.
    #[test]
    fn ut_settle_takes_plan_then_overrides_in_order() {
        let stages = [
            stage(1, None, &[]),
            stage(2, Some(Tier::Batched), &[]),
            stage(3, Some(Tier::Skipped), &[]),
        ];
        let over = [
            (2, Tier::Skipped),
            (2, Tier::Own),
            (3, Tier::Batched),
            (9, Tier::Skipped),
        ];
        assert_eq!(
            settle(&stages, &over).unwrap(),
            [(1, Tier::Own), (2, Tier::Own), (3, Tier::Batched)]
        );
        assert_eq!(
            settle(&stages, &[]).unwrap(),
            [(1, Tier::Own), (2, Tier::Batched), (3, Tier::Skipped)]
        );
    }

    /// GA-R-071 — a skipped stage with a risk is refused, by plan or override;
    /// another tier is not.
    #[test]
    fn ut_settle_refuses_skipped_on_a_risky_stage() {
        let risky = [stage(
            1,
            Some(Tier::Skipped),
            &[Risk::PersistedState, Risk::HostWrites],
        )];
        assert_eq!(
            settle(&risky, &[]),
            Err(TierRefused::Risky {
                stage: 1,
                risk: Risk::PersistedState
            })
        );
        let by_override = [stage(2, Some(Tier::Batched), &[Risk::PublicSurface])];
        assert_eq!(
            settle(&by_override, &[(2, Tier::Skipped)]),
            Err(TierRefused::Risky {
                stage: 2,
                risk: Risk::PublicSurface
            })
        );
        assert_eq!(settle(&by_override, &[]).unwrap(), [(2, Tier::Batched)]);
        assert_eq!(settle(&risky, &[(1, Tier::Own)]).unwrap(), [(1, Tier::Own)]);
    }
}
