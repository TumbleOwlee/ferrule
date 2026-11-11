//! Stage tiers: how a stage is reviewed. The plan proposes one tier per
//! stage, gate 2 settles them, and a skipped tier is refused on a stage
//! that carries a risk.

use std::path::Path;

use rusqlite::Connection;

use crate::card::board;
use crate::card::typed::{Name, Typed};
use crate::config::Config;
use crate::git;
use crate::project::Layout;
use crate::run::Error;
use crate::run::gate::PlanStage;
use crate::store::Board;

/// Stages a batch holds at most.
pub const BATCH_STAGES: usize = 3;
/// Changed lines that close a batch.
pub const BATCH_LINES: usize = 1200;

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
    /// A plan risk word that names no risk.
    UnknownRisk { stage: u32, word: String },
}

/// Why a stage got its own review beyond its plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Promotion {
    SpecChange,
    BatchBlocker,
}

impl Promotion {
    /// The promotion a word names.
    #[must_use]
    pub fn parse(word: &str) -> Option<Promotion> {
        match word {
            "spec-change" => Some(Promotion::SpecChange),
            "batch-blocker" => Some(Promotion::BatchBlocker),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Promotion::SpecChange => "spec-change",
            Promotion::BatchBlocker => "batch-blocker",
        }
    }
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
/// [`TierRefused::Risky`] for a stage that ends skipped with a risk,
/// [`TierRefused::UnknownRisk`] for a risk word that names no risk.
pub fn settle(stages: &[PlanStage], over: &[(u32, Tier)]) -> Result<Vec<(u32, Tier)>, TierRefused> {
    let mut out = Vec::with_capacity(stages.len());
    for s in stages {
        if let Some(word) = s.unknown_risk.first() {
            return Err(TierRefused::UnknownRisk {
                stage: s.n,
                word: word.clone(),
            });
        }
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

/// Decides `stage`'s tier and review point, logs the tier on its card and
/// returns the output line.
///
/// # Errors
///
/// [`Error::Card`] without the stage's card, [`Error::Sql`] when the state
/// database failed.
pub fn decide(
    db: &mut Board,
    layout: &Layout,
    config: &Config,
    slug: &str,
    stage: u32,
    after_blocker: bool,
    now: &str,
) -> Result<String, Error> {
    let conn = db.conn_mut();
    let card = card_id(slug, stage);
    if board::state(conn, &card)?.is_none() {
        return Err(Error::Card);
    }
    let wt = board::worktree(conn, layout, slug, &card)?;
    let st = effective(conn, slug, stage)?;
    let promotion = if after_blocker {
        Some(Promotion::BatchBlocker)
    } else if st.promoted.is_some() {
        st.promoted
    } else if st.planned != Tier::Own
        && spec_changed(
            &wt,
            stage,
            crate::run::stage_base(conn, &wt, slug, stage, &config.main_branch)?.as_deref(),
        )
    {
        Some(Promotion::SpecChange)
    } else {
        None
    };
    let tier = if promotion.is_some() {
        Tier::Own
    } else {
        st.planned
    };
    let same = board::last_event(conn, &card, &[Name::Tier])?.is_some_and(|e| {
        matches!(e.typed(), Typed::Tier { tier: t, promoted }
            if t == tier && promoted == promotion.map(Promotion::as_str))
    });
    if !same {
        let args: Vec<String> = promotion
            .map(|p| format!("promoted={}", p.as_str()))
            .into_iter()
            .collect();
        board::log(conn, &card, &format!("tier={}", tier.as_str()), &args, now).map_err(
            |e| match e {
                board::LogError::Sql(e) => Error::Sql(e),
                // The card exists and the line is built here: neither can fail.
                other @ (board::LogError::Card | board::LogError::Reject(_)) => Error::Sql(
                    rusqlite::Error::ToSqlConversionFailure(other.to_string().into()),
                ),
            },
        )?;
    }
    let mut line = format!("tier={} review=", tier.as_str());
    match review_point(conn, &wt, slug, stage, &config.main_branch)? {
        ReviewPoint::Stage(_) => line.push_str("stage"),
        ReviewPoint::None => line.push_str("none"),
        ReviewPoint::Batch(v) => {
            let ids: Vec<String> = v.iter().map(|n| format!("s{n}")).collect();
            line.push_str("batch stages=");
            line.push_str(&ids.join(","));
        }
    }
    if let Some(p) = promotion {
        line.push_str(" promoted=");
        line.push_str(p.as_str());
    }
    Ok(line)
}

fn card_id(slug: &str, stage: u32) -> String {
    format!("{slug}.s{stage}")
}

/// The tier a stage card's `tier:` field plans, `own` when absent.
fn planned(conn: &Connection, slug: &str, stage: u32) -> rusqlite::Result<Tier> {
    Ok(board::field(conn, &card_id(slug, stage), "tier")?
        .as_deref()
        .and_then(Tier::parse)
        .unwrap_or(Tier::Own))
}

/// A stage's planned tier and the one that applies: a newest `tier=` event
/// that promoted the stage makes it `own`.
///
/// # Errors
///
/// The state database failed.
pub fn effective(conn: &Connection, slug: &str, stage: u32) -> rusqlite::Result<StageTier> {
    let planned = planned(conn, slug, stage)?;
    let promoted = board::last_event(conn, &card_id(slug, stage), &[Name::Tier])?.and_then(|e| {
        match e.typed() {
            Typed::Tier { promoted, .. } => promoted.and_then(Promotion::parse),
            _ => None,
        }
    });
    Ok(StageTier {
        planned,
        effective: if promoted.is_some() {
            Tier::Own
        } else {
            planned
        },
        promoted,
    })
}

/// Whether the worktree changes `docs/specs` since `base` (`HEAD` when
/// `None`), untracked files included; never for stage 0, which lands the
/// brief's spec text.
#[must_use]
pub fn spec_changed(wt: &Path, stage: u32, base: Option<&str>) -> bool {
    if stage == 0 {
        return false;
    }
    let tracked = git::out(
        wt,
        &[
            "diff",
            "--name-only",
            base.unwrap_or("HEAD"),
            "--",
            "docs/specs",
        ],
    )
    .is_some_and(|o| !o.trim().is_empty());
    tracked
        || git::out(wt, &["ls-files", "-z", "--others", "--exclude-standard"])
            .unwrap_or_default()
            .split('\0')
            .any(|p| p.starts_with("docs/specs/"))
}

/// Changed lines between `base` and the commit `to`.
fn committed_lines(wt: &Path, base: &str, to: &str) -> usize {
    git::out(wt, &["diff", "--numstat", base, to])
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let mut cols = l.split('\t');
            let mut n = || {
                cols.next()
                    .and_then(|c| c.parse::<usize>().ok())
                    .unwrap_or(0)
            };
            n() + n()
        })
        .sum()
}

/// Where the review falls after `stage`: walks the run's stages in order,
/// batching consecutive batched ones until a batch closes.
///
/// # Errors
///
/// The state database failed.
pub fn review_point(
    conn: &Connection,
    wt: &Path,
    slug: &str,
    stage: u32,
    main: &str,
) -> rusqlite::Result<ReviewPoint> {
    let cards = board::stages(conn, slug)?;
    let mut open: Vec<u32> = Vec::new();
    let mut open_base: Option<String> = None;
    for (i, (k, id, _)) in cards
        .iter()
        .enumerate()
        .filter(|(_, (k, _, _))| *k <= stage)
    {
        let k = *k;
        let t = effective(conn, slug, k)?;
        match t.effective {
            Tier::Skipped => {
                if k == stage {
                    return Ok(ReviewPoint::None);
                }
            }
            Tier::Own => {
                if k == stage {
                    return Ok(
                        if t.promoted == Some(Promotion::SpecChange) && !open.is_empty() {
                            open.push(k);
                            ReviewPoint::Batch(open)
                        } else {
                            ReviewPoint::Stage(k)
                        },
                    );
                }
                open.clear();
            }
            Tier::Batched => {
                open.push(k);
                if open.len() == 1 {
                    open_base = crate::run::stage_base(conn, wt, slug, open[0], main)?;
                }
                let lines = match (&open_base, k == stage) {
                    (Some(b), true) => crate::run::diff_size(wt, b).map_or(0, |(l, _)| l),
                    (Some(b), false) => board::last_value(conn, id, Name::Commit)?
                        .map_or(0, |c| committed_lines(wt, b, &c)),
                    (None, _) => 0,
                };
                let next = match cards.get(i + 1) {
                    Some((n, _, _)) => planned(conn, slug, *n)? != Tier::Batched,
                    None => true,
                };
                if open.len() >= BATCH_STAGES || lines >= BATCH_LINES || next {
                    if k == stage {
                        return Ok(ReviewPoint::Batch(open));
                    }
                    open.clear();
                }
            }
        }
    }
    Ok(ReviewPoint::None)
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
            unknown_risk: Vec::new(),
        }
    }

    /// The words round trip through `parse` and `as_str`.
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

    /// GA-E-014 — a risk word that names no risk is refused, whatever the tier.
    #[test]
    fn ut_settle_refuses_an_unknown_risk_word() {
        let mut typo = stage(1, Some(Tier::Skipped), &[]);
        typo.unknown_risk = vec!["host-write".to_owned()];
        assert_eq!(
            settle(&[typo], &[]),
            Err(TierRefused::UnknownRisk {
                stage: 1,
                word: "host-write".to_owned()
            })
        );
    }

    /// The promotion words round trip, and a `tier=` event reads its tier and
    /// promotion.
    #[test]
    fn ut_tier_events_read_their_promotion() {
        for p in [Promotion::SpecChange, Promotion::BatchBlocker] {
            assert_eq!(Promotion::parse(p.as_str()), Some(p));
        }
        assert_eq!(Promotion::parse("never"), None);
        let mut s = crate::store::Store::memory().unwrap();
        board::create(
            &mut s.conn,
            "r.s1",
            board::State::Open,
            &[("tier".to_owned(), "batched".to_owned())],
            "t",
        )
        .unwrap();
        let got = effective(&s.conn, "r", 1).unwrap();
        assert_eq!(
            (got.planned, got.effective, got.promoted),
            (Tier::Batched, Tier::Batched, None)
        );
        board::log(
            &mut s.conn,
            "r.s1",
            "tier=own",
            &["promoted=spec-change".to_owned()],
            "t",
        )
        .unwrap();
        let got = effective(&s.conn, "r", 1).unwrap();
        assert_eq!(
            (got.planned, got.effective, got.promoted),
            (Tier::Batched, Tier::Own, Some(Promotion::SpecChange))
        );
    }

    /// GA-E-012 — `spec_changed` sees a tracked or untracked change under
    /// `docs/specs`, never for stage 0.
    #[test]
    fn ut_spec_changed_sees_tracked_and_untracked_specs() {
        const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(wt)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "-q"]);
        // Writes the empty tree the base names.
        git(&["write-tree"]);
        std::fs::write(wt.join("other.md"), "o\n").unwrap();
        git(&["add", "other.md"]);
        assert!(!spec_changed(wt, 1, Some(EMPTY_TREE)));
        std::fs::create_dir_all(wt.join("docs/specs")).unwrap();
        std::fs::write(wt.join("docs/specs/a.md"), "a\n").unwrap();
        assert!(spec_changed(wt, 1, Some(EMPTY_TREE)), "untracked");
        assert!(!spec_changed(wt, 0, Some(EMPTY_TREE)));
        git(&["add", "docs"]);
        assert!(spec_changed(wt, 1, Some(EMPTY_TREE)), "tracked");
    }
}
