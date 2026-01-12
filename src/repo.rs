use crate::error::{CustomError, Result};
use error_stack::ResultExt;
use jj_cli::{
    cli_util::{find_workspace_dir, start_repo_transaction},
    config::{ConfigEnv, config_from_environment, default_config_layers},
};
use jj_lib::{
    backend::CommitId,
    commit::Commit,
    config::{ConfigLayer, ConfigSource},
    git::{self, GitImportOptions, GitRefKind, parse_git_ref},
    git_backend::GitBackend,
    local_working_copy::{LocalWorkingCopy, LocalWorkingCopyFactory},
    repo::{ReadonlyRepo, Repo, StoreFactories},
    settings::UserSettings,
    workspace::{
        DefaultWorkspaceLoaderFactory, WorkingCopyFactories, Workspace, WorkspaceLoaderFactory,
    },
};
use std::{collections::HashMap, path::Path, sync::Arc};
use temp_dir::TempDir;

pub struct RepoHandle {
    pub repo: Arc<ReadonlyRepo>,
    pub workspace: Workspace,
    _tempdir: Option<TempDir>,
}

pub fn open(path: &Path) -> Result<RepoHandle> {
    let workspace_path = path.join(".jj");
    if !workspace_path.exists() {
        return init_jj_repo(path);
    };
    let workspace = load_jj_repo(path)?;
    let repo = workspace
        .repo_loader()
        .load_at_head()
        .change_context(CustomError::RepoError)?;
    Ok(RepoHandle {
        repo,
        workspace,
        _tempdir: None,
    })
}

fn load_jj_repo(path: &Path) -> Result<Workspace> {
    let mut raw_config = config_from_environment(default_config_layers());
    let mut config_env = ConfigEnv::from_environment();
    let loader = DefaultWorkspaceLoaderFactory
        .create(find_workspace_dir(path))
        .change_context(CustomError::RepoError)?;
    config_env.reset_repo_path(loader.repo_path());
    config_env
        .reload_repo_config(&mut raw_config)
        .change_context(CustomError::ConfigError)?;
    config_env.reset_workspace_path(loader.workspace_root());
    config_env
        .reload_workspace_config(&mut raw_config)
        .change_context(CustomError::ConfigError)?;
    let config = config_env
        .resolve_config(&raw_config)
        .change_context(CustomError::RepoError)?;
    let mut store_factories = StoreFactories::default();
    store_factories.add_backend(
        GitBackend::name(),
        Box::new(|settings, store_path| Ok(Box::new(GitBackend::load(settings, store_path)?))),
    );
    let mut working_copy_factories = WorkingCopyFactories::new();
    working_copy_factories.insert(
        LocalWorkingCopy::name().to_owned(),
        Box::new(LocalWorkingCopyFactory {}),
    );
    let settings = UserSettings::from_config(config).change_context(CustomError::RepoError)?;

    loader
        .load(&settings, &store_factories, &working_copy_factories)
        .change_context(CustomError::RepoError)
}

fn init_jj_repo(git_repo_path: &Path) -> Result<RepoHandle> {
    let git_repo_path = git_repo_path
        .canonicalize()
        .change_context(CustomError::RepoError)
        .attach("failed to resolve repository path")?;

    let git_repo = gix::open(&git_repo_path).change_context(CustomError::RepoError)?;
    let trunk_alias = get_trunk_alias(&git_repo)?;

    let workspace_root = TempDir::new()
        .change_context(CustomError::RepoError)
        .attach("could not create dir for jj workspace")?;
    let repo_path = workspace_root.path().join(".jj/repo");

    let mut raw_config = config_from_environment(default_config_layers());
    if let Some(ref symbol) = trunk_alias {
        let mut layer = ConfigLayer::empty(ConfigSource::User);
        layer
            .set_value("revset-aliases.\"trunk()\"", symbol.to_string())
            .change_context(CustomError::ConfigError)?;
        raw_config.as_mut().add_layer(layer);
    }

    let mut config_env = ConfigEnv::from_environment();
    config_env.reset_repo_path(&repo_path);
    config_env
        .reload_repo_config(&mut raw_config)
        .change_context(CustomError::ConfigError)?;
    config_env.reset_workspace_path(workspace_root.path());
    config_env
        .reload_workspace_config(&mut raw_config)
        .change_context(CustomError::ConfigError)?;
    let config = config_env
        .resolve_config(&raw_config)
        .change_context(CustomError::RepoError)?;
    let settings = UserSettings::from_config(config).change_context(CustomError::RepoError)?;

    let (workspace, repo) =
        Workspace::init_external_git(&settings, workspace_root.path(), git_repo.path())
            .change_context(CustomError::RepoError)
            .attach("could not initialize jj repo")?;

    let mut tx = start_repo_transaction(&repo, &[]);
    git::import_refs(
        tx.repo_mut(),
        &GitImportOptions {
            auto_local_bookmark: false,
            abandon_unreachable_commits: false,
            remote_auto_track_bookmarks: HashMap::new(),
        },
    )
    .change_context(CustomError::RepoError)?;

    let repo = tx
        .commit("import git refs")
        .change_context(CustomError::RepoError)?;

    Ok(RepoHandle {
        workspace,
        repo,
        _tempdir: Some(workspace_root),
    })
}

fn get_trunk_alias(repo: &gix::Repository) -> Result<Option<String>> {
    for remote in ["upstream", "origin"] {
        let ref_name = format!("refs/remotes/{remote}/HEAD");
        if let Some(reference) = repo
            .try_find_reference(&ref_name)
            .change_context(CustomError::RepoError)?
            && let Some(reference_name) = reference.target().try_name()
            && let Some((GitRefKind::Bookmark, symbol)) = str::from_utf8(reference_name.as_bstr())
                .ok()
                .and_then(|name| parse_git_ref(name.as_ref()))
        {
            let symbol = symbol.name.to_remote_symbol(remote.as_ref());
            return Ok(Some(symbol.to_string()));
        }
    }
    Ok(None)
}

#[derive(Debug)]
pub enum StoreSearchResult<'a> {
    Fetch(&'a CommitId),
    Import(Commit),
}

pub fn ensure_commits_exist<'a, I>(shas: I, repo: &impl Repo) -> Result<Vec<StoreSearchResult<'a>>>
where
    I: IntoIterator<Item = &'a CommitId>,
{
    let missing: Vec<StoreSearchResult> = shas
        .into_iter()
        .filter_map(|sha| {
            let commit = match repo.store().get_commit(sha) {
                Ok(commit) => commit,
                Err(_) => return Some(StoreSearchResult::Fetch(sha)),
            };

            let jj_has_commit = repo.index().has_id(sha).is_ok_and(|value| value);
            if !jj_has_commit {
                return Some(StoreSearchResult::Import(commit));
            }

            None
        })
        .collect();
    Ok(missing)
}

pub fn fetch_commits(
    commits: &[StoreSearchResult],
    repo: Arc<ReadonlyRepo>,
) -> Result<Arc<ReadonlyRepo>> {
    let to_fetch: Vec<&CommitId> = commits
        .iter()
        .filter_map(|c| match c {
            StoreSearchResult::Fetch(id) => Some(*id),
            StoreSearchResult::Import(_) => None,
        })
        .collect();

    if !to_fetch.is_empty() {
        let Some(git_backend) = repo.store().backend_impl::<GitBackend>() else {
            return Err(CustomError::CommitError("not backed by a git repo".to_string()).into());
        };
        let git_repo = git_backend.git_repo();

        let remote = git_repo
            .find_default_remote(gix::remote::Direction::Fetch)
            .transpose()
            .change_context(CustomError::RepoError)?
            .ok_or(CustomError::CommitError(
                "No default remote configured".to_string(),
            ))?;

        let refspecs: Vec<String> = to_fetch.iter().map(|c| format!("{}", c)).collect();
        let remote = remote
            .with_refspecs(
                refspecs.iter().map(|s| s.as_str()),
                gix::remote::Direction::Fetch,
            )
            .change_context(CustomError::RepoError)?;
        let connection = remote
            .connect(gix::remote::Direction::Fetch)
            .change_context(CustomError::RequestError)?;
        connection
            .prepare_fetch(
                gix::progress::Discard,
                gix::remote::ref_map::Options::default(),
            )
            .change_context(CustomError::RequestError)?
            .receive(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
            .change_context(CustomError::RequestError)?;

        git_backend
            .import_head_commits(to_fetch.iter().copied())
            .change_context(CustomError::RepoError)?;
    }

    let mut tx = repo.start_transaction();

    let store = tx.repo_mut().store();
    let commits: Vec<Commit> = commits
        .iter()
        .map(|result| match result {
            StoreSearchResult::Fetch(id) => {
                store.get_commit(id).change_context(CustomError::RepoError)
            }
            StoreSearchResult::Import(commit) => Ok(commit.clone()),
        })
        .collect::<Result<Vec<_>>>()?;

    tx.repo_mut()
        .add_heads(&commits)
        .change_context(CustomError::RepoError)?;

    for commit in &commits {
        tx.repo_mut().remove_head(commit.id());
    }

    let updated_repo = tx
        .commit("import fetched commits")
        .change_context(CustomError::RepoError)?;

    Ok(updated_repo)
}
