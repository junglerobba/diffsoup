use crate::error::{CustomError, Result};
use error_stack::ResultExt;
use jj_cli::{
    cli_util::{find_workspace_dir, start_repo_transaction},
    config::{ConfigEnv, config_from_environment, default_config_layers},
};
use jj_lib::{
    backend::CommitId,
    config::{ConfigLayer, ConfigSource},
    git::{self, GitRefKind, GitSettings, parse_git_ref},
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
    let workspace_root = TempDir::new()
        .change_context(CustomError::RepoError)
        .attach("could not create dir for jj workspace")?;

    let repo_path = workspace_root.path().join(".jj/repo");

    let git_repo = gix::open(git_repo_path).change_context(CustomError::RepoError)?;
    let trunk_alias = get_trunk_alias(&git_repo)?;

    let mut raw_config = config_from_environment(default_config_layers());
    if let Some(ref symbol) = trunk_alias {
        let mut layer = ConfigLayer::empty(ConfigSource::Workspace);
        layer
            .set_value("revset-aliases.trunk", symbol.to_string())
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

    let (workspace, repo) = Workspace::init_external_git(
        &settings,
        workspace_root.path(),
        &git_repo_path.join(".git"),
    )
    .change_context(CustomError::RepoError)
    .attach("could not initialize jj repo")?;

    let git_settings =
        GitSettings::from_settings(repo.settings()).change_context(CustomError::RepoError)?;
    let mut tx = start_repo_transaction(&repo, &[]);
    git::import_refs(tx.repo_mut(), &git_settings).change_context(CustomError::RepoError)?;

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
