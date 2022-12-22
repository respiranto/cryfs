use anyhow::{ensure, Result};
use binary_layout::Field;

use super::super::{
    layout::{node, NodeLayout, FORMAT_VERSION_HEADER},
    DataNode,
};
use cryfs_blockstore::{Block, BlockId, BlockStore};
use cryfs_utils::data::Data;

pub struct DataLeafNode<B: BlockStore + Send + Sync> {
    block: Block<B>,
}

impl<B: BlockStore + Send + Sync> DataLeafNode<B> {
    pub fn new(block: Block<B>, layout: &NodeLayout) -> Result<Self> {
        let view = node::View::new(block.data());
        ensure!(
            view.format_version_header().read() == FORMAT_VERSION_HEADER,
            "Loaded a node with format version {} but the current version is {}",
            view.format_version_header().read(),
            FORMAT_VERSION_HEADER,
        );
        assert_eq!(
            0, view.depth().read(),
            "Loaded a leaf with depth {}. This doesn't make sense, it should have been loaded as an inner node",
            view.depth().read(),
        );
        assert!(block.data().len() > node::data::OFFSET, "Block doesn't have enough space for header. This should have been checked before calling DataLeafNode::new");
        let max_bytes_per_leaf = layout.max_bytes_per_leaf();
        let size = view.size().read();
        ensure!(
            size <= max_bytes_per_leaf,
            "Loaded a leaf that claims to store {} bytes but the maximum is {}.",
            size,
            max_bytes_per_leaf,
        );
        Ok(Self { block })
    }

    pub fn block_id(&self) -> &BlockId {
        self.block.block_id()
    }

    pub(super) fn raw_blockdata(&self) -> &Data {
        self.block.data()
    }

    pub(super) fn into_block(self) -> Block<B> {
        self.block
    }

    pub(super) fn as_block_mut(&mut self) -> &mut Block<B> {
        &mut self.block
    }

    pub fn num_bytes(&self) -> u32 {
        let view = node::View::new(self.block.data());
        view.size().read()
    }

    pub fn max_bytes_per_leaf(&self) -> u32 {
        NodeLayout {
            block_size_bytes: u32::try_from(self.block.data().len()).unwrap(),
        }
        .max_bytes_per_leaf()
    }

    pub fn resize(&mut self, new_num_bytes: u32) {
        assert!(
            new_num_bytes <= self.max_bytes_per_leaf(),
            "Trying to resize to {} bytes which is larger than the maximal size of {}",
            new_num_bytes,
            self.max_bytes_per_leaf()
        );
        let mut view = node::View::new(self.block.data_mut());
        let old_num_bytes = view.size().read();
        if new_num_bytes < old_num_bytes {
            let newly_unused_data_region = &mut view.data_mut()
                [usize::try_from(new_num_bytes).unwrap()..usize::try_from(old_num_bytes).unwrap()];
            newly_unused_data_region.fill(0);
        }
        view.size_mut().write(new_num_bytes);
    }

    pub fn data(&self) -> &[u8] {
        let view = node::View::new(self.block.data().as_ref());
        let leaf_size = view.size().read() as usize;
        let leaf_data = view.into_data().into_slice();
        &leaf_data[..leaf_size]
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        let view = node::View::new(self.block.data_mut().as_mut());
        let leaf_size = view.size().read() as usize;
        let leaf_data = view.into_data().into_slice();
        &mut leaf_data[..leaf_size]
    }

    pub fn upcast(self) -> DataNode<B> {
        DataNode::Leaf(self)
    }
}

// `data` must be the size of the full leaf, even if the leaf uses fewer bytes. `num_bytes` can be used for that.
pub fn serialize_leaf_node_optimized(mut data: Data, num_bytes: u32, layout: &NodeLayout) -> Data {
    assert_eq!(
        usize::try_from(layout.max_bytes_per_leaf()).unwrap(),
        data.len()
    );
    assert!(
        num_bytes <= layout.max_bytes_per_leaf(),
        "Tried to create leaf with {} bytes but each leaf can only hold {}",
        num_bytes,
        layout.max_bytes_per_leaf()
    );
    // TODO assert that data[num_bytes..] is zeroed out
    assert!(data.available_prefix_bytes() >= node::data::OFFSET, "Data objects passed to serialize_leaf_node must have at least {} prefix bytes available, but only had {}", node::data::OFFSET, data.available_prefix_bytes());
    data.grow_region_fail_if_reallocation_necessary(node::data::OFFSET, 0)
        .expect("Not enough prefix bytes available for data object passed to serialize_leaf_node");
    let mut view = node::View::new(&mut data);
    view.format_version_header_mut()
        .write(FORMAT_VERSION_HEADER);
    view.unused_mut().write(0);
    view.depth_mut().write(0);
    view.size_mut().write(num_bytes);
    // view.data is already set correctly because we grew this view from the data input
    data
}

#[cfg(test)]
mod tests {
    use super::super::super::testutils::*;
    use super::*;
    use rand::{rngs::SmallRng, Rng, SeedableRng};

    mod serialize_leaf_node {
        use super::*;

        #[test]
        fn test_serialize_leaf_node_optimized() {
            let layout = NodeLayout {
                block_size_bytes: PHYSICAL_BLOCK_SIZE_BYTES,
            };
            const SIZE: usize = 10;
            let mut data: Data = vec![0; PHYSICAL_BLOCK_SIZE_BYTES as usize].into();
            data.shrink_to_subregion(
                ((PHYSICAL_BLOCK_SIZE_BYTES - layout.max_bytes_per_leaf()) as usize)..,
            );
            SmallRng::seed_from_u64(0).fill(&mut data[..SIZE]);
            let serialized = serialize_leaf_node_optimized(data.clone(), SIZE as u32, &layout);
            let view = node::View::new(serialized.as_ref());
            assert_eq!(view.format_version_header().read(), FORMAT_VERSION_HEADER);
            assert_eq!(view.unused().read(), 0);
            assert_eq!(view.depth().read(), 0);
            assert_eq!(view.size().read(), SIZE as u32);
            assert_eq!(data.as_ref(), view.data());
        }
    }

    mod block_id {
        use super::*;

        #[tokio::test]
        async fn loaded_node_returns_correct_key() {
            with_nodestore(|nodestore| {
                Box::pin(async move {
                    let block_id = *new_full_leaf_node(nodestore).await.block_id();

                    let loaded = load_leaf_node(nodestore, block_id).await;
                    assert_eq!(block_id, *loaded.block_id());
                })
            })
            .await;
        }
    }

    mod resize {
        use super::*;

        // TODO Make const instead of fn
        #[allow(non_snake_case)]
        fn LEAF_SIZES() -> Vec<u32> {
            vec![
                0,
                1,
                5,
                16,
                32,
                512,
                NodeLayout {
                    block_size_bytes: PHYSICAL_BLOCK_SIZE_BYTES,
                }
                .max_bytes_per_leaf(),
            ]
        }

        #[tokio::test]
        async fn has_new_size() {
            async fn test(leaf_size: u32) {
                with_nodestore(|nodestore| {
                    Box::pin(async move {
                        let mut leaf = nodestore
                            .create_new_leaf_node(&data_fixture(100, 1))
                            .await
                            .unwrap();

                        leaf.resize(leaf_size);
                        assert_eq!(leaf_size, leaf.num_bytes());
                        assert_eq!(leaf_size as usize, leaf.data().len());

                        // Check the size is still correct after reloading it
                        let block_id = *leaf.block_id();
                        drop(leaf);
                        let leaf = load_leaf_node(nodestore, block_id).await;
                        assert_eq!(leaf_size, leaf.num_bytes());
                        assert_eq!(leaf_size as usize, leaf.data().len());
                    })
                })
                .await
            }

            for leaf_size in LEAF_SIZES() {
                test(leaf_size).await;
            }
        }

        #[tokio::test]
        async fn growing() {
            with_nodestore(|nodestore| {
                Box::pin(async move {
                    let mut leaf = nodestore
                        .create_new_leaf_node(&data_fixture(100, 1))
                        .await
                        .unwrap();

                    leaf.resize(200);
                    // Old data is still intact
                    assert_eq!(data_fixture(100, 1).as_ref(), &leaf.data()[0..100]);
                    // New data is zeroed out
                    assert_eq!(&[0; 100], &leaf.data()[100..200]);
                })
            })
            .await;
        }

        #[tokio::test]
        async fn shrinking_and_growing() {
            with_nodestore(|nodestore| {
                Box::pin(async move {
                    let mut leaf = nodestore
                        .create_new_leaf_node(&full_leaf_data(1))
                        .await
                        .unwrap();

                    assert_eq!(full_leaf_data(1)[0..200], leaf.data()[0..200]);
                    leaf.resize(100);
                    leaf.resize(200);
                    // Never-touched data is still intact
                    assert_eq!(full_leaf_data(1)[0..100], leaf.data()[0..100]);
                    // Briefly shrunken area is zeroed out
                    assert_eq!(&[0; 100], &leaf.data()[100..200]);
                })
            })
            .await;
        }

        #[tokio::test]
        async fn shrinking() {
            with_nodestore(|nodestore| {
                Box::pin(async move {
                    let mut leaf = nodestore
                        .create_new_leaf_node(&full_leaf_data(1))
                        .await
                        .unwrap();

                    const HEADER_LEN: usize = node::data::OFFSET;
                    assert_eq!(
                        full_leaf_data(1)[0..200],
                        leaf.raw_blockdata()[HEADER_LEN..][0..200]
                    );
                    leaf.resize(100);
                    // Still-in-range data is still intact
                    assert_eq!(
                        full_leaf_data(1)[0..100],
                        leaf.raw_blockdata()[HEADER_LEN..][0..100]
                    );
                    // Out-of-range data is zeroed out
                    assert_eq!(
                        &vec![0; nodestore.layout().max_bytes_per_leaf() as usize - 100],
                        &leaf.raw_blockdata()[HEADER_LEN..][100..]
                    );
                })
            })
            .await;
        }
    }

    mod data_and_data_mut {
        use super::*;

        #[tokio::test]
        async fn empty_leaf_is_empty() {
            with_nodestore(|nodestore| {
                Box::pin(async move {
                    let mut leaf = nodestore
                        .create_new_leaf_node(&vec![0u8; 0].into())
                        .await
                        .unwrap();

                    assert_eq!(&[0u8; 0], leaf.data());
                    assert_eq!(&[0u8; 0], leaf.data_mut());

                    // Still empty after loading
                    let block_id = *leaf.block_id();
                    drop(leaf);
                    let mut leaf = load_leaf_node(nodestore, block_id).await;
                    assert_eq!(&[0u8; 0], leaf.data());
                    assert_eq!(&[0u8; 0], leaf.data_mut());
                })
            })
            .await;
        }

        #[tokio::test]
        async fn after_resizing_has_new_size_and_is_zeroed_out() {
            with_nodestore(|nodestore| {
                Box::pin(async move {
                    let mut leaf = nodestore
                        .create_new_leaf_node(&vec![0u8; 0].into())
                        .await
                        .unwrap();
                    leaf.resize(100);

                    assert_eq!(&[0u8; 100], leaf.data());
                    assert_eq!(&[0u8; 100], leaf.data_mut());

                    // Still correct after loading
                    let block_id = *leaf.block_id();
                    drop(leaf);
                    let mut leaf = load_leaf_node(nodestore, block_id).await;
                    assert_eq!(&[0u8; 100], leaf.data());
                    assert_eq!(&[0u8; 100], leaf.data_mut());
                })
            })
            .await;
        }

        #[tokio::test]
        async fn after_writing_has_new_data() {
            with_nodestore(|nodestore| {
                Box::pin(async move {
                    let mut leaf = nodestore
                        .create_new_leaf_node(&vec![0u8; 0].into())
                        .await
                        .unwrap();
                    leaf.resize(100);

                    leaf.data_mut().copy_from_slice(&data_fixture(100, 1));

                    assert_eq!(data_fixture(100, 1).as_ref(), leaf.data());
                    assert_eq!(data_fixture(100, 1).as_ref(), leaf.data_mut());

                    // Still correct after loading
                    let block_id = *leaf.block_id();
                    drop(leaf);
                    let mut leaf = load_leaf_node(nodestore, block_id).await;
                    assert_eq!(data_fixture(100, 1).as_ref(), leaf.data());
                    assert_eq!(data_fixture(100, 1).as_ref(), leaf.data_mut());
                })
            })
            .await;
        }
    }

    // TODO Test
    //  - new
    //  - into_block
    //  - as_block_mut
    //  - max_bytes_per_leaf
    //  - upcast
}