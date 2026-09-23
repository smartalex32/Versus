# Codex Agent Template

A reusable foundation for repository work: scoped agents, common development
skills, and concise project guidance. The template contains configuration and
documentation only.

## Contents

```text
.codex/
  config.toml
  agents/
    default.toml
    explorer.toml
    worker.toml
    tester.toml
    debugger.toml
    reviewer.toml
    architect.toml
    escalation-advisor.toml
    ux-designer.toml
.agents/skills/
  project-onboarding/SKILL.md
  pr-preparation/SKILL.md
AGENTS.md
AGENTS_PROJECT_REFERENCE.md
README.md
.gitignore
```

## Adopt in a project

1. Copy `.codex/`, `.agents/`, and the two `AGENTS*.md` files into the destination
   repository. Merge with existing guidance and configuration instead of replacing
   it blindly; merge useful ignore rules as well.
2. Replace the template-specific [project reference](AGENTS_PROJECT_REFERENCE.md)
   with the destination's architecture, constraints, and exact validation commands.
   Ask Codex to use `$project-onboarding` to establish those facts.
3. Review the models in the config and role files for your account. Keep model and
   reasoning settings paired when changing them. The defaults use
   Terra/Luna/Sol with Astra reserved for exceptional escalations;
   model availability varies by account.
4. Open and trust the destination project in Codex. Project configuration is loaded
   for trusted projects; personal and managed configuration also affect the result.
   See [configuration basics](https://learn.chatgpt.com/docs/config-file/config-basic).
5. Start a new task and verify that its available roles and skills include this
   template. Ask for a small read-only `explorer` subtask to smoke-test delegation.

Keep account credentials, absolute paths, trust entries, plugins, and machine
permissions in personal or managed configuration. Primary session permissions
come from that environment.

## Agent roles

The primary session coordinates and integrates work using **Terra / medium**.
It is configured in `.codex/config.toml`; `default.toml` is the fallback subagent.
Full model IDs are recorded in the TOML files.

| Role | Model / effort | Sandbox default | Assignment and return |
| --- | --- | --- | --- |
| `default` | Luna / low | Workspace write | Straightforward support edits; result and checks |
| `explorer` | Luna / medium | Read only | Trace repository behavior; paths, symbols, evidence |
| `worker` | Terra / medium | Workspace write | Bounded feature or fix; code, tests, validation |
| `tester` | Luna / medium | Workspace write | Execute checks and basic triage; failures and coverage gaps |
| `debugger` | Terra / high | Workspace write | Diagnose difficult failures; evidence, root cause, fix plan |
| `reviewer` | Terra / high | Read only | Assess a diff; actionable findings by severity |
| `architect` | Sol / high | Read only | Resolve a difficult decision; recommendation and plan |
| `escalation-advisor` | Astra / high | Read only | Exceptional technical questions; evidence, decision, implementation plan |
| `ux-designer` | Terra / medium | Read only | Define interface behavior; states and acceptance criteria |

Role files are discovered from `.codex/agents/`; each carries its own name,
description, settings, and instructions. See
[custom agents](https://learn.chatgpt.com/docs/agent-configuration/subagents#custom-agents).

Up to three subagents can be open in addition to the primary. `AGENTS.md` permits
delegation for substantial independent work; simple tasks stay with the primary.
Subagents receive separate assignments and do not create further agents. Sandbox
values are defaults: live parent permission overrides can supersede them. Role
instructions still constrain editing; the tester's write access supports caches
and generated test output, with test edits only when explicitly assigned.

Example assignment: “Use `explorer` to trace the settings save path. Return the
entry point, persistence code, relevant tests, and uncertainties. Make no edits.”

### Quality and budget decisions

These are starting allocations based on role scope, not measured guarantees:

- **Default: Luna / low.** Suitable for mechanical support with explicit acceptance
  criteria. Keep substantive implementation and reasoning with specialists.
- **Explorer: Luna / medium.** Extra reasoning helps connect callers, dependencies,
  and tests while retaining the least expensive model tier. Deep behavioral diagnosis
  belongs with debugger.
- **Worker and UX designer: Terra / medium.** Balanced defaults for bounded engineering
  and interface decisions. Split large assignments into coherent, reviewable parts.
- **Tester: Luna / medium.** Appropriate for documented commands, basic triage, and
  straightforward assigned test edits. Worker owns complex test design and fixtures.
- **Debugger and reviewer: Terra / high.** Spend more reasoning on causal analysis
  and subtle defects, but invoke these roles only when the task justifies them.
- **Architect: Sol / high.** Reserve for hard or high-risk decisions, including
  security boundaries, data-loss risks, and unresolved cross-system failures.
- **Escalation advisor: Astra / high.** Use for exceptional complexity, costly
  correctness risks, or an unresolved problem after focused investigation. Give it
  the specific question, evidence, attempted hypotheses, and success criteria.
  It advises; worker implements and tester validates the resulting plan.

This allocation follows the documented positioning of
[Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna) for cost-sensitive
work, [Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra) for balancing
quality and cost, and [Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol)
for complex professional work. [Astra](https://developers.openai.com/api/docs/models/gpt-6-astra)
adds a tier for the hardest work. API token prices are not estimates of Codex plan usage.

Planning and integration stay with the primary. Documentation is covered by worker
or default, and PR preparation by its skill. CI configuration uses worker; CI failures
use tester or debugger. Routine security checks belong in review; architect handles
specific high-risk design questions. Add dedicated security, performance, database,
or release specialists only when a project's recurring work needs that expertise.

Keep concurrency at three as a ceiling. Assign only useful independent work, pass
concise evidence between roles, and reuse checks when the final code is unchanged.
Skip a cheaper tier when the risk already warrants stronger analysis. If work stalls,
escalate with the attempted hypotheses and evidence rather than rerunning it blindly.
Use Astra directly when exceptional difficulty or the cost of a mistake already
justifies it; trying every cheaper tier is not required. Do not run Sol and Astra
on the same question by default or add Astra as a mandatory final review. Keep its
assignment focused and return routine implementation to the existing roles.
Role TOML model/effort settings take precedence over spawn selections, so choose
the appropriate role instead of assuming a spawn override will upgrade it. See
[subagent configuration](https://learn.chatgpt.com/docs/agent-configuration/subagents#custom-agents).

Judge the allocation over representative real tasks: successful completion,
missed defects, rework, elapsed time, and available usage data. Upgrade a role when
failures recur despite clear scope and sufficient context; avoid blanket increases
to high or maximum reasoning. The template defines no monetary spending cap.

## Workflow skills

Skills describe reusable procedures; agents describe delegated roles. They can be
used independently. Repository skills live under `.agents/skills/` and keep normal
automatic selection enabled.

- [project-onboarding](.agents/skills/project-onboarding/SKILL.md): create or refresh
  project facts and validation commands. Example: “Use `$project-onboarding` to
  adapt the project reference to this repository.”
- [pr-preparation](.agents/skills/pr-preparation/SKILL.md): prepare a title, body,
  and validation summary from the final diff. Example: “Use `$pr-preparation` to
  draft a PR description for this branch against `develop`.”

These skills reflect the repository development workflow; they assume no specific
application stack, hosting service, or external integration.

## Validate the template

From the repository root, parse every TOML file with Python 3.11+:

```sh
python -c "from pathlib import Path; import tomllib; files = sorted(Path('.codex').rglob('*.toml')); [tomllib.loads(p.read_text(encoding='utf-8')) for p in files]; print(f'Parsed {len(files)} TOML files')"
git diff --check
git status --short
```

Check unique role names, matching filenames, nonempty descriptions and instructions,
supported model/effort combinations, skill frontmatter, and local documentation
links. If the system `skill-creator` is installed, run its `scripts/quick_validate.py`
against each skill directory.

On clients supporting strict config validation, start a session with:

```sh
codex --strict-config
```

Exit after inspecting startup. Strict mode rejects unrecognized configuration
fields; it does not prove each role can run.
Confirm role discovery and perform the read-only smoke test described above after
adoption. The template uses standalone agent TOML files and current `[agents]`
settings; check compatibility with your client using the
[configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).

## Maintain

Keep shared conduct in `AGENTS.md`, project facts in `AGENTS_PROJECT_REFERENCE.md`,
role instructions in their TOML files, and reusable procedures in skills. Add a
role or skill only when it has a distinct responsibility or workflow. Update this
catalog when those responsibilities change.
