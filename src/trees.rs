use std::fmt::Display;

use crate::error::{CustomError, Result};
use error_stack::ResultExt;
use jj_lib::{commit::Commit, merged_tree::MergedTree, repo::Repo, rewrite::rebase_to_dest_parent};

#[derive(Debug)]
pub enum DiffTree<'a> {
    Interdiff { from: &'a Commit, to: &'a Commit },
    AddedCommit { commit: &'a Commit },
    RemovedCommit { commit: &'a Commit },
}

impl DiffTree<'_> {
    pub fn from<'a>(from: Option<&'a Commit>, to: Option<&'a Commit>) -> Option<DiffTree<'a>> {
        match (from, to) {
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

impl DiffTree<'_> {
    pub fn get_trees(&self, repo: &dyn Repo) -> Result<(MergedTree, MergedTree)> {
        match self {
            Self::Interdiff { from, to } => {
                let from_tree = rebase_to_dest_parent(repo, std::slice::from_ref(from), to)
                    .change_context(CustomError::RepoError)?;
                let to_tree = to.tree();

                Ok((from_tree, to_tree))
            }
            Self::AddedCommit { commit } => {
                let from_tree = commit
                    .parent_tree(repo)
                    .change_context(CustomError::RepoError)?;
                let to_tree = commit.tree();

                Ok((from_tree, to_tree))
            }
            Self::RemovedCommit { commit } => {
                let from_tree = commit.tree();
                let to_tree = commit
                    .parent_tree(repo)
                    .change_context(CustomError::RepoError)?;

                Ok((from_tree, to_tree))
            }
        }
    }
}
