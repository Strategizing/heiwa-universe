# GitHub CLI (`gh`) Playbook

Use this file for GitHub repository, PR, and GitHub Actions workflows.

## Auth and Context

Start with:

```bash
gh --version
gh auth status
gh repo view
```

If `gh auth status` fails, stop and ask whether to authenticate (`gh auth login`) or use a token.

## CI / GitHub Actions Triage

Common flow:

```bash
gh run list --limit 10
gh run view <run-id>
gh run view <run-id> --log
gh run view <run-id> --json jobs,conclusion,event,headBranch,headSha
```

Use job-focused inspection when a run contains multiple jobs:

```bash
gh run view <run-id> --json jobs
```

Re-run only after the root cause is understood:

```bash
gh run rerun <run-id>
```

Use `gh run watch <run-id>` to monitor a rerun.

## PR and Review Workflow

Useful commands:

```bash
gh pr status
gh pr view <number> --json title,state,headRefName,baseRefName,reviews,files
gh pr checks <number>
gh pr checkout <number>
```

## Repo and Issue Ops

Examples:

```bash
gh issue list
gh issue view <number>
gh release list
```

Use `--json` for scripts or structured summaries.

## Secrets and Variables

Avoid printing values. Inspect metadata only:

```bash
gh secret list
gh variable list
```

Scope may matter (repo, environment, org). Confirm target scope before changes.

## Common Failure Modes

- Wrong repository context (`gh repo view` points to another repo)
- Missing auth scopes for Actions, secrets, or org operations
- Rerun hides a flaky issue without fixing it
- CI succeeded but deploy provider failed (switch to Railway/Wrangler logs next)
