//! `ferrule gate`: approvals, and the step after an agent reports.

use std::io::{self, Write};

use clap::Subcommand;

use super::run::{self, ApproveArgs, NextAt};

/// Shown after `ferrule gate approve --help`.
const APPROVE_AFTER_LONG_HELP: &str = "\
Output:
  approve=done gate=<gate> [issue=<n>] [worktree=<path>] [stages=<n>] [<record>]
      The approval is recorded. `<record>` is what the last stop showed:
      `spec-rev=<n>`, `arch-rev=<n>` and `decisions-seen=<lines>`, also logged
      on the card. `issue=`, `worktree=` and `stages=` appear for gate2 only.
Refusals:
  approve=refused reason=card      No such run; check the slug with
                                   `ferrule board list`
  approve=refused reason=no-plan   gate2: plan.md is missing from the run dir
  approve=refused reason=no-stages gate2: plan.md has no `## Shared` stage table
  approve=refused reason=usage     gate2: --issue is neither `new` nor a number
  approve=refused reason=worktree detail=\"<git's error>\"
                                   gate2: `git worktree add` failed
  approve=refused reason=move      gate2: the parent card cannot move to
                                   inprogress
  approve=refused reason=no-issue|no-branch
                                   gate2: filing the issue failed
  approve=refused reason=stage-ref stage-ref=<token>
                                   gate2: the issue body names a stage or an
                                   agent (exit 3); a host error goes to
                                   stderr, exit 1
The approve=refused lines end in ` hint=<fix>`.
Exit codes: 0 done, 1 refused, 2 usage, 4 internal error.
See: ferrule guide show approval";

/// Shown after `ferrule gate next --help`.
const NEXT_AFTER_LONG_HELP: &str = "\
Statuses each position takes:
  gate2    ready, findings, clean, question, arch-gap, split, behavior-change,
           new, reuse (with issue=<n>)
  replan   ready, findings, clean, question, arch-gap, split, behavior-change
  gate3    findings (with count=<n>), clean
  gate4    ready, question
  ci       ci=<exit code of gh pr checks>

Output, in order:
  logged: <event>                  An event logged on the card:
                                   `plan-review=findings count=<n> round=<r>`,
                                   `plan-review=clean round=<r>`,
                                   `review=<n>-open scope=branch`,
                                   `review=<n>-resolved`, `review=clean`,
                                   `replan=pre-authorized` or `ci=<code>`
  next=<word> [reason=<why>]       The one next action:
    relay            question: relay the question file to the user
    arch-gap         spawn the architect, then resume the planner
    split            handle the split request
    behavior-change  back to the spec author
    review           spawn the plan reviewer
    follow-up        spawn the plan reviewer again after findings
    revise           plan review found issues (rounds 1 and 2): resume the
                     planner with the review file
    issue            plan review is done (clean, or findings in round 3): ask
                     the spec author for the tracking issue
    stop             show the stop and wait for the user's approval
    notice           show the notice, then go on as if approved
    fix              gate3: findings are open; ci: CI is red
    stage-stop       gate3 clean with a stage not done: decide its stop
    pr               gate3 clean on every stage: spawn the spec author for
                     the PR body
    done             ci=0: clean up the run
  reason=<why>                     Why `stop` stops, or `pr` has `needs-user`.
                                   gate2 stops for: lane (not fast),
                                   no-arch-stop, needs-user, outgrown (plan
                                   over two stages) and reuse. A re-plan stops
                                   for: not-gate4, needs-user, arch-moved and
                                   outgrown; otherwise it logs
                                   replan=pre-authorized and gives `notice`.
  run: <command>                   A command to run now, slug filled in
  do: <what>                       What to do next (spawn, resume, show)
  guide: ferrule guide show <topic> --slug <slug>
                                   The topic to read before acting
Errors:
  run-next=refused reason=card     No such run (exit 1); check the slug with
                                   `ferrule board list`
  run-next=refused reason=unknown at=<pos> status=<word>
                                   The position does not take that status
                                   (exit 2); the list above has the valid ones
  usage: ferrule gate next <slug> <pos> status=<word>  (stderr, exit 2)
                                   The status line lacks status= (ci=<code>
                                   at ci)
The run-next lines end in ` hint=<fix>`.
Exit codes: 0 ok, 1 refused, 2 usage, 4 internal error.

Example:
  ferrule gate next <slug> gate2 status=ready file=plan.md";

#[derive(Debug, Subcommand)]
pub enum GateCommand {
    /// Record an approved gate.
    ///
    /// `gate2` also files the issue, adds the worktree, moves the parent
    /// card to inprogress and creates the stage cards.
    #[command(after_long_help = APPROVE_AFTER_LONG_HELP)]
    Approve(ApproveArgs),
    /// Log an agent's outcome and print the next step.
    ///
    /// Prints `next=`, then `run:` and `do:` lines and a `guide:` line
    /// naming the topic to read next.
    ///
    /// Reads `status=`, `count=`, `issue=` and `ci=` from the status line and
    /// ignores the rest. Prints `run-next=unknown` (exit 2) for a status the
    /// position does not take.
    #[command(after_long_help = NEXT_AFTER_LONG_HELP)]
    Next {
        /// The run's slug.
        slug: String,
        /// Where the run is: the step whose agent wrote the status line.
        #[arg(value_enum)]
        position: NextAt,
        /// The agent's status line, as one argument or several.
        #[arg(
            required = true,
            allow_hyphen_values = true,
            value_name = "STATUS_LINE"
        )]
        status: Vec<String>,
    },
}

pub fn run(command: &GateCommand, out: &mut impl Write) -> io::Result<u8> {
    match command {
        GateCommand::Approve(args) => run::approve(args, out),
        GateCommand::Next {
            slug,
            position,
            status,
        } => run::next_step(slug, *position, status, out),
    }
}
