use crate::error::{CustomError, Result};
use error_stack::ResultExt;
use jj_cli::{
    cli_util::find_workspace_dir,
    config::{ConfigEnv, config_from_environment, default_config_layers},
};
use jj_lib::{
    backend::CommitId,
    git,
    git_backend::GitBackend,
    local_working_copy::{LocalWorkingCopy, LocalWorkingCopyFactory},
    object_id::ObjectId,
    ref_name::RefNameBuf,
    repo::{ReadonlyRepo, Repo, StoreFactories},
    settings::UserSettings,
    workspace::{
        DefaultWorkspaceLoaderFactory, WorkingCopyFactories, Workspace, WorkspaceLoaderFactory,
    },
};
use std::{path::Path, sync::Arc};

// TODO support plain git repos
// bit more complex because it would need to initialize a temporary colocated jj repo
pub fn open(path: &Path) -> Result<Workspace> {
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

pub fn ensure_commits_exist<'a, I>(shas: I, repo: &impl Repo) -> Result<Vec<&'a str>>
where
    I: Iterator<Item = &'a RefNameBuf>,
{
    let Some(git_backend) = repo.store().backend_impl::<GitBackend>() else {
        return Err(CustomError::CommitError("not backed by a git repo".to_string()).into());
    };
    let git_repo = git_backend.git_repo();
    let missing = shas
        .map(|sha| {
            let commit_id = CommitId::try_from_hex(sha.as_str()).ok_or(CustomError::RepoError)?;
            let object_id = gix::ObjectId::try_from(commit_id.as_bytes())
                .change_context(CustomError::RepoError)?;
            Ok(git_repo
                .find_commit(object_id)
                .is_err()
                .then_some(sha.as_str()))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<&str>>();
    Ok(missing)
}

pub fn fetch_commits<'a, I>(commits: I, repo: Arc<ReadonlyRepo>) -> Result<Arc<ReadonlyRepo>>
where
    I: Iterator<Item = &'a str>,
{
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

    let remote_name = remote.name().map(|n| n.as_ref()).unwrap_or("origin".into());
    let refspecs: Vec<String> = commits
        .map(|sha| format!("{}:refs/remotes/{}/{}", sha, remote_name, sha))
        .collect();

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

    // import the fetched refs into jj
    let git_settings = git::GitSettings::from_settings(repo.settings())
        .change_context(CustomError::ConfigError)?;
    let mut tx = repo.start_transaction();
    git::import_refs(tx.repo_mut(), &git_settings).change_context(CustomError::RepoError)?;
    let updated_repo = tx
        .commit("import fetched commits")
        .change_context(CustomError::RepoError)?;

    Ok(updated_repo)
}
