use divrem::DivCeil;
use rand::{rngs::SmallRng, RngCore, SeedableRng};
use rayon::prelude::*;

const BLOCK_SIZE: usize = 2 * 1024;

/// A fixture that generates random but reproducible data. Useful for test cases.
/// It allows efficiently jumping around in the data stream and getting later
/// sections of the data without having to generate all the data ahead of it.
pub struct DataFixture {
    seed: u64,
}

impl DataFixture {
    pub fn new(seed: u64) -> Self {
        // Add a seed indirection by running it through the random generator once.
        // This means that consecutive seeds (e.g. 0, 1, 2, 3) will generate entirely
        // different data even though we calculate block seeds by adding the block index
        // to the seed in [Self::generate_block].
        let seed = SmallRng::seed_from_u64(seed).next_u64();
        Self { seed }
    }

    fn generate_block(&self, block_index: usize, in_block_offset: usize, dest: &mut [u8]) {
        assert!(in_block_offset + dest.len() <= BLOCK_SIZE);
        let mut rng = SmallRng::seed_from_u64(self.seed + block_index as u64);
        // We generate a multiple of 64bits because RngCore::fill_bytes() fills the
        // buffer with u32 or u64, not with u8.
        let block_size = DivCeil::div_ceil(in_block_offset + dest.len(), 8) * 8;
        let mut block = vec![0u8; block_size];
        rng.fill_bytes(block.as_mut());
        dest.copy_from_slice(&block[in_block_offset..in_block_offset + dest.len()]);
    }

    pub fn generate(&self, offset: u64, dest: &mut [u8]) {
        // Idea: Split the generated data stream into blocks.
        // Each block is generated by a different RNG seeded with self.seed + block_index.
        // This allows us to efficiently jump around in the data stream and get later
        // sections of the data without having to generate all the data ahead of it.

        if dest.len() == 0 {
            return;
        }

        let block_index_start = (offset / BLOCK_SIZE as u64) as usize;
        let block_index_end =
            DivCeil::div_ceil(offset + dest.len() as u64, BLOCK_SIZE as u64) as usize;

        let dest_len = dest.len();

        let first_block_offset = offset % BLOCK_SIZE as u64;
        let first_block_size = (BLOCK_SIZE as u64 - first_block_offset).min(dest.len() as u64);
        let dest_slices = subslices(dest, first_block_size as usize, BLOCK_SIZE);

        dest_slices
            .into_par_iter()
            .enumerate()
            .for_each(|(relative_block_index, dest)| {
                let block_index = block_index_start + relative_block_index;
                let block_offset = block_index * BLOCK_SIZE;
                let in_block_offset = if block_index == block_index_start {
                    offset - block_offset as u64
                } else {
                    assert!(block_offset as u64 >= offset);
                    0
                } as usize;
                let in_block_end = if block_index == block_index_end - 1 {
                    (offset + dest_len as u64) - block_offset as u64
                } else {
                    assert!(block_offset as u64 + BLOCK_SIZE as u64 <= offset + dest_len as u64);
                    BLOCK_SIZE as u64
                } as usize;
                let in_block_size = in_block_end - in_block_offset;
                assert_eq!(dest.len(), in_block_size);
                self.generate_block(block_index, in_block_offset, dest);
            });
    }

    pub fn get(&self, size: usize) -> Vec<u8> {
        let mut data = vec![0; size];
        self.generate(0, &mut data);
        data
    }
}

/// Splits a mutable slice into mutable subslices of the given `block_size`
/// (except for the first one, which may have a different size).
/// TODO Replace this with https://doc.rust-lang.org/std/primitive.slice.html#method.as_chunks_mut once stable
fn subslices<'a, T>(
    arr: &'a mut [T],
    first_block_size: usize,
    block_size: usize,
) -> Vec<&'a mut [T]> {
    assert!(
        first_block_size <= arr.len(),
        "first_block_size == {first_block_size} > {} == arr.len()",
        arr.len(),
    );
    let (first_block, mut tail) = arr.split_at_mut(first_block_size);
    let num_tail_blocks = DivCeil::div_ceil(tail.len(), block_size);
    let mut blocks = Vec::with_capacity(1 + num_tail_blocks);
    blocks.push(first_block);
    for _ in 0..num_tail_blocks {
        let split_index = block_size.min(tail.len());
        let (block, new_tail) = std::mem::take(&mut tail).split_at_mut(split_index);
        tail = new_tail;
        blocks.push(block);
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    mod generate {
        use super::*;

        #[test]
        fn different_seeds_give_different_data() {
            let fixture1 = DataFixture::new(0);
            let fixture2 = DataFixture::new(1);
            let mut data1 = vec![0; 1024 * 1024];
            let mut data2 = vec![0; 1024 * 1024];
            fixture1.generate(0, &mut data1);
            fixture2.generate(0, &mut data2);
            assert_ne!(data1, data2);
        }

        #[test]
        fn same_seed_gives_same_data() {
            let fixture1 = DataFixture::new(0);
            let fixture2 = DataFixture::new(0);
            let mut data1 = vec![0; 1024 * 1024];
            let mut data2 = vec![0; 1024 * 1024];
            fixture1.generate(0, &mut data1);
            fixture2.generate(0, &mut data2);
            assert_eq!(data1, data2);
        }

        #[test]
        fn empty() {
            let fixture = DataFixture::new(0);
            let mut data = vec![0; 0];
            fixture.generate(0, &mut data);
            assert_eq!(data, vec![]);
        }

        #[test]
        fn one_byte() {
            let fixture = DataFixture::new(0);
            let mut data = vec![0; 1];
            fixture.generate(0, &mut data);
            assert_ne!(0, data[0]);
        }

        #[test]
        fn its_not_just_zeroes() {
            let fixture = DataFixture::new(0);
            let mut data = vec![0; 1024 * 1024];
            fixture.generate(0, &mut data);
            let num_zeroes = data.iter().filter(|&&x| x == 0).count();
            assert_eq!(4059, num_zeroes);
        }

        #[derive(Clone, Copy)]
        struct Params {
            section_size: u64,
            data_size: u64,
        }

        const SMALL_DATA_SIZE: u64 = (BLOCK_SIZE as f64 * 3.2) as u64;
        const DATA_SIZE: u64 = (BLOCK_SIZE as f64 * 142.2) as u64;

        #[rstest]
        #[case::generate_byte_by_byte_1(Params {
            section_size: 1,
            // smaller data size because otherwise test takes too much time
            data_size: SMALL_DATA_SIZE,
        })]
        #[case::generate_byte_by_byte_2(Params {
            section_size: 2,
            // smaller data size because otherwise test takes too much time
            data_size: SMALL_DATA_SIZE,
        })]
        #[case::generate_byte_by_byte_3(Params {
            section_size: 3,
            // smaller data size because otherwise test takes too much time
            data_size: SMALL_DATA_SIZE,
        })]
        #[case::section_smaller_than_block_size_but_unaligned(Params {
            section_size: (BLOCK_SIZE as f64 * 0.9) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_smaller_than_block_size_and_aligned_1(Params {
            section_size: (BLOCK_SIZE / 2) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_smaller_than_block_size_and_aligned_2(Params {
            section_size: (BLOCK_SIZE / 3) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_smaller_than_block_size_and_aligned_3(Params {
            section_size: (BLOCK_SIZE / 4) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_larger_than_block_size_but_unaligned(Params {
            section_size: (BLOCK_SIZE as f64 * 10.4) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_larger_than_block_size_and_aligned_1(Params {
            section_size: (BLOCK_SIZE * 2) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_larger_than_block_size_and_aligned_2(Params {
            section_size: (BLOCK_SIZE * 3) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_larger_than_block_size_and_aligned_3(Params {
            section_size: (BLOCK_SIZE * 4) as u64,
            data_size: DATA_SIZE,
        })]
        #[case::section_size_equal_to_block_size(Params {
            section_size: BLOCK_SIZE as u64,
            data_size: DATA_SIZE,
        })]
        fn generating_data_in_sections_generates_same_data_as_generating_whole_data(
            #[case] params: Params,
        ) {
            let fixture = DataFixture::new(0);
            let mut data1 = vec![0; params.data_size as usize];
            let mut data2 = vec![0; params.data_size as usize];
            fixture.generate(0, &mut data1);

            let mut offset = 0u64;
            while offset < data2.len() as u64 {
                let section_size = params.section_size.min(data2.len() as u64 - offset);
                fixture.generate(
                    offset,
                    &mut data2[offset as usize..(offset + section_size) as usize],
                );
                offset += section_size;
            }
            assert_eq!(data1, data2);
        }
    }

    mod get {
        use super::*;

        #[test]
        fn get_empty() {
            let fixture = DataFixture::new(0);
            let data = fixture.get(0);
            assert_eq!(data, vec![]);
        }

        #[test]
        fn one_byte() {
            let fixture = DataFixture::new(0);
            let data = fixture.get(1);
            assert_eq!(1, data.len());
            assert_ne!(0, data[0]);
        }

        #[test]
        fn get_returns_same_data_as_generate() {
            let fixture = DataFixture::new(0);
            let mut data = vec![0; 1024 * 1024];
            fixture.generate(0, &mut data);
            assert_eq!(data, fixture.get(1024 * 1024));
        }
    }
}