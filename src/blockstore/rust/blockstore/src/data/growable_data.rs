use anyhow::{ensure, Error, Result};
use std::convert::TryFrom;
use std::marker::PhantomData;
use std::ops::Add;
use std::ops::Sub;
use std::ops::{Deref, DerefMut};
use typenum::Unsigned;

use super::data::Data;

/// This is similar to [Data], but it remembers at compile time (in const generic arguments)
/// how much prefix bytes and suffix bytes are available. This means [GrowableData::grow_region]
/// will know at compile time if it succeeds and this can be used to write safe APIs that require
/// data types with a certain number of prefix or suffix bytes and will check that invariant at compile time.
pub struct GrowableData<PrefixBytes: Unsigned, SuffixBytes: Unsigned> {
    data: Data,
    _p: PhantomData<PrefixBytes>,
    _s: PhantomData<SuffixBytes>,
}

impl<PrefixBytes: Unsigned, SuffixBytes: Unsigned> GrowableData<PrefixBytes, SuffixBytes> {
    pub const fn available_prefix_bytes(&self) -> usize {
        PrefixBytes::USIZE
    }

    pub const fn available_suffix_bytes(&self) -> usize {
        SuffixBytes::USIZE
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    fn _check_invariant(&self) {
        assert!(self.data.available_prefix_bytes() >= PrefixBytes::USIZE);
        assert!(self.data.available_suffix_bytes() >= SuffixBytes::USIZE);
    }

    pub fn into_subregion<DeleteNumBytesAtBeginning: Unsigned, DeleteNumBytesAtEnd: Unsigned>(
        mut self,
    ) -> GrowableData<
        <PrefixBytes as Add<DeleteNumBytesAtBeginning>>::Output,
        <SuffixBytes as Add<DeleteNumBytesAtEnd>>::Output,
    >
    where
        PrefixBytes: Add<DeleteNumBytesAtBeginning>,
        <PrefixBytes as Add<DeleteNumBytesAtBeginning>>::Output: Unsigned,
        SuffixBytes: Add<DeleteNumBytesAtEnd>,
        <SuffixBytes as Add<DeleteNumBytesAtEnd>>::Output: Unsigned,
    {
        let len = self.data.len();
        assert!(
            DeleteNumBytesAtBeginning::USIZE + DeleteNumBytesAtEnd::USIZE <= len,
            "Tried to delete {} + {} bytes from a data region with size {}",
            DeleteNumBytesAtBeginning::USIZE,
            DeleteNumBytesAtEnd::USIZE,
            len,
        );
        self.data.shrink_to_subregion(
            DeleteNumBytesAtBeginning::USIZE..(len - DeleteNumBytesAtEnd::USIZE),
        );
        let result = GrowableData {
            data: self.data,
            _p: PhantomData,
            _s: PhantomData,
        };
        result._check_invariant();
        result
    }

    // TODO Test
    pub fn grow_region<AddNumBytesAtBeginning: Unsigned, AddNumBytesAtEnd: Unsigned>(
        mut self,
    ) -> GrowableData<
        <PrefixBytes as Sub<AddNumBytesAtBeginning>>::Output,
        <SuffixBytes as Sub<AddNumBytesAtEnd>>::Output,
    >
    where
        PrefixBytes: Sub<AddNumBytesAtBeginning>,
        <PrefixBytes as Sub<AddNumBytesAtBeginning>>::Output: Unsigned,
        SuffixBytes: Sub<AddNumBytesAtEnd>,
        <SuffixBytes as Sub<AddNumBytesAtEnd>>::Output: Unsigned,
    {
        // const INVARIANT: bool =
        //     GreaterEquals::<{ PREFIX_BYTES }, { ADD_NUM_BYTES_AT_BEGINNING }>::RESULT;
        // static_assertions::const_assert!({ Self::PREFIX_BYTES >= ADD_NUM_BYTES_AT_BEGINNING });
        // static_assertions::const_assert!(Self::SUFFIX_BYTES >= ADD_NUM_BYTES_AT_END);
        self.data.grow_region_fail_if_reallocation_necessary(
            AddNumBytesAtBeginning::USIZE,
            AddNumBytesAtEnd::USIZE,
        ).expect("This can't happen since our type should enforce that there are enough bytes available");
        let result = GrowableData {
            data: self.data,
            _p: PhantomData,
            _s: PhantomData,
        };
        result._check_invariant();
        result
    }

    // TODO Remove
    pub fn extract(self) -> Data {
        self.data
    }
}

impl<PrefixBytes: Unsigned, SuffixBytes: Unsigned> AsRef<[u8]>
    for GrowableData<PrefixBytes, SuffixBytes>
{
    fn as_ref(&self) -> &[u8] {
        self.data.as_ref()
    }
}

impl<PrefixBytes: Unsigned, SuffixBytes: Unsigned> AsMut<[u8]>
    for GrowableData<PrefixBytes, SuffixBytes>
{
    fn as_mut(&mut self) -> &mut [u8] {
        self.data.as_mut()
    }
}

// TODO Test
impl<PrefixBytes: Unsigned, SuffixBytes: Unsigned> Deref
    for GrowableData<PrefixBytes, SuffixBytes>
{
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.as_ref()
    }
}

// TODO Test
impl<PrefixBytes: Unsigned, SuffixBytes: Unsigned> DerefMut
    for GrowableData<PrefixBytes, SuffixBytes>
{
    fn deref_mut(&mut self) -> &mut [u8] {
        self.as_mut()
    }
}

impl From<Vec<u8>> for GrowableData<typenum::U0, typenum::U0> {
    // TODO Test
    fn from(data: Vec<u8>) -> Self {
        Self {
            data: data.into(),
            _p: PhantomData,
            _s: PhantomData,
        }
    }
}

impl<PrefixBytes: Unsigned, SuffixBytes: Unsigned> TryFrom<Data>
    for GrowableData<PrefixBytes, SuffixBytes>
{
    // TODO Custom error type
    type Error = Error;

    // TODO Test
    fn try_from(data: Data) -> Result<Self> {
        ensure!(data.available_prefix_bytes() == PrefixBytes::USIZE, "The given data object has {} prefix bytes available, but we tried to convert it into a GrowableData requiring {} prefix bytes", data.available_prefix_bytes(), PrefixBytes::USIZE);
        ensure!(data.available_suffix_bytes() == SuffixBytes::USIZE, "The given data object has {} suffix bytes available, but we tried to convert it into a GrowableData requiring {} suffix bytes", data.available_suffix_bytes(), SuffixBytes::USIZE);
        Ok(Self {
            data,
            _p: PhantomData,
            _s: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, RngCore, SeedableRng};
    use typenum::{U0, U10, U19, U24, U3, U49, U5, U51, U89, U93};

    fn data_region(size: usize, seed: u64) -> Vec<u8> {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut res = vec![0; size];
        rng.fill_bytes(&mut res);
        res
    }

    #[test]
    fn given_fullrangedata_when_callingasref() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        assert_eq!(data.as_ref(), &data_region(1024, 0));
    }

    #[test]
    fn given_fullrangedata_when_callingasmut() {
        let mut data: GrowableData<U0, U0> = data_region(1024, 0).into();
        assert_eq!(data.as_mut(), &data_region(1024, 0));
    }

    #[test]
    fn given_fullrangedata_when_gettingavailablebytes() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        assert_eq!(0, data.available_prefix_bytes());
        assert_eq!(0, data.available_suffix_bytes());
    }

    #[test]
    fn given_fullsubregiondata_when_callingasref() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U0, U0>();
        assert_eq!(0, subdata.available_prefix_bytes());
        assert_eq!(0, subdata.available_suffix_bytes());

        assert_eq!(subdata.as_ref(), &data_region(1024, 0));
    }

    #[test]
    fn given_fullsubregiondata_when_callingasmut() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let mut subdata = data.into_subregion::<U0, U0>();
        assert_eq!(0, subdata.available_prefix_bytes());
        assert_eq!(0, subdata.available_suffix_bytes());

        assert_eq!(subdata.as_mut(), &data_region(1024, 0));
    }

    #[test]
    fn given_fullsubregiondata_when_gettingavailablebytes() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U0, U0>();
        assert_eq!(0, subdata.available_prefix_bytes());
        assert_eq!(0, subdata.available_suffix_bytes());
    }

    #[test]
    fn given_openendsubregiondata_when_callingasref() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U5, U0>();
        assert_eq!(subdata.as_ref(), &data_region(1024, 0)[5..]);
    }

    #[test]
    fn given_openendsubregiondata_when_callingasmut() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let mut subdata = data.into_subregion::<U5, U0>();
        assert_eq!(subdata.as_mut(), &data_region(1024, 0)[5..]);
    }

    #[test]
    fn given_openendsubregiondata_when_gettingavailablebytes() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U5, U0>();
        assert_eq!(5, subdata.available_prefix_bytes());
        assert_eq!(0, subdata.available_suffix_bytes());
    }

    #[test]
    fn given_openbeginningsubregiondata_when_callingasref() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U0, U24>();
        assert_eq!(subdata.as_ref(), &data_region(1024, 0)[..1000]);
    }

    #[test]
    fn given_openbeginningsubregiondata_when_callingasmut() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let mut subdata = data.into_subregion::<U0, U24>();
        assert_eq!(subdata.as_mut(), &data_region(1024, 0)[..1000]);
    }

    #[test]
    fn given_openbeginningsubregiondata_when_gettingavailablebytes() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U0, U24>();
        assert_eq!(0, subdata.available_prefix_bytes());
        assert_eq!(24, subdata.available_suffix_bytes());
    }

    #[test]
    fn given_subregiondata_when_callingasref() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U5, U24>();
        assert_eq!(subdata.as_ref(), &data_region(1024, 0)[5..1000]);
    }

    #[test]
    fn given_subregiondata_when_callingasmut() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let mut subdata = data.into_subregion::<U5, U24>();
        assert_eq!(subdata.as_mut(), &data_region(1024, 0)[5..1000]);
    }

    #[test]
    fn given_subregiondata_when_gettingavailablebytes() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data.into_subregion::<U5, U24>();
        assert_eq!(5, subdata.available_prefix_bytes());
        assert_eq!(24, subdata.available_suffix_bytes());
    }

    #[test]
    fn nested_subregions_still_do_the_right_thing() {
        let data: GrowableData<U0, U0> = data_region(1024, 0).into();
        let subdata = data
            .into_subregion::<U0, U0>() // into_subregion(..) -> new length: 1024
            .into_subregion::<U5, U0>() // into_subregion(5..) -> new length: 1019
            .into_subregion::<U0, U19>() //into_subregion(..1000) -> new length: 1000
            .into_subregion::<U0, U49>() //into_subregion(..951) -> new length: 951
            .into_subregion::<U10, U51>() //into_subregion(10..900) -> new length: 890
            .into_subregion::<U3, U89>() //into_subregion(3..801) -> new length: 798
            // and all types of ranges again, just in case they don't work if a certain other range happens beforehand
            .into_subregion::<U0, U0>() // into_subregion(..) -> new length: 798
            .into_subregion::<U5, U0>() // into_subregion(5..) -> new length: 793
            .into_subregion::<U0, U93>() // into_subregion(..700) -> new length: 700
            .into_subregion::<U0, U49>() // into_subregion(..651) -> new length: 651
            .into_subregion::<U10, U51>() //into_subregion(10..600) -> new_length: 590
            .into_subregion::<U3, U89>(); //into_subregion(3..501) -> new_length: 498
        assert_eq!(36, subdata.available_prefix_bytes());
        assert_eq!(490, subdata.available_suffix_bytes());
        assert_eq!(
            subdata.as_ref(),
            &data_region(1024, 0)[..][5..][..1000][..=950][10..900][3..=800][..][5..][..700]
                [..=650][10..600][3..=500]
        );
    }
}