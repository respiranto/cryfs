use std::num::NonZeroU64;

pub trait NonZeroU64Ext {
    #[must_use = "this returns the result of the operation, without modifying the original"]
    fn checked_add(self, other: u64) -> Option<NonZeroU64>;
}

impl NonZeroU64Ext for NonZeroU64 {
    #[inline]
    fn checked_add(self, other: u64) -> Option<NonZeroU64> {
        // Code from https://doc.rust-lang.org/src/core/num/nonzero.rs.html#503-510
        if let Some(result) = self.get().checked_add(other) {
            // SAFETY: $Int::checked_add returns None on overflow
            // so the result cannot be zero.
            Some(unsafe { NonZeroU64::new_unchecked(result) })
        } else {
            None
        }
    }
}