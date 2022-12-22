#include <gtest/gtest.h>

#include "blobstore/implementations/onblocks/datanodestore/DataInnerNode.h"
#include "blobstore/implementations/onblocks/datanodestore/DataLeafNode.h"
#include "blobstore/implementations/onblocks/datanodestore/DataNodeStore.h"

#include <blockstore/implementations/testfake/FakeBlockStore.h>
#include <blockstore/implementations/testfake/FakeBlock.h>

#include <memory>
#include <cpp-utils/pointer/cast.h>

using ::testing::Test;

using cpputils::dynamic_pointer_move;

using blockstore::BlockId;
using blockstore::BlockStore;
using blockstore::testfake::FakeBlockStore;
using cpputils::Data;
using namespace blobstore;
using namespace blobstore::onblocks;
using namespace blobstore::onblocks::datanodestore;

using cpputils::make_unique_ref;
using cpputils::unique_ref;
using std::vector;

class DataInnerNodeTest : public Test
{
public:
  static constexpr uint32_t BLOCKSIZE_BYTES = 1024;

  DataInnerNodeTest() : _blockStore(make_unique_ref<FakeBlockStore>()),
                        blockStore(_blockStore.get()),
                        nodeStore(make_unique_ref<DataNodeStore>(std::move(_blockStore), BLOCKSIZE_BYTES)),
                        ZEROES(nodeStore->layout().maxBytesPerLeaf()),
                        leaf(nodeStore->createNewLeafNode(Data(0))),
                        node(nodeStore->createNewInnerNode(1, {leaf->blockId()}))
  {

    ZEROES.FillWithZeroes();
  }

  unique_ref<DataInnerNode> LoadInnerNode(const BlockId &blockId)
  {
    auto node = nodeStore->load(blockId).value();
    return dynamic_pointer_move<DataInnerNode>(node).value();
  }

  BlockId CreateNewInnerNodeReturnKey(const DataNode &firstChild)
  {
    return nodeStore->createNewInnerNode(firstChild.depth() + 1, {firstChild.blockId()})->blockId();
  }

  unique_ref<DataInnerNode> CreateNewInnerNode()
  {
    auto new_leaf = nodeStore->createNewLeafNode(Data(0));
    return nodeStore->createNewInnerNode(1, {new_leaf->blockId()});
  }

  unique_ref<DataInnerNode> CreateAndLoadNewInnerNode(const DataNode &firstChild)
  {
    auto blockId = CreateNewInnerNodeReturnKey(firstChild);
    return LoadInnerNode(blockId);
  }

  unique_ref<DataInnerNode> CreateNewInnerNode(uint8_t depth, const vector<blockstore::BlockId> &children)
  {
    return nodeStore->createNewInnerNode(depth, children);
  }

  BlockId CreateNewInnerNodeReturnKey(uint8_t depth, const vector<blockstore::BlockId> &children)
  {
    return CreateNewInnerNode(depth, children)->blockId();
  }

  unique_ref<DataInnerNode> CreateAndLoadNewInnerNode(uint8_t depth, const vector<blockstore::BlockId> &children)
  {
    auto blockId = CreateNewInnerNodeReturnKey(depth, children);
    return LoadInnerNode(blockId);
  }

  BlockId AddALeafTo(DataInnerNode *node)
  {
    auto leaf2 = nodeStore->createNewLeafNode(Data(0));
    node->addChild(*leaf2);
    return leaf2->blockId();
  }

  BlockId CreateNodeWithDataConvertItToInnerNodeAndReturnKey()
  {
    auto node = CreateNewInnerNode();
    AddALeafTo(node.get());
    AddALeafTo(node.get());
    auto child = nodeStore->createNewLeafNode(Data(0));
    unique_ref<DataInnerNode> converted = DataNode::convertToNewInnerNode(std::move(node), nodeStore->layout(), *child);
    return converted->blockId();
  }

  unique_ref<DataInnerNode> CopyInnerNode(const DataInnerNode &node)
  {
    auto copied = nodeStore->createNewNodeAsCopyFrom(node);
    return dynamic_pointer_move<DataInnerNode>(copied).value();
  }

  BlockId InitializeInnerNodeAddLeafReturnKey()
  {
    auto node = DataInnerNode::CreateNewNode(blockStore, nodeStore->layout(), 1, {leaf->blockId()});
    AddALeafTo(node.get());
    return node->blockId();
  }

  unique_ref<BlockStore> _blockStore;
  BlockStore *blockStore;
  unique_ref<DataNodeStore> nodeStore;
  Data ZEROES;
  unique_ref<DataLeafNode> leaf;
  unique_ref<DataInnerNode> node;

private:
  DISALLOW_COPY_AND_ASSIGN(DataInnerNodeTest);
};

constexpr uint32_t DataInnerNodeTest::BLOCKSIZE_BYTES;

TEST_F(DataInnerNodeTest, LastChildWhenOneChild)
{
  EXPECT_EQ(leaf->blockId(), node->readLastChild().blockId());
}

TEST_F(DataInnerNodeTest, LastChildWhenTwoChildren)
{
  BlockId blockId = AddALeafTo(node.get());
  EXPECT_EQ(blockId, node->readLastChild().blockId());
}

TEST_F(DataInnerNodeTest, LastChildWhenThreeChildren)
{
  AddALeafTo(node.get());
  BlockId blockId = AddALeafTo(node.get());
  EXPECT_EQ(blockId, node->readLastChild().blockId());
}
