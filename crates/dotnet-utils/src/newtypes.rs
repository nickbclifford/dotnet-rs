use std::ops::{Add, Sub, AddAssign, SubAssign, Mul};
use std::fmt::{self, Display, Formatter};
use gc_arena::Collect;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Collect)]
#[collect(require_static)]
pub struct ByteOffset(pub usize);

impl Display for ByteOffset {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<usize> for ByteOffset {
    fn from(offset: usize) -> Self {
        ByteOffset(offset)
    }
}

impl From<ByteOffset> for usize {
    fn from(offset: ByteOffset) -> Self {
        offset.0
    }
}

impl ByteOffset {
    pub const ZERO: Self = ByteOffset(0);

    pub fn new(offset: usize) -> Self {
        ByteOffset(offset)
    }

    pub fn checked_add(self, other: impl Into<usize>) -> Option<Self> {
        self.0.checked_add(other.into()).map(ByteOffset)
    }

    pub fn checked_sub(self, other: impl Into<usize>) -> Option<Self> {
        self.0.checked_sub(other.into()).map(ByteOffset)
    }

    pub fn checked_mul(self, other: usize) -> Option<Self> {
        self.0.checked_mul(other).map(ByteOffset)
    }

    pub fn as_usize(self) -> usize {
        self.0
    }
}

impl Add<usize> for ByteOffset {
    type Output = Self;
    fn add(self, rhs: usize) -> Self {
        ByteOffset(self.0 + rhs)
    }
}

impl Add<ByteOffset> for ByteOffset {
    type Output = Self;
    fn add(self, rhs: ByteOffset) -> Self {
        ByteOffset(self.0 + rhs.0)
    }
}

impl Sub<usize> for ByteOffset {
    type Output = Self;
    fn sub(self, rhs: usize) -> Self {
        ByteOffset(self.0 - rhs)
    }
}

impl Sub<ByteOffset> for ByteOffset {
    type Output = Self;
    fn sub(self, rhs: ByteOffset) -> Self {
        ByteOffset(self.0 - rhs.0)
    }
}

impl Mul<usize> for ByteOffset {
    type Output = Self;
    fn mul(self, rhs: usize) -> Self {
        ByteOffset(self.0 * rhs)
    }
}

impl AddAssign<usize> for ByteOffset {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl AddAssign<ByteOffset> for ByteOffset {
    fn add_assign(&mut self, rhs: ByteOffset) {
        self.0 += rhs.0;
    }
}

impl SubAssign<usize> for ByteOffset {
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

impl SubAssign<ByteOffset> for ByteOffset {
    fn sub_assign(&mut self, rhs: ByteOffset) {
        self.0 -= rhs.0;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Collect)]
#[collect(require_static)]
pub struct FieldIndex(pub usize);

impl Display for FieldIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<usize> for FieldIndex {
    fn from(index: usize) -> Self {
        FieldIndex(index)
    }
}

impl From<FieldIndex> for usize {
    fn from(index: FieldIndex) -> Self {
        index.0
    }
}

impl FieldIndex {
    pub fn new(index: usize) -> Self {
        FieldIndex(index)
    }

    pub fn as_usize(self) -> usize {
        self.0
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Collect)]
#[collect(require_static)]
pub struct ArenaId(pub u64);

impl Display for ArenaId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for ArenaId {
    fn from(id: u64) -> Self {
        ArenaId(id)
    }
}

impl From<ArenaId> for u64 {
    fn from(id: ArenaId) -> Self {
        id.0
    }
}

impl ArenaId {
    pub const INVALID: Self = ArenaId(u64::MAX);

    pub fn new(id: u64) -> Self {
        ArenaId(id)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Collect)]
#[collect(require_static)]
pub struct LocalIndex(pub usize);

impl LocalIndex {
    pub fn as_usize(self) -> usize {
        self.0
    }
}

impl Add<usize> for LocalIndex {
    type Output = Self;
    fn add(self, rhs: usize) -> Self {
        LocalIndex(self.0 + rhs)
    }
}

impl Sub<usize> for LocalIndex {
    type Output = Self;
    fn sub(self, rhs: usize) -> Self {
        LocalIndex(self.0 - rhs)
    }
}

impl AddAssign<usize> for LocalIndex {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for LocalIndex {
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Collect)]
#[collect(require_static)]
pub struct ArgumentIndex(pub usize);

impl ArgumentIndex {
    pub fn as_usize(self) -> usize {
        self.0
    }
}

impl Add<usize> for ArgumentIndex {
    type Output = Self;
    fn add(self, rhs: usize) -> Self {
        ArgumentIndex(self.0 + rhs)
    }
}

impl Sub<usize> for ArgumentIndex {
    type Output = Self;
    fn sub(self, rhs: usize) -> Self {
        ArgumentIndex(self.0 - rhs)
    }
}

impl AddAssign<usize> for ArgumentIndex {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for ArgumentIndex {
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}

impl Display for LocalIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Display for ArgumentIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Display for StackSlotIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Collect)]
#[collect(require_static)]
pub struct StackSlotIndex(pub usize);

impl StackSlotIndex {
    pub fn as_usize(self) -> usize {
        self.0
    }

    pub fn saturating_sub(self, rhs: impl Into<usize>) -> Self {
        StackSlotIndex(self.0.saturating_sub(rhs.into()))
    }

    pub fn saturating_sub_idx(self, rhs: StackSlotIndex) -> Self {
        StackSlotIndex(self.0.saturating_sub(rhs.0))
    }

    pub fn checked_sub(self, rhs: usize) -> Option<Self> {
        self.0.checked_sub(rhs).map(StackSlotIndex)
    }
}

impl From<StackSlotIndex> for usize {
    fn from(index: StackSlotIndex) -> Self {
        index.0
    }
}

impl Add<usize> for StackSlotIndex {
    type Output = Self;
    fn add(self, rhs: usize) -> Self {
        StackSlotIndex(self.0 + rhs)
    }
}

impl Add<LocalIndex> for StackSlotIndex {
    type Output = Self;
    fn add(self, rhs: LocalIndex) -> Self {
        StackSlotIndex(self.0 + rhs.0)
    }
}

impl Add<ArgumentIndex> for StackSlotIndex {
    type Output = Self;
    fn add(self, rhs: ArgumentIndex) -> Self {
        StackSlotIndex(self.0 + rhs.0)
    }
}

impl Add<StackSlotIndex> for StackSlotIndex {
    type Output = Self;
    fn add(self, rhs: StackSlotIndex) -> Self {
        StackSlotIndex(self.0 + rhs.0)
    }
}

impl Sub<usize> for StackSlotIndex {
    type Output = Self;
    fn sub(self, rhs: usize) -> Self {
        StackSlotIndex(self.0 - rhs)
    }
}

impl Sub<StackSlotIndex> for StackSlotIndex {
    type Output = Self;
    fn sub(self, rhs: StackSlotIndex) -> Self {
        StackSlotIndex(self.0 - rhs.0)
    }
}

impl AddAssign<usize> for StackSlotIndex {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

impl SubAssign<usize> for StackSlotIndex {
    fn sub_assign(&mut self, rhs: usize) {
        self.0 -= rhs;
    }
}
