use crate::error::{CustomError, Result};
use error_stack::ResultExt;
use jj_cli::{
    cli_util::find_workspace_dir,
    config::{ConfigEnv, config_from_environment, default_config_layers},
};
use jj_lib::{
    git_backend::GitBackend,
    local_working_copy::{LocalWorkingCopy, LocalWorkingCopyFactory},
    repo::StoreFactories,
    settings::UserSettings,
    workspace::{
        DefaultWorkspaceLoaderFactory, WorkingCopyFactories, Workspace, WorkspaceLoaderFactory,
    },
};
use std::path::Path;

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
