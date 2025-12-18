# diffsoup
**A Gerrit-style patchset diff viewer for pull requests, using jujutsu**

Showing not just the final diff, but what actually changed across rebases.

Anyone who prefers a patch-based workflow rather than the branch-based one preferred by most git forges will know that these tools are terrible at showing changes between iterations.

Diffsoup exists to make rebased and amended pull requests reviewable again, by comparing patchset to patchset instead of branch to base.


## Usage
The most common usage is simply:
```sh
diffsoup <pull request url>
```
This needs to be run inside a local checkout of the target repository, and you must be able to fetch from its remote.

diffsoup will then fetch the PR history and any commits that do not exist locally, rebase and interdiff those patchsets using jj-lib and present them in a gerrit-style view of each iteration.

This way it requires no special support from the forge other than pull request history.

### Authentication
For accessing pull request history, authentication may be required. This is currently done via environment variables:

 - GitHub: `GITHUB_TOKEN`
 - Bitbucket Data Center: `BITBUCKET_TOKEN`

### Change tracking
For reliable tracking across rebases, diffsoup relies on the change-id commit header (visible via `git cat-file -p <sha>`). This is not the same `Change-Id:` commit trailer as used by Gerrit,
but instead an emerging standard being adopted across git tooling for similar logical change tracking.

 - [jj] writes this header by default since v0.30.0
 - GitButler has agreed with jj on adopting this standard
 - [Gerrit] is considering jj support via this header
 - There are active conversations about standardizing this upstream in [git]

If a commit does not contain a change-id header, diffsoup falls back to a best-effort heuristic based on author identity and timestamps.  
This is an approximation and may create mismatches, so for best results, the header is recommended, although it's not always that easy to convince colleagues to adopt new tooling :)

## Installation
Other than a rust toolchain, no additional dependencies are currently required.
```sh
cargo install --path .
```
A nix flake is included for both package installation and a development shell.

### Status
The tool is in a working state, though expect some rough edges, especially regarding error reporting.

Support for more git forges is straightforward to implement, provided the forge exposes an API to fetch the full iteration history of a pull request, including commit SHAs. Contributions in this area are welcome.

The scope is intentionally small, so it does not handle anything except displaying patchset iterations and diffs. Diffs can be copied to clipboard in standard git diff format and pasted into another diff viewer or used as patches, but it's not intended for this to replace your code review UI, only to answer the question of what a push actually changed.

[jj]: https://github.com/jj-vcs/jj/releases/tag/v0.30.0
[gerrit]: https://gerrit-review.googlesource.com/c/homepage/+/464287
[git]: https://lore.kernel.org/git/CAESOdVAspxUJKGAA58i0tvks4ZOfoGf1Aa5gPr0FXzdcywqUUw@mail.gmail.com/T/#u

