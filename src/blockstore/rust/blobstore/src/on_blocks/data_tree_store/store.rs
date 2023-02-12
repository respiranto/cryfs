use anyhow::Result;
use async_trait::async_trait;
#[cfg(test)]
use futures::TryStreamExt;
#[cfg(test)]
use std::collections::HashSet;

#[cfg(test)]
use crate::on_blocks::data_node_store::DataNode;
use crate::{on_blocks::data_node_store::DataNodeStore, RemoveResult};
use cryfs_blockstore::{BlockId, BlockStore, LockingBlockStore};
use cryfs_utils::{
    async_drop::{AsyncDrop, AsyncDropGuard},
    data::Data,
};

use super::tree::DataTree;

#[derive(Debug)]
pub struct DataTreeStore<B: BlockStore + Send + Sync> {
    node_store: AsyncDropGuard<DataNodeStore<B>>,
}

impl<B: BlockStore + Send + Sync> DataTreeStore<B> {
    pub async fn new(
        block_store: AsyncDropGuard<LockingBlockStore<B>>,
        block_size_bytes: u32,
    ) -> Result<AsyncDropGuard<Self>> {
        Ok(AsyncDropGuard::new(Self {
            node_store: DataNodeStore::new(block_store, block_size_bytes).await?,
        }))
    }
}

impl<B: BlockStore + Send + Sync> DataTreeStore<B> {
    pub async fn load_tree(&self, root_node_id: BlockId) -> Result<Option<DataTree<'_, B>>> {
        Ok(self
            .node_store
            .load(root_node_id)
            .await?
            .map(|root_node| DataTree::new(root_node, &self.node_store)))
    }

    pub async fn create_tree(&self) -> Result<DataTree<'_, B>> {
        let new_leaf = self
            .node_store
            .create_new_leaf_node(&Data::from(vec![]))
            .await?;
        Ok(DataTree::new(new_leaf.upcast(), &self.node_store))
    }

    pub async fn try_create_tree(&self, id: BlockId) -> Result<Option<DataTree<'_, B>>> {
        let new_leaf = self
            .node_store
            .try_create_new_leaf_node(id, &Data::from(vec![]))
            .await?;
        Ok(new_leaf.map(|new_leaf| DataTree::new(new_leaf.upcast(), &self.node_store)))
    }

    pub async fn remove_tree_by_id(&self, root_node_id: BlockId) -> Result<RemoveResult> {
        match self.load_tree(root_node_id).await? {
            Some(tree) => {
                DataTree::remove(tree).await?;
                Ok(RemoveResult::SuccessfullyRemoved)
            }
            None => Ok(RemoveResult::NotRemovedBecauseItDoesntExist),
        }
    }

    pub async fn num_nodes(&self) -> Result<u64> {
        self.node_store.num_nodes().await
    }

    pub fn estimate_space_for_num_blocks_left(&self) -> Result<u64> {
        self.node_store.estimate_space_for_num_blocks_left()
    }

    pub fn virtual_block_size_bytes(&self) -> u32 {
        self.node_store.virtual_block_size_bytes()
    }

    pub async fn load_block_depth(&self, id: &BlockId) -> Result<Option<u8>> {
        Ok(self.node_store.load(*id).await?.map(|node| node.depth()))
    }

    #[cfg(test)]
    // This needs to load all blocks, so it's not very efficient. Only use it for tests.
    pub async fn all_tree_roots(&self) -> Result<Vec<BlockId>> {
        let all_nodes: Vec<BlockId> = self.node_store.all_nodes().await?.try_collect().await?;
        let mut potential_roots: HashSet<BlockId> = all_nodes.iter().copied().collect();

        for node_id in all_nodes {
            match self.node_store.load(node_id).await? {
                Some(DataNode::Leaf(_)) | None => { /* do nothing */ }
                Some(DataNode::Inner(inner)) => {
                    for child_id in inner.children() {
                        potential_roots.remove(&child_id);
                    }
                }
            }
        }

        Ok(potential_roots.into_iter().collect())
    }

    #[cfg(test)]
    pub async fn clear_cache_slow(&self) -> Result<()> {
        self.node_store.clear_cache_slow().await
    }
}

#[async_trait]
impl<B: BlockStore + Send + Sync> AsyncDrop for DataTreeStore<B> {
    type Error = anyhow::Error;

    async fn async_drop_impl(&mut self) -> Result<(), Self::Error> {
        self.node_store.async_drop().await
    }
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]

    use super::super::testutils::*;
    use super::*;

    mod load_tree {
        use super::*;

        #[tokio::test]
        async fn not_existing() {
            with_treestore(|store| {
                Box::pin(async move {
                    let tree = store
                        .load_tree(BlockId::from_hex("d86afd0489d7c3046c446e8ec1a049fe").unwrap())
                        .await
                        .unwrap();
                    assert!(tree.is_none());
                })
            })
            .await;
        }

        #[tokio::test]
        async fn existing_one_leaf_node() {
            with_treestore(|store| {
                Box::pin(async move {
                    let root_id = *store.create_tree().await.unwrap().root_node_id();
                    let tree = store.load_tree(root_id).await.unwrap().unwrap();
                    assert_eq!(root_id, *tree.root_node_id());
                })
            })
            .await;
        }

        #[tokio::test]
        async fn existing_multiple_leaf_nodes() {
            with_treestore(|store| {
                Box::pin(async move {
                    let root_id = {
                        let mut tree = store.create_tree().await.unwrap();
                        tree.resize_num_bytes(10 * PHYSICAL_BLOCK_SIZE_BYTES as u64)
                            .await
                            .unwrap();
                        *tree.root_node_id()
                    };
                    let tree = store.load_tree(root_id).await.unwrap().unwrap();
                    assert_eq!(root_id, *tree.root_node_id());
                })
            })
            .await;
        }
    }

    mod create_tree {
        use super::*;

        #[tokio::test]
        async fn loadable_after_creation() {
            with_treestore(|store| {
                Box::pin(async move {
                    let root_id = *store.create_tree().await.unwrap().root_node_id();
                    let tree = store.load_tree(root_id).await.unwrap().unwrap();
                    assert_eq!(root_id, *tree.root_node_id());
                })
            })
            .await;
        }

        #[tokio::test]
        async fn is_just_one_empty_leaf_node() {
            with_treestore_and_nodestore(|treestore, nodestore| {
                Box::pin(async move {
                    let mut tree = treestore.create_tree().await.unwrap();
                    assert_eq!(tree.num_nodes().await.unwrap(), 1);
                    assert_eq!(tree.num_bytes().await.unwrap(), 0);
                    tree.flush().await.unwrap();

                    let DataNode::Leaf(node) = nodestore.load(*tree.root_node_id()).await.unwrap().unwrap() else {
                        panic!("Expected inner node");
                    };
                    assert_eq!(0, node.num_bytes());
                })
            })
            .await;
        }
    }

    mod try_create_tree {
        use super::*;

        #[tokio::test]
        async fn loadable_after_creation() {
            with_treestore(|store| {
                Box::pin(async move {
                    let root_id = BlockId::from_hex("d86afd0489d7c3046c446e8ec1a049fe").unwrap();
                    assert_eq!(
                        root_id,
                        *store
                            .try_create_tree(root_id)
                            .await
                            .unwrap()
                            .unwrap()
                            .root_node_id()
                    );
                    let tree = store.load_tree(root_id).await.unwrap().unwrap();
                    assert_eq!(root_id, *tree.root_node_id());
                })
            })
            .await;
        }

        #[tokio::test]
        async fn is_just_one_empty_leaf_node() {
            with_treestore_and_nodestore(|treestore, nodestore| {
                Box::pin(async move {
                    let root_id = BlockId::from_hex("d86afd0489d7c3046c446e8ec1a049fe").unwrap();
                    let mut tree = treestore.try_create_tree(root_id).await.unwrap().unwrap();
                    assert_eq!(tree.num_nodes().await.unwrap(), 1);
                    assert_eq!(tree.num_bytes().await.unwrap(), 0);
                    tree.flush().await.unwrap();

                    let DataNode::Leaf(node) = nodestore.load(*tree.root_node_id()).await.unwrap().unwrap() else {
                        panic!("Expected inner node");
                    };
                    assert_eq!(0, node.num_bytes());
                })
            })
            .await;
        }

        #[tokio::test]
        async fn with_already_existing_id() {
            with_treestore(|store| {
                Box::pin(async move {
                    let root_id = BlockId::from_hex("d86afd0489d7c3046c446e8ec1a049fe").unwrap();
                    assert_eq!(
                        root_id,
                        *store
                            .try_create_tree(root_id)
                            .await
                            .unwrap()
                            .unwrap()
                            .root_node_id()
                    );
                    assert!(store.try_create_tree(root_id).await.unwrap().is_none());
                })
            })
            .await;
        }
    }

    mod remove_tree_by_id {
        use super::*;

        #[tokio::test]
        async fn givenEmptyTreeStore_whenRemovingNonExistingEntry_thenFails() {
            with_treestore(move |store| {
                Box::pin(async move {
                    assert_eq!(
                        RemoveResult::NotRemovedBecauseItDoesntExist,
                        store
                            .remove_tree_by_id(
                                BlockId::from_hex("3674b8dc1c3c1c41e331a1ebd4949087").unwrap()
                            )
                            .await
                            .unwrap()
                    );
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenOtherwiseEmptyTreeStore_whenRemovingExistingOneNodeTree_thenCannotBeLoadedAnymore(
        ) {
            with_treestore(move |store| {
                Box::pin(async move {
                    let root_id = *create_one_leaf_tree(&store).await.root_node_id();
                    assert!(store.load_tree(root_id).await.unwrap().is_some());

                    assert_eq!(
                        RemoveResult::SuccessfullyRemoved,
                        store.remove_tree_by_id(root_id).await.unwrap()
                    );
                    assert!(store.load_tree(root_id).await.unwrap().is_none());
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenOtherwiseEmptyTreeStore_whenRemovingExistingMultiNodeTree_thenCannotBeLoadedAnymore(
        ) {
            with_treestore(move |store| {
                Box::pin(async move {
                    const NUM_LEAVES: u64 = 10;
                    let root_id = *create_multi_leaf_tree(&store, NUM_LEAVES)
                        .await
                        .root_node_id();
                    assert!(store.load_tree(root_id).await.unwrap().is_some());

                    assert_eq!(
                        RemoveResult::SuccessfullyRemoved,
                        store.remove_tree_by_id(root_id).await.unwrap()
                    );
                    assert!(store.load_tree(root_id).await.unwrap().is_none());
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenOtherwiseEmptyTreeStore_whenRemovingExistingMultiNodeTree_thenDeletesAllNodesOfThisTree(
        ) {
            with_treestore_and_nodestore(move |treestore, nodestore| {
                Box::pin(async move {
                    const NUM_LEAVES: u64 = 10;
                    let root_id = *create_multi_leaf_tree(&treestore, NUM_LEAVES)
                        .await
                        .root_node_id();
                    treestore.clear_cache_slow().await.unwrap();
                    assert_eq!(NUM_LEAVES + 1, nodestore.num_nodes().await.unwrap());

                    assert_eq!(
                        RemoveResult::SuccessfullyRemoved,
                        treestore.remove_tree_by_id(root_id).await.unwrap()
                    );
                    treestore.clear_cache_slow().await.unwrap();
                    assert_eq!(0, nodestore.num_nodes().await.unwrap());
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenTreeStoreWithOtherTrees_whenRemovingNonExistingEntry_thenFails() {
            with_treestore(move |store| {
                Box::pin(async move {
                    let _other_tree = TreeFixture::create_tree_with_data_and_id(
                        &store,
                        BlockId::from_hex("41e331a31c3c1c1ebd4949087674b8dc").unwrap(),
                        10 * store.virtual_block_size_bytes() as usize,
                        0,
                    )
                    .await;

                    assert_eq!(
                        RemoveResult::NotRemovedBecauseItDoesntExist,
                        store
                            .remove_tree_by_id(
                                BlockId::from_hex("3674b8dc1c3c1c41e331a1ebd4949087").unwrap()
                            )
                            .await
                            .unwrap()
                    );
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenTreeStoreWithOtherTrees_whenRemovingExistingOneNodeTree_thenCannotBeLoadedAnymore(
        ) {
            with_treestore(move |store| {
                Box::pin(async move {
                    let _other_tree = TreeFixture::create_tree_with_data(
                        &store,
                        10 * store.virtual_block_size_bytes() as usize,
                        0,
                    )
                    .await;
                    let root_id = *create_one_leaf_tree(&store).await.root_node_id();
                    assert!(store.load_tree(root_id).await.unwrap().is_some());

                    assert_eq!(
                        RemoveResult::SuccessfullyRemoved,
                        store.remove_tree_by_id(root_id).await.unwrap()
                    );
                    assert!(store.load_tree(root_id).await.unwrap().is_none());
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenTreeStoreWithOtherTrees_whenRemovingExistingMultiNodeTree_thenCannotBeLoadedAnymore(
        ) {
            with_treestore(move |store| {
                Box::pin(async move {
                    const NUM_LEAVES: u64 = 10;

                    let _other_tree = TreeFixture::create_tree_with_data(
                        &store,
                        NUM_LEAVES as usize * store.virtual_block_size_bytes() as usize,
                        0,
                    )
                    .await;

                    let root_id = *create_multi_leaf_tree(&store, NUM_LEAVES)
                        .await
                        .root_node_id();
                    assert!(store.load_tree(root_id).await.unwrap().is_some());

                    assert_eq!(
                        RemoveResult::SuccessfullyRemoved,
                        store.remove_tree_by_id(root_id).await.unwrap()
                    );
                    assert!(store.load_tree(root_id).await.unwrap().is_none());
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenTreeStoreWithOtherTrees_whenRemovingExistingMultiNodeTree_thenDeletesAllNodesOfThisTree(
        ) {
            with_treestore_and_nodestore(move |treestore, nodestore| {
                Box::pin(async move {
                    const NUM_LEAVES: u64 = 10;

                    let _other_tree = TreeFixture::create_tree_with_data(
                        &treestore,
                        NUM_LEAVES as usize * treestore.virtual_block_size_bytes() as usize,
                        0,
                    )
                    .await;

                    let root_id = *create_multi_leaf_tree(&treestore, NUM_LEAVES)
                        .await
                        .root_node_id();
                    treestore.clear_cache_slow().await.unwrap();
                    assert_eq!(2 * NUM_LEAVES + 2, nodestore.num_nodes().await.unwrap());

                    assert_eq!(
                        RemoveResult::SuccessfullyRemoved,
                        treestore.remove_tree_by_id(root_id).await.unwrap()
                    );
                    treestore.clear_cache_slow().await.unwrap();
                    assert_eq!(NUM_LEAVES + 1, nodestore.num_nodes().await.unwrap());
                })
            })
            .await
        }

        #[tokio::test]
        async fn givenTreeStoreWithOtherTrees_whenRemovingExistingMultiNodeTree_thenDoesntDeleteOtherTrees(
        ) {
            with_treestore_and_nodestore(move |treestore, nodestore| {
                Box::pin(async move {
                    const NUM_LEAVES: u64 = 10;

                    let other_tree = TreeFixture::create_tree_with_data(
                        &treestore,
                        NUM_LEAVES as usize * treestore.virtual_block_size_bytes() as usize,
                        0,
                    )
                    .await;

                    let root_id = *create_multi_leaf_tree(&treestore, NUM_LEAVES)
                        .await
                        .root_node_id();
                    treestore.clear_cache_slow().await.unwrap();
                    assert_eq!(2 * NUM_LEAVES + 2, nodestore.num_nodes().await.unwrap());

                    assert_eq!(
                        RemoveResult::SuccessfullyRemoved,
                        treestore.remove_tree_by_id(root_id).await.unwrap()
                    );
                    treestore.clear_cache_slow().await.unwrap();
                    assert_eq!(NUM_LEAVES + 1, nodestore.num_nodes().await.unwrap());

                    other_tree.assert_data_is_still_intact(&treestore).await;
                })
            })
            .await
        }
    }

    // TODO Test num_nodes
    // TODO Test estimate_space_for_num_blocks_left
    // TODO Test virtual_block_size_bytes
    // TODO Test load_block_depth
}
