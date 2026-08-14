mod bitbucket;
mod bitbucket_cloud;
mod github;
mod gitlab;
mod none;

use error_stack::ResultExt;
use gix::{bstr::ByteSlice, utils::AsBStrOpt};
use jj_lib::{
    backend::CommitId,
    git_backend::GitBackend,
    repo::{ReadonlyRepo, Repo},
    workspace::Workspace,
};
use std::{env::VarError, fmt::Debug, process::Stdio, sync::Arc};
use url::Url;

use crate::{
    error::{CustomError, Result},
    pr::{
        bitbucket::BitbucketFetcher, bitbucket_cloud::BitbucketCloudFetcher, github::GithubFetcher,
        gitlab::GitlabFetcher, none::NoFetcher,
    },
};

#[derive(Debug, Clone, Copy, Default)]
pub enum PageDirection {
    #[default]
    Forward,
    Backward,
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub head_ref: CommitId,
    pub base_ref: Option<CommitId>,
    pub unpublished: bool,
}

impl HistoryEntry {
    pub fn new(head_ref: CommitId, base_ref: Option<CommitId>) -> Self {
        Self {
            head_ref,
            base_ref,
            unpublished: false,
        }
    }

    pub fn pending(mut self, pending: bool) -> Self {
        self.unpublished = pending;
        self
    }
}

impl From<CommitId> for HistoryEntry {
    fn from(value: CommitId) -> Self {
        Self {
            head_ref: value,
            base_ref: None,
            unpublished: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub direction: PageDirection,
    pub next: Option<Pagination>,
}

impl<T> Page<T> {
    pub fn latest(&self) -> Option<&T> {
        match self.direction {
            PageDirection::Forward => self.items.first(),
            PageDirection::Backward => self.items.last(),
        }
    }

    pub fn insert(&mut self, item: T) {
        match self.direction {
            PageDirection::Forward => self.items.insert(0, item),
            PageDirection::Backward => self.items.push(item),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct OffsetPagination {
    offset: usize,
    limit: Option<usize>,
    direction: PageDirection,
}

#[derive(Debug, Clone, Default)]
pub struct CursorPagination {
    cursor: Option<String>,
    limit: usize,
    direction: PageDirection,
}

#[derive(Debug, Clone)]
pub enum Pagination {
    Offset(OffsetPagination),
    Cursor(CursorPagination),
}

impl Pagination {
    pub fn direction(&self) -> PageDirection {
        match self {
            Pagination::Offset(offset) => offset.direction,
            Pagination::Cursor(cursor) => cursor.direction,
        }
    }
}

#[derive(Debug, Copy, Clone)]
enum Forge {
    Github,
    Gitlab,
    Bitbucket,
    BitbucketDatacenter,
}

impl Forge {
    fn new(host: impl AsRef<str>) -> Option<Self> {
        match host.as_ref() {
            "github" => Some(Self::Github),
            "gitlab" => Some(Self::Gitlab),
            "bitbucket" => Some(Self::Bitbucket),
            "bitbucket-datacenter" => Some(Self::BitbucketDatacenter),
            _ => None,
        }
    }

    fn from_url(url: &Url) -> Option<Self> {
        match url.host_str() {
            Some("github.com") => Some(Self::Github),
            Some("gitlab.com") => Some(Self::Gitlab),
            Some("bitbucket.org") => Some(Self::Bitbucket),
            _ => None,
        }
    }

    fn env(self) -> &'static str {
        match self {
            Self::Github => "GITHUB_TOKEN",
            Self::Gitlab => "GITLAB_TOKEN",
            Self::BitbucketDatacenter | Self::Bitbucket => "BITBUCKET_TOKEN",
        }
    }

    fn cmd(self) -> Option<(&'static str, String)> {
        match self {
            Self::Github => Some(("gh", "auth token".to_string())),
            Self::Gitlab | Self::Bitbucket | Self::BitbucketDatacenter => None,
        }
    }
}

#[derive(Debug, Clone)]
enum TokenSource {
    Env(&'static str),
    Command(String),
}

impl TokenSource {
    fn get(&self) -> Result<Option<String>> {
        match &self {
            Self::Env(env) => match std::env::var(env) {
                Ok(res) => Ok(Some(res)),
                Err(VarError::NotPresent) => Ok(None),
                Err(e) => Err(e).change_context_lazy(|| {
                    CustomError::ProcessError(format!("Unable to read token from env {env}"))
                }),
            },

            Self::Command(cmd) => {
                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()
                    .change_context_lazy(|| {
                        CustomError::ProcessError(format!("Error running token command: {cmd}"))
                    })?;
                if !output.status.success() {
                    return Err(CustomError::ProcessError(format!(
                        "Token command {cmd} exited with status {}:\n{}",
                        output.status.code().unwrap_or(-1),
                        String::from_utf8_lossy(&output.stderr)
                    ))
                    .into());
                }
                let token = String::from_utf8(output.stdout).change_context_lazy(|| {
                    CustomError::ProcessError(format!("Invalid output from token command: {cmd}"))
                })?;
                let trimmed = token.trim();
                if trimmed.is_empty() {
                    Err(
                        CustomError::ProcessError(format!("Token command returned nothing: {cmd}"))
                            .into(),
                    )
                } else {
                    Ok(Some(trimmed.into()))
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ForgeConfig {
    forge: Forge,
    token: Vec<TokenSource>,
}

impl ForgeConfig {
    fn new(host: &Url, repo: &gix::Repository) -> Result<Self> {
        let config = repo.config_snapshot();
        let origin = host.origin().unicode_serialization();
        let forge = match config.string_by("diffsoup", origin.as_bstr_opt(), "forge") {
            Some(value) => {
                let value = value.to_str().change_context(CustomError::ConfigError)?;
                Forge::new(value)
                    .ok_or_else(|| CustomError::ProcessError(format!("Unknown forge {value}")))
            }
            None => Forge::from_url(host).ok_or_else(|| {
                CustomError::ProcessError(format!(
                    r#"
Unknown forge host {origin}!
If this is one of the supported forges, then please add this to your git config:

[diffsoup "{origin}"]
    forge = <forge>
    tokenCommand = <optional, command that returns auth token>
                        "#
                ))
            }),
        }?;

        let mut token_source = Vec::new();

        token_source.push(TokenSource::Env(forge.env()));
        if let Some(config_value) = config
            .string_by("diffsoup", origin.as_bstr_opt(), "tokenCommand")
            .map(|value| value.to_string())
        {
            token_source.push(TokenSource::Command(config_value));
        };

        if let Some((tool, cmd)) = forge.cmd()
            && command_exists(tool)
        {
            token_source.push(TokenSource::Command(format!("{tool} {cmd}")));
        }

        Ok(Self {
            forge,
            token: token_source,
        })
    }

    fn token(&self) -> Result<Option<String>> {
        for source in &self.token {
            if let Some(token) = source.get()? {
                return Ok(Some(token));
            }
        }
        Err(CustomError::ProcessError("No configured source returned a token".to_string()).into())
    }
}

#[derive(Debug, Clone)]
pub struct PullRequest {
    pub target_branch: String,
    pub source_branch: String,
}

pub trait PrFetcher: Debug + Send {
    fn get_pull_request(&self) -> Result<Option<PullRequest>> {
        Ok(None)
    }

    fn fetch_history(&self, pagination: Option<&Pagination>) -> Result<Page<HistoryEntry>>;
}

pub fn get_pr_fetcher(
    url: Option<String>,
    from: Option<String>,
    to: Option<String>,
    repo: Arc<ReadonlyRepo>,
    workspace: &Workspace,
) -> Result<Option<Box<dyn PrFetcher>>> {
    match (url, from, to) {
        (None, Some(from), Some(to)) => {
            Ok(Some(Box::new(NoFetcher::new(&from, &to, repo, workspace)?)))
        }
        (Some(url), _, _) => {
            let parsed = url::Url::parse(&url).change_context_lazy(|| {
                CustomError::UrlError(format!("Could not parse URL from {url}"))
            })?;
            let Some(git_backend) = repo.store().backend_impl::<GitBackend>() else {
                return Err(
                    CustomError::CommitError("not backed by a git repo".to_string()).into(),
                );
            };
            let repo = git_backend.git_repo();

            let config = ForgeConfig::new(&parsed, &repo)?;
            let token = config.token()?;
            match config.forge {
                Forge::Github => Ok(Some(Box::new(GithubFetcher::new(&parsed, token, repo)?))),
                Forge::Gitlab => Ok(Some(Box::new(GitlabFetcher::new(&parsed, token)?))),
                Forge::Bitbucket => Ok(Some(Box::new(BitbucketCloudFetcher::new(&parsed, token)?))),
                Forge::BitbucketDatacenter => {
                    Ok(Some(Box::new(BitbucketFetcher::new(&parsed, token)?)))
                }
            }
        }
        (_, _, _) => Ok(None),
    }
}

fn command_exists(cmd: &str) -> bool {
    let res = std::process::Command::new(cmd)
        .stdout(Stdio::null())
        .spawn();
    !matches!(res, Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use anyhow::Context;

    // IMPORTANT: do not add any tests that set or read the same env variable,
    // they WILL get in each other's way. Either find some test runner that can
    // handle this or abstract away from the real env.

    struct Config<'a> {
        forge: &'a str,
        host: &'a str,
        token_command: Option<&'a str>,
    }

    impl<'a> Config<'a> {
        fn print<W: std::io::Write>(&self, w: &mut W) -> anyhow::Result<()> {
            write!(
                w,
                r#"
[diffsoup "https://{}"]
    forge = {}
    {}
            "#,
                self.host,
                self.forge,
                self.token_command
                    .map(|v| format!("tokenCommand = {v}"))
                    .unwrap_or_default()
            )
            .context("write failed")
        }
    }

    fn init_git_repo(forge_config: &[Config]) -> anyhow::Result<temp_dir::TempDir> {
        let dir = temp_dir::TempDir::new()?;
        std::process::Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "init"])
            .status()?;
        let mut file = std::fs::File::create(dir.path().join(".git/config"))?;
        for conf in forge_config {
            conf.print(&mut file)?;
        }
        Ok(dir)
    }

    #[test]
    fn forge_config_reads_from_git_config() -> anyhow::Result<()> {
        let dir = init_git_repo(&[Config {
            forge: "github",
            host: "git.example.org",
            token_command: Some("echo THIS IS A TOKEN"),
        }])?;
        let repo = gix::open(dir.path())?;

        let config = super::ForgeConfig::new(
            &url::Url::from_str("https://git.example.org/repos/project/pulls/21")?,
            &repo,
        );
        assert!(config.is_ok(), "config should not error");

        let token = config.unwrap().token();
        assert!(token.is_ok(), "should get token");
        assert_eq!(token.unwrap(), Some("THIS IS A TOKEN".to_string()));

        Ok(())
    }

    #[test]
    fn forge_config_env_has_priority() -> anyhow::Result<()> {
        let dir = init_git_repo(&[Config {
            forge: "gitlab",
            host: "git.example.org",
            token_command: Some("echo WRONG TOKEN"),
        }])?;
        let repo = gix::open(dir.path())?;

        unsafe {
            std::env::set_var("GITLAB_TOKEN", "USE THIS TOKEN");
        }

        let config = super::ForgeConfig::new(
            &url::Url::from_str("https://git.example.org/repos/project/pulls/21")?,
            &repo,
        );
        assert!(config.is_ok(), "config should not error");

        let token = config.unwrap().token();
        assert!(token.is_ok(), "should get token");
        assert_eq!(token.unwrap(), Some("USE THIS TOKEN".to_string()));

        Ok(())
    }

    #[test]
    fn forge_config_matches_by_url() -> anyhow::Result<()> {
        let dir = init_git_repo(&[
            Config {
                forge: "github",
                host: "github.com",
                token_command: Some("echo NOT THIS ONE"),
            },
            Config {
                forge: "github",
                host: "git.example.org",
                token_command: Some("echo THIS IS THE ONE"),
            },
        ])?;
        let repo = gix::open(dir.path())?;

        let config = super::ForgeConfig::new(
            &url::Url::from_str("https://git.example.org/does/not/exist/pulls/21")?,
            &repo,
        );
        assert!(config.is_ok(), "config should not error");

        let token = config.unwrap().token();
        assert_eq!(token.unwrap(), Some("THIS IS THE ONE".to_string()));
        Ok(())
    }

    #[test]
    fn forge_config_should_error_from_cmd() -> anyhow::Result<()> {
        let dir = init_git_repo(&[Config {
            forge: "bitbucket-datacenter",
            host: "git.example.org",
            token_command: Some("exit 1"),
        }])?;
        let repo = gix::open(dir.path())?;

        let config = super::ForgeConfig::new(
            &url::Url::from_str("https://git.example.org/does/not/exist/pulls/21")?,
            &repo,
        );
        assert!(config.is_ok(), "config should not error");

        let token = config.unwrap().token();
        assert!(token.is_err(), "should not get token");
        Ok(())
    }
}
