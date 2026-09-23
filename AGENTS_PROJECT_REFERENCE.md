# Project Reference

## Purpose and boundaries

This repository is a reusable Codex configuration template. Its deliverables are
agent definitions, workflow skills, and supporting guidance. Keep application
source, sample products, dependency manifests, build outputs, and runtime state
out of this template. Add configuration or documentation only when it serves reuse.

## Layout and ownership

- `.codex/config.toml`: primary model defaults and subagent controls.
- `.codex/agents/*.toml`: one self-contained role per file; authoritative role
  settings, selection descriptions, instructions, and return contracts.
- `.agents/skills/*/SKILL.md`: reusable workflows, selected only when applicable.
- `AGENTS.md`: shared execution, delegation, validation, and handoff expectations.
- `README.md`: setup, role catalog, checks, and adaptation guidance.

Keep general conduct in `AGENTS.md`, role-specific instructions in agent files,
and task-specific procedures in skills. Avoid duplicating them.

## Configuration conventions

- Pair model and reasoning settings in every role. Preserve existing assignments
  unless a requested change warrants revising them.
- Match each role filename to its unique `name`. Keep scope and boundaries concise.
- Advisory roles use `read-only`; editing and validation roles use
  `workspace-write` with explicit editing limits. Runtime policy can override defaults.
- Keep credentials, trust, platform setup, and personal integrations in user or
  managed configuration.
- Match skill folders to their frontmatter names. Use precise discovery descriptions
  and relative references; add scripts only for useful repeated automation.

## Validation

There is no application build or test suite. For template changes:

1. Parse all TOML files; check required role fields, unique names, and paired settings.
2. Check skill frontmatter, scope, and local links. Run the skill-creator validator
   when available.
3. Use a compatible client's strict config check and inspect role discovery when
   changing configuration. Syntax validation alone does not prove runtime loading.
4. Review `git diff --check`, the full diff, and the untracked file list.

Runnable syntax checks and client verification guidance are in `README.md`.

## Adapting this reference

When adopting the template for an application repository, replace this document's
template-specific content with verified project facts:

- Purpose, users, scope, and explicit non-goals.
- Stack, runtime versions, package manager, and setup prerequisites.
- Module map, entry points, data flow, and ownership boundaries.
- Architectural constraints, public interfaces, and critical invariants.
- Exact test, lint, type-check, build, and local-run commands with working directories.
- Environment variable names and credential sources, never their values.
- Canonical documentation and confirmed baseline failures.

Mark unknowns explicitly. Link to maintained sources instead of copying large
documents. Refresh this reference when the project's behavior or structure changes.
