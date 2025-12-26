use crate::{
    error::{CustomError, Result},
    trees::DiffTree,
};
use error_stack::ResultExt;
use jj_cli::{
    diff_util::{self, DiffFormat, DiffRenderer, DiffStatOptions, UnifiedDiffOptions},
    formatter::ColorFormatter,
    revset_util,
    ui::Ui,
};
use jj_lib::{
    commit::Commit,
    conflicts::ConflictMarkerStyle,
    copies::CopyRecords,
    git_backend::GitBackend,
    object_id::ObjectId,
    repo::Repo,
    repo_path::RepoPathUiConverter,
    revset::{
        self, Revset, RevsetDiagnostics, RevsetExtensions, RevsetIteratorExt, RevsetParseContext,
        RevsetWorkspaceContext, SymbolResolver, SymbolResolverExtension,
    },
    rewrite::rebase_to_dest_parent,
    workspace::Workspace,
};
use std::{collections::HashMap, fs::canonicalize, path::PathBuf};

#[derive(Debug, Clone)]
pub struct CommitDiff {
    pub from: Option<CommitMeta>,
    pub to: Option<CommitMeta>,
    pub stats: DiffStats,
}

impl CommitDiff {
    pub fn has_changes(&self) -> bool {
        match (&self.from, &self.to) {
            (None, Some(_)) | (Some(_), None) => true,
            (Some(from), Some(to)) => self.stats.changed_files > 0 && from.sha != to.sha,
            (None, None) => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommitMeta {
    pub sha: String,
    pub message: String,
}

#[derive(Debug, Default, Copy, Clone)]
pub struct DiffStats {
    pub additions: usize,
    pub removals: usize,
    pub changed_files: usize,
}

fn evaluate_revset_expr<'a>(
    expr: &str,
    workspace: &Workspace,
    repo: &'a impl Repo,
) -> Result<Box<dyn Revset + 'a>> {
    let aliases_map = &revset_util::load_revset_aliases(&Ui::null(), workspace.settings().config())
        .map_err(|_| CustomError::RepoError)?;
    let cwd = canonicalize(PathBuf::from(".")).change_context(CustomError::RepoError)?;
    let context = RevsetParseContext {
        aliases_map,
        local_variables: HashMap::new(),
        user_email: "",
        date_pattern_context: chrono::Utc::now().fixed_offset().into(),
        default_ignored_remote: None,
        use_glob_by_default: false,
        extensions: &RevsetExtensions::default(),
        workspace: Some(RevsetWorkspaceContext {
            path_converter: &RepoPathUiConverter::Fs {
                cwd,
                base: workspace.workspace_root().to_owned(),
            },
            workspace_name: workspace.workspace_name(),
        }),
    };
    let expression = revset::parse(&mut RevsetDiagnostics::default(), expr, &context)
        .change_context(CustomError::ExprError)?;
    let symbol_resolver = SymbolResolver::new(repo, &[] as &[Box<dyn SymbolResolverExtension>]);
    let resolved = expression
        .resolve_user_expression(repo, &symbol_resolver)
        .change_context(CustomError::ExprError)?;
    resolved
        .evaluate(repo)
        .change_context(CustomError::ExprError)
}

pub fn get_commit(expr: &str, workspace: &Workspace, repo: &impl Repo) -> Result<Commit> {
    let revset = evaluate_revset_expr(expr, workspace, repo)?;
    let mut iter = revset.iter().commits(repo.store());
    match (iter.next(), iter.next()) {
        (Some(Ok(commit)), None) => Ok(commit),
        (Some(_), Some(_)) => Err(CustomError::CommitError(
            "expression resolved to more than one commit".to_string(),
        )
        .into()),
        (_, _) => Err(CustomError::CommitError(
            "expression didn't resolve to a commit".to_string(),
        )
        .into()),
    }
}

fn get_commits(expr: &str, workspace: &Workspace, repo: &impl Repo) -> Result<Vec<Commit>> {
    let revset = evaluate_revset_expr(expr, workspace, repo)?;
    revset
        .iter()
        .commits(repo.store())
        .collect::<std::result::Result<Vec<_>, _>>()
        .change_context(CustomError::ExprError)
}

#[derive(Clone, Hash, Debug, PartialEq, Eq)]
enum DiffSource {
    ChangeId(String),
    // If change ids are not available, fall back to commit metadata
    // which doesn't change across rewrites for best effort matching
    Meta {
        author_name: String,
        author_email: String,
        author_timestamp: i64,
    },
}

impl DiffSource {
    pub fn from_commit(commit: &Commit, repo: &impl Repo) -> Result<Self> {
        if let Some(git_backend) = repo.store().backend_impl::<GitBackend>() {
            let object_id = gix::ObjectId::try_from(commit.id().as_bytes())
                .change_context(CustomError::RepoError)?;
            let repo = git_backend.git_repo();
            let git_commit = repo
                .find_commit(object_id)
                .change_context(CustomError::RepoError)?;
            let decoded = git_commit.decode().change_context(CustomError::RepoError)?;
            if decoded.extra_headers().find("change-id").is_some() {
                return Ok(DiffSource::ChangeId(commit.change_id().reverse_hex()));
            }
        }
        Ok(DiffSource::Meta {
            author_name: commit.author().name.to_owned(),
            author_email: commit.author().email.to_owned(),
            author_timestamp: commit.author().timestamp.timestamp.0,
        })
    }
}

pub fn calculate_branch_diff(
    from_branch: &str,
    to_branch: &str,
    workspace: &Workspace,
    repo: &impl Repo,
) -> Result<Vec<CommitDiff>> {
    let fork_point_expr = format!("fork_point({} | {} | trunk())", from_branch, to_branch);

    let from_expr = format!("{}..{}", fork_point_expr, from_branch);
    let from_commits = get_commits(&from_expr, workspace, repo)?;

    let to_expr = format!("::{} ~ ::trunk()", to_branch);
    let to_commits = get_commits(&to_expr, workspace, repo)?;

    let from_sources = from_commits
        .iter()
        .map(|c| DiffSource::from_commit(c, repo))
        .collect::<Result<Vec<_>>>()?;
    let to_sources = to_commits
        .iter()
        .map(|c| DiffSource::from_commit(c, repo))
        .collect::<Result<Vec<_>>>()?;

    let mut from_map = HashMap::new();
    let mut to_map = HashMap::new();
    let mut change_ids = Vec::new();

    let mut from_idx = 0;
    let mut to_idx = 0;

    while from_idx < from_sources.len() || to_idx < to_sources.len() {
        match (from_sources.get(from_idx), to_sources.get(to_idx)) {
            (Some(from_source), Some(to_source)) => {
                if from_source == to_source {
                    from_map.insert(from_source.clone(), &from_commits[from_idx]);
                    to_map.insert(to_source.clone(), &to_commits[to_idx]);
                    change_ids.push(from_source);

                    from_idx += 1;
                    to_idx += 1;
                } else if from_sources[from_idx..].contains(to_source) {
                    from_map.insert(from_source.clone(), &from_commits[from_idx]);
                    change_ids.push(from_source);
                    from_idx += 1;
                } else {
                    to_map.insert(to_source.clone(), &to_commits[to_idx]);
                    change_ids.push(to_source);
                    to_idx += 1;
                }
            }
            (Some(from_source), None) => {
                from_map.insert(from_source.clone(), &from_commits[from_idx]);
                change_ids.push(from_source);
                from_idx += 1;
            }
            (None, Some(to_source)) => {
                to_map.insert(to_source.clone(), &to_commits[to_idx]);
                change_ids.push(to_source);
                to_idx += 1;
            }
            (None, None) => unreachable!(),
        }
    }

    let mut commit_diffs = Vec::new();

    for change_id in change_ids {
        let from_commit = from_map.get(change_id);
        let to_commit = to_map.get(change_id);

        let from_meta = from_commit.map(|c| CommitMeta {
            sha: c.id().hex(),
            message: c.description().to_owned(),
        });

        let to_meta = to_commit.map(|c| CommitMeta {
            sha: c.id().hex(),
            message: c.description().to_owned(),
        });

        let stats = match (from_commit, to_commit) {
            (Some(from), Some(to)) if from.id() == to.id() => calculate_commit_stats(to, repo),
            (Some(from), Some(to)) => calculate_diff_stats(from, to, repo),
            (Some(from), None) => calculate_commit_stats(from, repo),
            (None, Some(to)) => calculate_commit_stats(to, repo),
            (None, None) => Ok(DiffStats::default()),
        }
        .change_context(CustomError::RepoError)?;

        commit_diffs.push(CommitDiff {
            from: from_meta,
            to: to_meta,
            stats,
        });
    }

    Ok(commit_diffs)
}

fn calculate_diff_stats(from: &Commit, to: &Commit, repo: &impl Repo) -> Result<DiffStats> {
    let from_tree = rebase_to_dest_parent(repo, std::slice::from_ref(from), to)
        .change_context(CustomError::RepoError)?;
    let to_tree = to.tree();

    let matcher = jj_lib::matchers::EverythingMatcher;
    let copy_records = CopyRecords::default();
    let diff_stream = from_tree.diff_stream_with_copies(&to_tree, &matcher, &copy_records);

    let diff_stat_options = DiffStatOptions::default();

    let stats = futures::executor::block_on(diff_util::DiffStats::calculate(
        repo.store(),
        diff_stream,
        &diff_stat_options,
        ConflictMarkerStyle::Git,
    ))
    .change_context(CustomError::ProcessError(
        "couldn't block on future".to_owned(),
    ))?;

    Ok(DiffStats {
        additions: stats.count_total_added(),
        removals: stats.count_total_removed(),
        changed_files: stats.entries().len(),
    })
}

fn calculate_commit_stats(commit: &Commit, repo: &impl Repo) -> Result<DiffStats> {
    let parents: Vec<Commit> = commit
        .parents()
        .collect::<std::result::Result<Vec<_>, _>>()
        .change_context(CustomError::CommitError(
            "failed to get commit parents".to_string(),
        ))?;

    if parents.is_empty() {
        return Ok(DiffStats::default());
    }

    let parent = &parents[0];
    calculate_diff_stats(parent, commit, repo)
}

pub fn render_interdiff(
    trees: &DiffTree,
    workspace: &Workspace,
    repo: &impl Repo,
    width: u16,
) -> Result<String> {
    let (from_tree, to_tree) = trees.get_trees(repo)?;

    let matcher = jj_lib::matchers::EverythingMatcher;

    let cwd = canonicalize(PathBuf::from(".")).change_context(CustomError::RepoError)?;
    let repo_path_converter = RepoPathUiConverter::Fs {
        cwd,
        base: workspace.workspace_root().to_owned(),
    };
    let renderer = DiffRenderer::new(
        repo,
        &repo_path_converter,
        ConflictMarkerStyle::Git,
        vec![DiffFormat::Git(Box::new(
            UnifiedDiffOptions::from_settings(workspace.settings())
                .change_context(CustomError::ConfigError)?,
        ))],
    );

    let copy_records = CopyRecords::default();
    let mut diff = Vec::new();
    let mut formatter = ColorFormatter::new(&mut diff, Vec::new().into(), false);
    futures::executor::block_on(renderer.show_diff(
        &Ui::null(),
        &mut formatter,
        jj_lib::merge::Diff::new(&from_tree, &to_tree),
        &matcher,
        &copy_records,
        width.into(),
    ))
    .change_context(CustomError::ProcessError(
        "couldn't block on future".to_owned(),
    ))?;

    drop(formatter);
    String::from_utf8(diff).change_context(CustomError::ProcessError(
        "failed to parse diff output as UTF-8".to_owned(),
    ))
}
