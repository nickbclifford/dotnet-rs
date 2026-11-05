//! Frame-owned, non-moving storage ("homes") for locals whose address is taken.
//!
//! A local named by `ldloca` lives in its frame's home block rather than in an
//! evaluation-stack slot, so a byref or native-int address taken from it stays valid while
//! the evaluation stack grows and while the local is reassigned. The block is laid out
//! once per method instantiation ([`LocalHomeLayout`]) and allocated per frame
//! ([`FrameHomes`]). Its bytes hold each local in the same encoding a value-type field
//! uses, so GC references are serialized `ObjectRef`/`ManagedPtr` words traced through
//! the block's field layout.

use std::{ptr::NonNull, sync::Arc};

use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::raw_block::{RawBlock, RawBlockError};
use dotnet_value::layout::{FieldLayoutManager, LayoutManager};
use gc_arena::collect::Trace;

/// Where a homed local lives in the home block and how it is decoded.
#[derive(Debug)]
pub struct LocalHome {
    /// Byte offset of the local in the home block.
    pub offset: usize,
    /// Encoding of the local in the block.
    pub layout: Arc<LayoutManager>,
    /// Present when the local is a value type: what a load rebuilds the value from.
    pub value_type: Option<HomeValueType>,
}

/// The identity a value-type local is loaded as, matching the value `init_locals` creates.
#[derive(Debug)]
pub struct HomeValueType {
    pub description: TypeDescription,
    pub generics: GenericLookup,
    pub fields: Arc<FieldLayoutManager>,
}

/// The home block layout shared by every frame of one method instantiation.
#[derive(Debug)]
pub struct LocalHomeLayout {
    /// One field per homed local; drives GC tracing of the block.
    block: FieldLayoutManager,
    /// Indexed by local index; `None` for locals that stay in evaluation-stack slots.
    homes: Box<[Option<LocalHome>]>,
    /// Whether any home holds an `ObjectRef` or `ManagedPtr`.
    needs_trace: bool,
}

impl LocalHomeLayout {
    /// `block` must place every home in `homes` at its `offset` with its `layout`.
    pub fn new(block: FieldLayoutManager, homes: Box<[Option<LocalHome>]>) -> Self {
        debug_assert!(homes.iter().flatten().all(|home| {
            block
                .fields
                .values()
                .any(|field| field.position.as_usize() == home.offset)
        }));
        let needs_trace = block.has_ref_fields
            || !block.gc_desc.unaligned_offsets.is_empty()
            || block.gc_desc.bitmap.iter().any(|word| *word != 0);
        Self {
            block,
            homes,
            needs_trace,
        }
    }

    /// The home of local `index`, if it has one.
    #[inline]
    pub fn home(&self, index: usize) -> Option<&LocalHome> {
        self.homes.get(index).and_then(Option::as_ref)
    }

    /// Block alignment: the largest home alignment, and at least 16 like `localloc` blocks.
    pub fn alignment(&self) -> usize {
        self.block.alignment.max(16)
    }

    /// Block size in bytes.
    pub fn size(&self) -> usize {
        self.block.total_size
    }
}

/// The home block of one frame, released with the frame.
#[derive(Debug, Default)]
pub struct FrameHomes {
    homes: Option<(RawBlock, Arc<LocalHomeLayout>)>,
}

impl FrameHomes {
    /// Allocate a zeroed block for `layout`. Zero bytes are the default value of every local.
    pub fn allocate(layout: Arc<LocalHomeLayout>) -> Result<Self, RawBlockError> {
        let block = RawBlock::zeroed(layout.size(), layout.alignment())?;
        Ok(Self {
            homes: Some((block, layout)),
        })
    }

    /// Release the block. Addresses into it must not be used afterwards.
    pub fn release(&mut self) {
        self.homes = None;
    }

    /// Whether this frame has no home block.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.homes.is_none()
    }

    /// The home of local `index` and the address of its first byte, derived from the block
    /// base. The address stays valid until the block is released.
    #[inline]
    pub fn get(&self, index: usize) -> Option<(NonNull<u8>, &LocalHome)> {
        let (block, layout) = self.homes.as_ref()?;
        let home = layout.home(index)?;
        let address = block
            .ptr_at(home.offset)
            .expect("local home offset lies within its block");
        Some((address, home))
    }

    /// Visit every GC reference and managed pointer stored in the block.
    pub fn trace<'gc, Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        let Some((block, layout)) = &self.homes else {
            return;
        };
        if !layout.needs_trace {
            return;
        }
        // SAFETY: F10.RawMemoryAccessValid — the block is live and `payload_len` bytes long; it
        // was zero-initialized and is written only with layout encodings of its homes. Tracing
        // runs outside instruction execution, so no write through a home address overlaps this
        // shared view, which ends before `trace` returns.
        let bytes =
            unsafe { std::slice::from_raw_parts(block.base().as_ptr(), block.payload_len()) };
        layout.block.trace(bytes, cc);
    }
}
