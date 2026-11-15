# Agent Guide

This file holds instructions for coding agents working in this repository.

<!-- devloop:design-judgment v2 sha:6de8e0eb -->
## Design Judgment

**Default to the minimal necessary complexity.** Prefer the smallest design that satisfies the current product need, fits the existing architecture, and leaves a clear path to extend later. Be suspicious of abstractions, invariants, retries, background machinery, or schema constraints that mostly exist to make the design feel complete rather than to solve a present problem.

Interpret "smallest design" architecturally, not as the fewest changed lines. First identify the current product need and any invariant it actually requires; then choose the smallest design that satisfies those, fits the existing architecture, and does not move away from a recorded architectural direction. A locally small patch that weakens the required invariant, creates a parallel path, or adds a temporary abstraction already known to be replaced is not minimal. If the stronger invariant is not a present product need, prefer explicitly tolerating and deferring the problem over building machinery for completeness or shipping a partial solution with a stronger claim.

Operator direction sets the product constraints, but it does not remove the agent's obligation to exercise engineering judgment. Surface and argue when a requested direction would conflict with the existing architecture or a recorded direction, or would spend durable complexity on a low-value problem; once the operator decides with that trade-off visible, follow the decision.

**Understand the request in context.** Respect clear limits the operator sets on files or layers. If the request leaves that choice open, make only the changes needed to achieve the requested outcome. A request to change what is displayed does not by itself change what must be stored or who may access it. Preserve those contracts unless the request or an existing requirement calls for changing them. If different interpretations would materially change the scope, clarify before proceeding. A chosen option means its literal text, not a reviewer's longer recommendation for the same finding. Anything else you believe the work needs goes in the handoff as a suggestion, not into the change.

**Build the smallest increment that delivers working behavior.** Add schemas, contracts, tooling, or design decisions when the current increment needs them. Define contracts first when coordination or compatibility requires it; generate them from code when code is the source of truth. Say what shipped work already meets before proposing more, and name the job each proposed step serves. A step is not built because a list contains it.

**The operator's rulings are final; your own proposals are not.** When the operator rules something tolerated, deferred, or not a blocker, record it once and move on: do not re-raise it, ask for confirmation, or let a reviewer reopen it. Answer the operator's question directly, without turning it into a broader proposal or an edit. Base claims about the operator's experience on their words or session records. Guidance alone does not establish what happened. Follow the operator's instruction to proceed, including any refinements. A request to revise or discuss a proposal does not by itself authorize implementation. Your own announcements, plan steps, scope lines, and deferrals are current scoping, not mandates and not architecture. When your best reading of an instruction would reverse a recorded decision, ask one question naming both readings before touching files.

**Argue with numbers, and audit your own counter-proposal.** When pushing back on a proposed design, the minimality principles above are priors to test, not verdicts to cite: quantify the binding constraint before judging a design over- or under-built. Then red-team your alternative as hard as the proposal, against the same conditions. A position that needs a new compensating patch each time it is challenged is the escalating-commitment signal from Debugging Discipline at design scope — and it applies to both sides of the discussion. When it is your position, stop defending it and re-derive from scratch; when it is the operator's, your job is to help them break the loop with evidence, not to concede to authority. If neither design survives and the discussion stalls, drop both: enumerate the actual requirements together and derive a solution from those.

**Weigh every edge case before fixing it.** Consider these factors together:

- **Likelihood** — how often does this happen in the current product shape?
- **Impact** — what actually happens to the user or operator?
- **Recoverability** — can they recover with existing UI or an obvious manual action?
- **Cost** — how much implementation, testing, documentation, and future constraint does the fix add?

Tolerate or defer a case when the four factors together show that the fix costs more than the failure and required invariants remain satisfied. Low likelihood alone cannot justify deferring a high-impact, unrecoverable failure. Don't introduce durable complexity to make rare behavior tidy. The same weighing applies to a reviewer's finding: one that fails it is declined with a brief reason, not fixed and not escalated. Track a deferred case when later action is needed. Explain deliberate inconsistencies at the site when they would otherwise mislead a maintainer. A deferral that forces an interim mechanism is a build decision: price the interim and its later removal before recommending it. Call out each trade-off briefly in the handoff, so the operator can move the bar.

**Prefer deriving values from authoritative inputs over maintaining additional mutable copies.** Keep domain calculations independent of caching and scheduling details; justify any required coupling.

**Fail loud > self-heal.** Prefer a single attempt that exposes failure and gives the operator a useful recovery hint. Don't add retry loops, sequencing tricks, or duplicate-execution guards merely to make the operation appear seamless. Before retrying a state-changing command, check whether the previous attempt took effect and whether repeating it is safe.

**Implementation complexity must earn its place.** Add a status value, index, retry, rollback, or layer only when a _present_ feature needs it. The cost of a wrong abstraction is paid by every later reader; the cost of a right abstraction deferred by one week is usually nothing.
<!-- /devloop:design-judgment -->

<!-- devloop:debugging-discipline v3 sha:c88ccfe2 -->
## Debugging Discipline

**A failed fix is evidence your mental model is incomplete, not yet evidence the design is wrong.** Retry smaller and more carefully. The failure mode to watch for is **escalating commitment under uncertainty**: repeated failed guesses can make a rewrite feel justified, when the fix usually exists at a smaller scope you have not located yet.

- **When a dependency behaves unexpectedly, read its source before changing your approach.** Find the code that handles the behavior instead of guessing from symptoms alone. A configuration change can be the right fix, but understand why it works and verify that it preserves the behavior you need.
- **Enumerate every input that could produce the symptom before concluding any one is _the_ cause.** Stopping one doesn't stop the others. For event-driven code this means listing the full family that can fire, not the first one that looks plausible.
- **Symptom-based theories are hypotheses, not conclusions.** Before editing, name what you think is happening and what would falsify it. If the fix fails, treat the hypothesis as incomplete or unproven — narrow with a log, source-reading, or a smaller repro before editing again.
- **"Simple" or "minimal" means the smallest sufficient fix with the feature's requirements held constant.** Replacing a broken inline editor with a cruder prompt removes the interaction instead of fixing it. Starting over or switching libraries does not establish that you understood or fixed the cause. Redesign when evidence says the design is wrong, not merely because repeated attempts failed.
<!-- /devloop:debugging-discipline -->

<!-- devloop:test-design v2 sha:b716b670 -->
## Test Design

- **TDD for behavior changes.** Smallest meaningful failing test first; tests are part of the design. A regression guard is seen failing against the pre-fix code before the fix is committed; a guard that cannot fail is false confidence.
- **Test at the lowest useful level** with the real contract for that surface: state transitions for domain logic, request/response for HTTP handlers, auth boundaries for identity, user-visible state changes for UI. Cover the unhappy paths the change makes reachable (authorization failures, stale versions, malformed input, expiry, retries, idempotency, boundaries).
- **Size verification to the change.** Test the behavior the change adds or alters, using the project's existing test approach. Run the repository's checks and add focused checks for relevant risks they do not cover. If an additional check would take substantial effort or need ongoing maintenance, explain its benefit and cost and get the operator's agreement first. Reassess the total cost as checks grow or repeat. Get agreement before turning a one-off investigation into a requirement for later work. A verification result expires when its surface changes: re-run the check that covers a surface after the last edit to it.
- **Avoid over-stubbing.** A test that passes while real integration fails is worse than no test. When a unit test needs heavy mocking, prefer a narrower pure-helper test, a handler-level test with real local collaborators, or a smoke check.
- **Skipping tests is an explicit engineering call** — justify it with the reason, what verification you ran instead, and what test to add if the surface grows.
<!-- /devloop:test-design -->

<!-- devloop:writing-style v6 sha:78db1613 -->
## Writing Style

### Comments

- **Default to no comment.** Only when the _why_ is non-obvious — a hidden constraint, subtle invariant, workaround for a specific bug, behavior that would surprise a reader.
- Don't restate _what_ the code does; identifiers do that.
- Don't reference the current task, fix, or callers (`// used by X`, `// added for the Y flow`, `// handles the case from issue #N`). That belongs in the PR description and rots.
- **One comment, one decision.** Explain the non-obvious reason for the code beside it. Let that explanation wrap naturally. Split independent explanations and place each beside the code it explains. Move broader background to the relevant guide; retain the local context needed to change the code safely.
- **No banner or decorative blocks.** Avoid comments that serve only as visual headings or repeat the name of the code below them.
- Keep public API documentation where developers using the API will find it, including directly above the function or type definition when appropriate.

### Documentation prose

Write docs, issue bodies, and PR/issue comments for both humans and agents. An agent's ability to infer the meaning does not establish that a human can follow it.

- Short sentences, one idea each. Plain wording is not weaker wording: keep the exact claims, names, and invariants, and drop only the compression.
- Don't stack qualifications into em-dash chains or nested parentheticals. Give each qualification its own sentence, or promote the set to a list.
- When prose enumerates cases, obligations, or layers, use a list. A reader should check items off, not parse them out of a paragraph.
- Status headers state a document's current standing only. Revision narration ("revised after review pass N…") is churn; git history already carries it.
- When the operator calls a passage unclear, lead with the key point and use plain language. Add context, structure, or an example when needed to explain it.
- **Keep documentation proportionate to its lasting value.** Write down current behavior, reusable instructions, and decisions whose reasoning future readers need. Consolidate settled findings into the authoritative explanation. Leave run-by-run narration, intermediate verdicts, repeated verification logs, and superseded approaches in PR discussions or local results. Preserve the evidence and limitations needed to assess a conclusion, with enough context and examples to understand it. Create a separate document only when it serves a clear reader need. Preserve historical records that the project explicitly requires.
- **Maintain references, not investigation diaries.** Before handing off, reread affected documentation as a whole. Update the authoritative explanation in place so readers can understand the current system without reconstructing the investigation. Check document structure as well as sentences: several concise reports can still leave readers with competing answers.
- **Keep the evidence readers need.** Preserve the supporting details someone needs to assess a proposed decision or check whether a statement is correct and applies to their situation.
  - Keep the sample inputs or test cases used to establish a result. Record the exact code or dependency versions tested, since results may change between versions.
  - Explain how measurements were made so readers can reproduce them or judge whether a comparison was fair.
  - State what remains unknown. For example, “Desktop performance was tested; mobile performance remains unmeasured.”
  - Keep useful reasons for decisions, including why an approach was rejected. These help readers understand the choice and its trade-offs.
  - Keep a detail when it helps a reader act correctly or check the conclusion. A minimum version to install stays. A list of failed versions can go when it no longer helps explain or verify the requirement. If those failures still establish the requirement or help diagnose a problem, preserve the evidence.
  - Link to an existing source, such as a test report or measurement record, instead of copying its contents into every document.
- **Respect approval boundaries.** Consolidation must not silently change operator-owned intent or approved decisions. If new evidence challenges either, bring it to the operator before rewriting it.
- **Instruction files are paid for on every run.** Skills, agent roles, and `AGENTS.md` load whenever they apply. Keep them focused on instructions, and put broader investigation and design reasoning in specs or design docs. Keep a reason beside its instruction when a human needs it to understand the meaning or an agent needs it to apply the instruction correctly.

### Code

- Use explicit branches when nested expressions or fallback chains obscure the logic.

### Shell output

- Single space between args; never pad to align value columns. Padded whitespace reads as a typo and costs reader doubt.
<!-- /devloop:writing-style -->

<!-- devloop:commit-conventions v3 sha:85e8d889 -->
## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) for **all** commits, including bot-emitted: `<type>(<scope>): <subject>`.

- `feat(<scope>):` — user-visible new behavior.
- `fix(<scope>):` — bug fix.
- `refactor(<scope>):` — restructure code without changing behavior.
- `test(<scope>):` — add or update tests.
- `perf(<scope>):` — improve performance.
- `chore(<scope>):` — version bumps and tooling maintenance.
- `ci(<scope>):` — only for CI workflow changes.
- `docs(<scope>):` — docs-only.

Scopes are this repository's own top-level units. Use a combined `chore(a,b): …` form only when a change genuinely spans both.

- Summary in imperative mood, lower case, no trailing period.
- Body explains _why_, not _what_. The diff shows what changed.
- One logical change per commit. Avoid unrelated formatting changes. Mechanical changes such as a reformat go in their own commit.
- Never `--amend` unless explicitly asked; create a new commit. Hook failure means the commit didn't happen, so an amend would modify the _previous_ commit and may destroy work.
- Never `--no-verify` to skip hooks. If a commit or hook fails, inspect and report the failure, then fix its cause before retrying.

Committing and pushing:

- Immediately before committing or pushing, check that you are on the intended branch. Review the staged diff before committing, and leave unrelated work untouched.
- Commit in its own step. Confirm it succeeded and contains the intended changes before pushing, cleaning up, or dispatching a review.
- Push after committing on a task branch unless told not to. Never force-push.
- Ask before splitting work into a second branch or PR.
- Use GitHub closing keywords only when the merge should close the issue. Use `see #N` for other references, and check the PR's closing references before merging.
- Avoid exposing private repository details in public issues, PRs, or commit messages. Check repository visibility when uncertain.
<!-- /devloop:commit-conventions -->

<!-- devloop:handoff-contract v2 sha:87478112 -->
## Handoff

Every task ends with a handoff. Reporting what changed is the easy half; the obligations below are the half that gets dropped, and each one exists because the operator cannot recover it from the diff.

- **What changed, and what you verified.** Name the commands you ran and their results. State plainly what you could not verify and why — an unavailable surface, a check that needs a browser, an environment you don't have. A passing type-check is not evidence that a feature works.
- **What is pushed.** Reviewers read the pushed commit, not your working tree, so push before reporting. A push is done when the branch's CI conclusion has been read, not when the push returned.
- **Anomalies you saw.** Investigate anomalies that could affect the change's correctness or safety, with effort proportionate to the risk. Check whether the change caused or worsened an anomaly before calling it unrelated. Explain the evidence. Report unrelated, low-impact observations without making them automatic blockers.
- **Trade-offs you took deliberately.** An edge case you tolerated, a problem you deferred, a stronger invariant you decided the present need didn't require. One line each, as information. Ask the operator only about what blocks the work, so they can move the bar without answering a questionnaire.
- **Tests you skipped.** The reason, the verification you ran instead, and the test to add if the surface grows.
- **Drift you noticed.** Documentation that no longer matches the tree, a convention the code has quietly stopped following, a stale assumption in the task itself. Surface it even when it was not your task to fix.
- **Side effects you caused.** A logic fix that changes an error state, a validation message that now reads badly, a behavior change a reader of the diff would not predict.
- **Changes you did not make.** Files in the worktree you didn't touch, and anything you left alone because it looked user-owned.
- **Commands the operator must run.** Put these commands in a repository script rather than a command block to copy and paste. Pass secrets through environment variables.

State these even when they are unflattering, and especially when nobody asked. A handoff that only reports success transfers no information.
<!-- /devloop:handoff-contract -->

<!-- devloop:pipelines v2 sha:9ffefe80 -->
## Pipelines

Work in this repository is organised into pipelines under `docs/pipelines/`. Each one holds a unit of work end to end: `intent.md` states the problem, `spec.md` records the decisions, `plan.md` tracks the build.

Before implementing anything, read `docs/pipelines/README.md` and the `intent.md` of the pipeline you are working in.

- Do not start work on a pipeline unless its intent is `approved` or `in-progress`. The operator approves intents; approval is the gate, and closed pipelines stay closed.
- Do not edit an `intent.md` to match what was built. It states the problem, and it keeps stating the problem after the solution changes. If the build shows the intent is wrong, stop and bring the evidence to the operator.
- Your goal is the outcome in the intent, not the steps in the plan. Verify against the intent before handing off; the operator confirms `done`.
- Record decisions in the pipeline's `spec.md`. There is no separate decision tree.
- Answer every open question in the intent with a decision that quotes it word for word before handing off. Once quoted, the answered question may be removed from the intent. Never add your own questions to the intent: decide questions about how to do the work, and ask the operator about what the work is.
- Put `Proposed: awaiting operator approval.` under each decision you take, and remove it when the operator approves the decision, including approval given in conversation. Do not reverse or materially change a decision without that line. Bring the evidence to the operator instead. You may depart from a spec's preference by recording a decision that says why.
- Before implementing, list the binding decisions from other pipelines with the command in the README's spec section, and read each listed decision. They constrain your work even though another pipeline made them. Only an approved `Overturns:` decision lifts one; follow a proposed binding as if it were approved.
- Mark a decision `Binding:` only when you can name the later pipeline that could violate it without noticing.
- The plan is yours. Revise it whenever the build shows it is wrong, keep `plan.md` current as you build, and mark what was verified by which command separately from what is merely asserted.
<!-- /devloop:pipelines -->
