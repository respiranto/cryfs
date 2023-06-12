use async_trait::async_trait;
use cryfs_rustfs::{
    object_based_api::Device, AbsolutePath, FsError, FsResult, Gid, Mode, Statfs, Uid,
};
use std::sync::{Arc, Mutex};

use super::dir::{DirInode, InMemoryDirRef};
use super::file::{InMemoryFileRef, InMemoryOpenFileRef};
use super::node::InMemoryNodeRef;
use super::symlink::InMemorySymlinkRef;

pub struct RootDir {
    // We're pointing directly to the [DirInode] instead of using an [InMemoryDirRef] to avoid
    // reference cycles. [InMemoryDirRef] has a reference back to [RootDir].
    rootdir: Arc<Mutex<DirInode>>,
}

impl RootDir {
    pub fn new(uid: Uid, gid: Gid) -> Arc<Self> {
        let mode = Mode::default()
            .add_dir_flag()
            .add_user_read_flag()
            .add_user_write_flag()
            .add_user_exec_flag();
        Arc::new(Self {
            rootdir: Arc::new(Mutex::new(DirInode::new(mode, uid, gid))),
        })
    }

    pub fn load_node(self: &Arc<Self>, path: &AbsolutePath) -> FsResult<InMemoryNodeRef> {
        let mut current_node = InMemoryNodeRef::Dir(InMemoryDirRef::from_inode(
            Arc::downgrade(&self),
            Arc::clone(&self.rootdir),
        ));
        for component in path.iter() {
            match &current_node {
                InMemoryNodeRef::Dir(dir) => {
                    current_node = dir.get_child(&component)?;
                }
                InMemoryNodeRef::Symlink(_) | InMemoryNodeRef::File(_) => {
                    return Err(FsError::NodeIsNotADirectory);
                }
            }
        }
        Ok(current_node)
    }

    pub fn load_dir(self: &Arc<Self>, path: &AbsolutePath) -> FsResult<InMemoryDirRef> {
        let node = self.load_node(path)?;
        match node {
            InMemoryNodeRef::Dir(dir) => Ok(dir),
            _ => Err(FsError::NodeIsNotADirectory),
        }
    }

    pub fn load_symlink(self: &Arc<Self>, path: &AbsolutePath) -> FsResult<InMemorySymlinkRef> {
        let node = self.load_node(path)?;
        match node {
            InMemoryNodeRef::Symlink(symlink) => Ok(symlink),
            _ => Err(FsError::NodeIsNotASymlink),
        }
    }

    pub fn load_file(self: &Arc<Self>, path: &AbsolutePath) -> FsResult<InMemoryFileRef> {
        let node = self.load_node(path)?;
        match node {
            InMemoryNodeRef::File(file) => Ok(file),
            InMemoryNodeRef::Dir(_) => Err(FsError::NodeIsADirectory),
            InMemoryNodeRef::Symlink(_) => {
                // TODO What's the right error here?
                Err(FsError::UnknownError)
            }
        }
    }
}

pub struct InMemoryDevice {
    rootdir: Arc<RootDir>,
}

impl InMemoryDevice {
    pub fn new(uid: Uid, gid: Gid) -> Self {
        Self {
            rootdir: RootDir::new(uid, gid),
        }
    }
}

#[async_trait]
impl Device for InMemoryDevice {
    type Node<'a> = InMemoryNodeRef;
    type Dir<'a> = InMemoryDirRef;
    type Symlink<'a> = InMemorySymlinkRef;
    type File<'a> = InMemoryFileRef;
    type OpenFile = InMemoryOpenFileRef;

    async fn load_node(&self, path: &AbsolutePath) -> FsResult<Self::Node<'_>> {
        self.rootdir.load_node(path)
    }

    async fn load_dir(&self, path: &AbsolutePath) -> FsResult<Self::Dir<'_>> {
        self.rootdir.load_dir(path)
    }

    async fn load_symlink(&self, path: &AbsolutePath) -> FsResult<Self::Symlink<'_>> {
        self.rootdir.load_symlink(path)
    }

    async fn load_file(&self, path: &AbsolutePath) -> FsResult<Self::File<'_>> {
        self.rootdir.load_file(path)
    }

    async fn statfs(&self) -> FsResult<Statfs> {
        todo!()
    }

    async fn destroy(self) {
        // Nothing to do
    }
}
