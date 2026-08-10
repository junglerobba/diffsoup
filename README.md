# diffsoup
**A Gerrit-style patchset diff viewer for pull requests, using jujutsu**

Anyone who prefers a patch-based workflow rather than the branch-based one used
by most git forges will know that these tools are terrible at showing changes
between iterations.

Diffsoup exists to make rebased and amended pull requests reviewable again, by
comparing patchset to patchset on a per-commit basis instead of bundling all
changes, including everything coming in from trunk.

## Usage
The most common usage is simply:
```sh
diffsoup <pull request url>
```
This needs to be run inside a local checkout of the target repository, and you
must be able to fetch from its remote via SSH.

diffsoup will then fetch the PR history and any commits that do not exist
locally, rebase and interdiff those patchsets using jj-lib and present them in a
gerrit-style view of each iteration.

This way it requires no special support from the forge other than pull request
history.

### Configuration and authentication
Configuration is done through gitconfig, through user and repo level config,
with the same priority as usual.

`~/.gitconfig`
```gitconfig
# subsection name must match pull request URL
[diffsoup "https://git.example.org"]
    # one of github, gitlab, bitbucket, bitbucket-datacenter
    forge = gitlab
    # optional, env used as fallback
    tokenCommand = pass Token/git.example.org
```

The tool attempts to detect forge automatically from the pull request URL, but
manual configuration always takes precedence.
Same with authentication, which might be required for accessing pull request
history, where the output of tokenCommand is used, with the possibility of
overriding it via these env vars:

 - GitHub: `GITHUB_TOKEN`, with `gh auth token` as a fallback, if `gh` is
  installed
 - Gitlab: `GITLAB_TOKEN`
 - Bitbucket Cloud / Data Center: `BITBUCKET_TOKEN`

tokenCommand executes as a shell command, so use with caution, and only in
trusted repos.

### Change tracking
For reliable tracking across rebases, diffsoup relies on the change-id commit
header (visible via `git cat-file -p <sha>`). This is not the same `Change-Id:`
commit trailer as used by Gerrit, but instead an emerging standard being adopted
across git tooling for similar logical change tracking.

 - [jj] writes this header by default since v0.30.0
 - GitButler has agreed with jj on adopting this standard
 - [Gerrit] is considering jj support via this header
 - There are active conversations about standardizing this upstream in [git]

If a commit does not contain a change-id header, diffsoup falls back to a
best-effort heuristic based on author identity and timestamps.
This is an approximation and may create mismatches, so for best results, the
header is recommended, although it's not always that easy to convince colleagues
to adopt new tooling :)

## Installation
Other than a rust toolchain, no additional dependencies are currently required.
```sh
cargo install --path .
```
A nix flake is included for both package installation and a development shell.

### Status
The tool is in a working state, though expect some rough edges, especially
regarding error reporting.

Support for more git forges is straightforward to implement, provided the forge
exposes an API to fetch the full iteration history of a pull request, including
commit SHAs. Contributions in this area are welcome.

The scope is intentionally small, so it does not handle anything except
displaying patchset iterations and diffs. Diffs can be copied to clipboard in
standard git diff format and pasted into another diff viewer or used as patches,
but it's not intended for this to replace a code review UI, only to answer the
question of what a push actually changed.

[jj]: https://github.com/jj-vcs/jj/releases/tag/v0.30.0
[gerrit]: https://gerrit-review.googlesource.com/c/homepage/+/464287
[git]: https://lore.kernel.org/git/CAESOdVAspxUJKGAA58i0tvks4ZOfoGf1Aa5gPr0FXzdcywqUUw@mail.gmail.com/T/#u

