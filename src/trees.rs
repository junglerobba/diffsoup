use std::{fmt::Display, path::Path};

use crate::error::{CustomError, Result};
use error_stack::ResultExt;
use jj_lib::{
    backend::{CopyId, TreeValue},
    commit::Commit,
    merge::Merge,
    merged_tree::{MergedTree, MergedTreeBuilder},
    repo::Repo,
    repo_path::RepoPathBuf,
    rewrite::rebase_to_dest_parent,
};

#[derive(Debug)]
pub enum DiffTree<'a> {
    Interdiff { from: &'a Commit, to: &'a Commit },
    AddedCommit { commit: &'a Commit },
    RemovedCommit { commit: &'a Commit },
}

impl DiffTree<'_> {
    pub fn from<'a>(from: Option<&'a Commit>, to: Option<&'a Commit>) -> Option<DiffTree<'a>> {
        match (from, to) {
            (Some(from), Some(to)) if from.id() == to.id() => {
                Some(DiffTree::AddedCommit { commit: to })
            }
            (Some(from), Some(to)) => Some(DiffTree::Interdiff { from, to }),
            (Some(commit), None) => Some(DiffTree::RemovedCommit { commit }),
            (None, Some(commit)) => Some(DiffTree::AddedCommit { commit }),
            (None, None) => None,
        }
    }
}

impl Display for DiffTree<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Interdiff { from, to } => write!(f, "{} -> {}", from.id(), to.id()),
            Self::AddedCommit { commit } => write!(f, "{} (new)", commit.id()),
            Self::RemovedCommit { commit } => write!(f, "{} (removed)", commit.id()),
        }
    }
}

fn write_virtual_tree(description: &str, tree: &MergedTree, repo: &dyn Repo) -> Result<MergedTree> {
    const COMMIT_DESCRIPTION_PATH: &str = ".__COMMIT_MESSAGE__";
    let path = RepoPathBuf::from_relative_path(Path::new(COMMIT_DESCRIPTION_PATH))
        .change_context(CustomError::RepoError)?;
    let blob_id =
        futures::executor::block_on(repo.store().write_file(&path, &mut description.as_bytes()))
            .change_context(CustomError::ProcessError(
                "failed to block on store write".to_string(),
            ))?;

    let mut virtual_tree = MergedTreeBuilder::new(tree.clone());
    virtual_tree.set_or_remove(
        path,
        Merge::normal(TreeValue::File {
            id: blob_id,
            executable: false,
            copy_id: CopyId::placeholder(),
        }),
    );
    virtual_tree
        .write_tree()
        .change_context(CustomError::RepoError)
}

impl DiffTree<'_> {
    pub fn get_trees(&self, repo: &dyn Repo) -> Result<(MergedTree, MergedTree)> {
        match self {
            Self::Interdiff { from, to } => {
                let rebased = rebase_to_dest_parent(repo, std::slice::from_ref(from), to)
                    .change_context(CustomError::RepoError)?;
                let (from_tree, to_tree) = if from.description() == to.description() {
                    (rebased, to.tree())
                } else {
                    (
                        write_virtual_tree(from.description(), &rebased, repo)?,
                        write_virtual_tree(to.description(), &to.tree(), repo)?,
                    )
                };

                Ok((from_tree, to_tree))
            }
            Self::AddedCommit { commit } => {
                let from_tree = commit
                    .parent_tree(repo)
                    .change_context(CustomError::RepoError)?;
                let to_tree = write_virtual_tree(commit.description(), &commit.tree(), repo)?;

                Ok((from_tree, to_tree))
            }
            Self::RemovedCommit { commit } => {
                let from_tree = write_virtual_tree(commit.description(), &commit.tree(), repo)?;
                let to_tree = commit
                    .parent_tree(repo)
                    .change_context(CustomError::RepoError)?;

                Ok((from_tree, to_tree))
            }
        }
    }
}
