//! Bounds checked access to guest memory.
//!
//! A guest passes addresses and lengths as `i32` values.
//! [`GuestPtr`] and [`GuestSlice`] turn them into checked ranges, and
//! [`GuestMemory`] reads and writes only through those ranges.

use wasmtime::AsContextMut;

use crate::error::{Error, MemoryError};
use crate::runtime::HostState;

/// One address in guest memory.
///
/// A guest passes an address as a signed 32 bit value, and the crate rejects
/// a negative one, so it serves the first two gibibytes of a guest memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestPtr(u32);

impl GuestPtr {
    /// The address as an unsigned offset.
    pub fn address(self) -> u32 {
        self.0
    }

    /// An address the host already validated, such as one it wrote itself.
    pub(crate) fn from_address(address: u32) -> Self {
        Self(address)
    }
}

impl TryFrom<i32> for GuestPtr {
    type Error = MemoryError;

    fn try_from(ptr: i32) -> Result<Self, MemoryError> {
        u32::try_from(ptr)
            .map(Self)
            .map_err(|_| MemoryError::NegativePointer { ptr })
    }
}

/// A range in guest memory, as an address and a length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestSlice {
    ptr: u32,
    len: u32,
}

impl GuestSlice {
    /// A range that starts at `ptr` and spans `len` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::RangeOverflow`] when the end does not fit in
    /// 32 bits.
    pub fn new(ptr: GuestPtr, len: u32) -> Result<Self, MemoryError> {
        ptr.0
            .checked_add(len)
            .map(|_| Self { ptr: ptr.0, len })
            .ok_or(MemoryError::RangeOverflow { ptr: ptr.0, len })
    }

    /// The start of the range.
    pub fn ptr(self) -> GuestPtr {
        GuestPtr(self.ptr)
    }

    /// The length of the range in bytes.
    pub fn len(self) -> u32 {
        self.len
    }

    /// Whether the range spans no bytes.
    #[cfg(test)]
    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    fn bounds(self, memory_size: usize) -> Result<std::ops::Range<usize>, MemoryError> {
        let start = usize::try_from(self.ptr).map_err(|_| self.out_of_bounds(memory_size))?;
        let len = usize::try_from(self.len).map_err(|_| self.out_of_bounds(memory_size))?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| self.out_of_bounds(memory_size))?;
        if end > memory_size {
            return Err(self.out_of_bounds(memory_size));
        }
        Ok(start..end)
    }

    fn out_of_bounds(self, memory_size: usize) -> MemoryError {
        MemoryError::OutOfBounds {
            ptr: self.ptr,
            len: self.len,
            memory_size,
        }
    }
}

impl TryFrom<(i32, i32)> for GuestSlice {
    type Error = MemoryError;

    fn try_from((ptr, len): (i32, i32)) -> Result<Self, MemoryError> {
        let ptr = GuestPtr::try_from(ptr)?;
        let len = u32::try_from(len).map_err(|_| MemoryError::NegativeLength { len })?;
        Self::new(ptr, len)
    }
}

/// A view of guest memory that checks every access.
///
/// Never keep it across a call into the guest, because the guest may grow
/// its memory.
pub struct GuestMemory<'a> {
    bytes: &'a mut [u8],
}

impl<'a> GuestMemory<'a> {
    pub(crate) fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes }
    }

    /// The memory size in bytes.
    #[cfg(test)]
    pub fn size(&self) -> usize {
        self.bytes.len()
    }

    /// The bytes of `slice`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::OutOfBounds`] when the range ends past the
    /// memory.
    pub fn read(&self, slice: GuestSlice) -> Result<&[u8], MemoryError> {
        let range = slice.bounds(self.bytes.len())?;
        Ok(&self.bytes[range])
    }

    /// The bytes of `slice`, for an in place write.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::OutOfBounds`] when the range ends past the
    /// memory.
    pub fn slice_mut(&mut self, slice: GuestSlice) -> Result<&mut [u8], MemoryError> {
        let range = slice.bounds(self.bytes.len())?;
        Ok(&mut self.bytes[range])
    }

    /// Copies `bytes` over `slice`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::LengthMismatch`] when `bytes` is not exactly as
    /// long as the range, and [`MemoryError::OutOfBounds`] when the range
    /// ends past the memory.
    pub fn write(&mut self, slice: GuestSlice, bytes: &[u8]) -> Result<(), MemoryError> {
        if bytes.len() != usize::try_from(slice.len).unwrap_or(usize::MAX) {
            return Err(MemoryError::LengthMismatch {
                expected: slice.len,
                actual: bytes.len(),
            });
        }
        self.slice_mut(slice)?.copy_from_slice(bytes);
        Ok(())
    }

    /// The little-endian `u32` at `ptr`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] when the four bytes are not inside the memory.
    pub fn read_u32(&self, ptr: GuestPtr) -> Result<u32, MemoryError> {
        let bytes = self.read(GuestSlice::new(ptr, 4)?)?;
        Ok(u32::from_le_bytes(word::<4>(bytes)))
    }

    /// Writes `value` as a little-endian `u32` at `ptr`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] when the four bytes are not inside the memory.
    pub fn write_u32(&mut self, ptr: GuestPtr, value: u32) -> Result<(), MemoryError> {
        self.write(GuestSlice::new(ptr, 4)?, &value.to_le_bytes())
    }

    /// The little-endian `u64` at `ptr`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] when the eight bytes are not inside the memory.
    pub fn read_u64(&self, ptr: GuestPtr) -> Result<u64, MemoryError> {
        let bytes = self.read(GuestSlice::new(ptr, 8)?)?;
        Ok(u64::from_le_bytes(word::<8>(bytes)))
    }

    /// Writes `value` as a little-endian `u64` at `ptr`.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError`] when the eight bytes are not inside the memory.
    pub fn write_u64(&mut self, ptr: GuestPtr, value: u64) -> Result<(), MemoryError> {
        self.write(GuestSlice::new(ptr, 8)?, &value.to_le_bytes())
    }
}

fn word<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut word = [0u8; N];
    word.copy_from_slice(&bytes[..N]);
    word
}

/// The guest memory and the host state of a store or a caller, in one
/// borrow.
///
/// Every host function that touches guest memory starts here.
/// The memory handle is cached after instantiation, so a wasm start section
/// cannot reach guest memory through a host function.
/// The ABI start functions are exports that run after instantiation, so they
/// can.
pub(crate) fn split<C: AsContextMut<Data = HostState>>(
    ctx: &mut C,
) -> Result<(GuestMemory<'_>, &mut HostState), Error> {
    let memory = ctx
        .as_context()
        .data()
        .memory()
        .ok_or(Error::MissingMemory)?;
    let (bytes, state) = memory.data_and_store_mut(ctx);
    Ok((GuestMemory::new(bytes), state))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: usize = 65_536;

    fn ptr(address: u32) -> GuestPtr {
        GuestPtr::from_address(address)
    }

    #[test]
    fn guest_ptr_accepts_non_negative_values_and_rejects_negative_ones() {
        // Arrange
        let values = [0, i32::MAX, -1];

        // Act
        let results: Vec<_> = values.iter().map(|&v| GuestPtr::try_from(v)).collect();

        // Assert
        assert_eq!(
            results,
            vec![
                Ok(ptr(0)),
                Ok(ptr(2_147_483_647)),
                Err(MemoryError::NegativePointer { ptr: -1 })
            ]
        );
    }

    #[test]
    fn guest_slice_rejects_negative_and_overflowing_ranges_and_accepts_the_rest() {
        // Arrange
        let inputs = [(-1, 4), (4, -1), (i32::MAX, i32::MAX)];

        // Act
        let results: Vec<_> = inputs
            .iter()
            .map(|&pair| GuestSlice::try_from(pair))
            .collect();

        // Assert
        assert_eq!(results[0], Err(MemoryError::NegativePointer { ptr: -1 }));
        assert_eq!(results[1], Err(MemoryError::NegativeLength { len: -1 }));
        assert!(results[2].is_ok());
        assert_eq!(
            GuestSlice::new(ptr(u32::MAX), 1),
            Err(MemoryError::RangeOverflow {
                ptr: u32::MAX,
                len: 1
            })
        );
    }

    #[test]
    fn guest_slice_reports_emptiness() {
        // Arrange
        let slices = [
            GuestSlice::new(ptr(8), 0).unwrap(),
            GuestSlice::new(ptr(8), 1).unwrap(),
        ];

        // Act
        let observed = [slices[0].is_empty(), slices[1].is_empty()];

        // Assert
        assert_eq!(observed, [true, false]);
        assert_eq!((slices[1].ptr(), slices[1].len()), (ptr(8), 1));
    }

    #[test]
    fn read_and_write_round_trip_at_both_ends() {
        // Arrange
        let mut bytes = vec![0u8; PAGE];
        let mut memory = GuestMemory::new(&mut bytes);
        let first = GuestSlice::new(ptr(0), 3).unwrap();
        let last = GuestSlice::new(ptr(u32::try_from(PAGE - 3).unwrap()), 3).unwrap();

        // Act
        let results = [memory.write(first, b"abc"), memory.write(last, b"xyz")];

        // Assert
        assert_eq!(results, [Ok(()), Ok(())]);
        assert_eq!(memory.read(first), Ok(b"abc".as_slice()));
        assert_eq!(memory.read(last), Ok(b"xyz".as_slice()));
        assert_eq!(memory.size(), PAGE);
    }

    #[test]
    fn an_empty_slice_at_the_end_is_in_bounds() {
        // Arrange
        let mut bytes = vec![0u8; PAGE];
        let memory = GuestMemory::new(&mut bytes);
        let slice = GuestSlice::new(ptr(u32::try_from(PAGE).unwrap()), 0).unwrap();

        // Act
        let read = memory.read(slice);

        // Assert
        assert_eq!(read, Ok(b"".as_slice()));
    }

    #[test]
    fn one_byte_past_the_end_is_out_of_bounds() {
        // Arrange
        let mut bytes = vec![0u8; PAGE];
        let memory = GuestMemory::new(&mut bytes);
        let slice = GuestSlice::new(ptr(u32::try_from(PAGE - 3).unwrap()), 4).unwrap();

        // Act
        let read = memory.read(slice);

        // Assert
        assert_eq!(
            read,
            Err(MemoryError::OutOfBounds {
                ptr: 65_533,
                len: 4,
                memory_size: PAGE
            })
        );
    }

    #[test]
    fn slice_mut_allows_an_in_place_fill() {
        // Arrange
        let mut bytes = vec![0u8; 16];
        let mut memory = GuestMemory::new(&mut bytes);
        let slice = GuestSlice::new(ptr(4), 4).unwrap();

        // Act
        memory.slice_mut(slice).unwrap().fill(0xab);

        // Assert
        assert_eq!(
            bytes,
            [0, 0, 0, 0, 0xab, 0xab, 0xab, 0xab, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn integers_are_little_endian() {
        // Arrange
        let mut bytes = vec![0u8; 16];
        let mut memory = GuestMemory::new(&mut bytes);

        // Act
        let results = [
            memory.write_u32(ptr(0), 0x0403_0201),
            memory.write_u64(ptr(8), 0x0807_0605_0403_0201),
        ];

        // Assert
        assert_eq!(results, [Ok(()), Ok(())]);
        assert_eq!(memory.read_u32(ptr(0)), Ok(0x0403_0201));
        assert_eq!(memory.read_u64(ptr(8)), Ok(0x0807_0605_0403_0201));
        assert_eq!(bytes, [1, 2, 3, 4, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn u64_fits_at_the_last_offset_and_not_one_later() {
        // Arrange
        let mut bytes = vec![0u8; PAGE];
        let mut memory = GuestMemory::new(&mut bytes);
        let last = ptr(u32::try_from(PAGE - 8).unwrap());
        let past = ptr(u32::try_from(PAGE - 7).unwrap());

        // Act
        let results = [memory.write_u64(last, 1), memory.write_u64(past, 1)];

        // Assert
        assert_eq!(results[0], Ok(()));
        assert_eq!(
            results[1],
            Err(MemoryError::OutOfBounds {
                ptr: 65_529,
                len: 8,
                memory_size: PAGE
            })
        );
    }

    #[test]
    fn write_with_the_wrong_length_is_rejected() {
        // Arrange
        let mut bytes = vec![0u8; 16];
        let mut memory = GuestMemory::new(&mut bytes);
        let slice = GuestSlice::new(ptr(0), 4).unwrap();

        // Act
        let result = memory.write(slice, b"abcde");

        // Assert
        assert_eq!(
            result,
            Err(MemoryError::LengthMismatch {
                expected: 4,
                actual: 5
            })
        );
    }
}
