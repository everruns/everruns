"""Hooks: what a verifier adds to the generic engine.

The engine knows points, levels, certainty, freshness, gates, journals and
the climb list, and nothing about what is being verified. Everything
specific (its checks, names its targets may use, extra exclusion rules, run
imports, report sections) comes from the verifier's own verifier.py, which
defines VERIFIER, an instance of a Hooks subclass. A verifier without one
gets these neutral defaults, enough when its checks need no extra logic.

Decision (owner, 2026-11-24): lib stays agnostic of what is verified, and
there is no shared layer between verifiers; two verifiers about the same
system each keep their own copy of their checks rather than share one.
"""


class Hooks:
    size_unit = None            # what milestones count ("agents"); None: no milestones
    dirty_paths = (".",)        # paths whose uncommitted changes mark a run dirty
    default_levels = None       # levels a plain `verify` runs; None: every unpaid level

    # Goal and targets
    def derived(self, cfg, size):
        """Extra names for target expressions, as g.*."""
        return {}

    def default_size(self, cfg):
        return None

    def milestone_sizes(self, cfg):
        return []

    # Checks
    def checks(self):
        return []

    def not_built(self):
        """Points no check covers yet: {point: (level, what would cover it)}."""
        return {}

    def context(self, cfg, args):
        """Extra attributes on the context every check receives."""
        return {}

    def add_args(self, ap):
        """Verifier command-line options."""

    # Runs and scoring
    def stamp(self, outputs, run):
        """Copy verifier-specific fields from check outputs onto the run and its entries."""

    def exclusion(self, cfg, run, point, entry, size):
        """Why this entry earns nothing under the verifier's rules, or None."""
        return None

    def skip_validity(self, cfg, run, point):
        """True when a validity point does not apply to this run."""
        return False

    def command(self, cfg, args, env):
        """Handle a verifier command (a paid dispatch, a run import). Return
        None if args hold none; an int exit code when done; or a run dict to
        score and report. env: runs, evidence, started, make_run."""
        return None

    # Report
    def summary(self, cfg, runs, fresh):
        """Extra fields for report.json."""
        return {}

    def record_fields(self):
        """Fields of the report kept in a recorded run's score line."""
        return ()

    def headline(self, cfg, rep):
        """Text after the score in the report's first line."""
        return ""

    def sections(self, cfg, rep):
        """Markdown lines after the headline (before milestones)."""
        return []

    def after_milestones(self, cfg, rep):
        """Markdown lines after milestones (before the climb list)."""
        return []

    def notes(self, cfg, rep):
        """Lines for the inputs and notes section."""
        return []

    def history_columns(self):
        """(title, field of the recorded score line) for --history."""
        return []


BASE = Hooks()
