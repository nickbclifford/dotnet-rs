//! Frame-owned, non-moving storage ("homes") for locals and arguments whose address is taken.
//!
//! A local named by `ldloca`, or an argument named by `ldarga`, lives in its frame's home block
//! rather than in an evaluation-stack slot, so a byref or native-int address taken from it stays
//! valid while the evaluation stack grows and while the variable is reassigned. The block is laid
//! out once per method instantiation ([`FrameHomeLayout`]) and allocated per frame
//! ([`FrameHomes`]). Its bytes hold each variable in the same encoding a value-type field uses,
//! so GC references are serialized `ObjectRef`/`ManagedPtr` words traced through the block's
//! field layout.
//!
//! Each block carries the serial of its frame. A byref to a home is a
//! [`PointerOrigin::FrameHome`](dotnet_value::pointer::PointerOrigin::FrameHome) pointer naming
//! that serial; it reaches the block only through [`FrameHomes::block_for`], which finds nothing
//! once the frame has released its block.

use std::{ptr::NonNull, sync::Arc};

use dotnet_types::{TypeDescription, generics::GenericLookup};
use dotnet_utils::raw_block::{RawBlock, RawBlockError};
use dotnet_value::{
    layout::{FieldLayoutManager, LayoutManager},
    pointer::FrameHomeSerial,
};
use gc_arena::collect::Trace;

/// Where a homed variable lives in the home block and how it is decoded.
#[derive(Debug)]
pub struct VariableHome {
    /// Byte offset of the variable in the home block.
    pub offset: usize,
    /// Encoding of the variable in the block.
    pub layout: Arc<LayoutManager>,
    /// Present when the variable is a value type: what a load rebuilds the value from.
    pub value_type: Option<HomeValueType>,
}

/// The identity a value-type variable is loaded as, matching the value `init_locals` creates or
/// the caller passes.
#[derive(Debug)]
pub struct HomeValueType {
    pub description: TypeDescription,
    pub generics: GenericLookup,
    pub fields: Arc<FieldLayoutManager>,
}

/// The home block layout shared by every frame of one method instantiation.
#[derive(Debug)]
pub struct FrameHomeLayout {
    /// One field per homed variable; drives GC tracing of the block.
    block: FieldLayoutManager,
    /// Indexed by local index; `None` for locals that stay in evaluation-stack slots.
    locals: Box<[Option<VariableHome>]>,
    /// Indexed by argument index (including `this`); `None` for arguments that stay in
    /// evaluation-stack slots.
    arguments: Box<[Option<VariableHome>]>,
    /// Whether any home holds an `ObjectRef` or `ManagedPtr`.
    needs_trace: bool,
}

impl FrameHomeLayout {
    /// `block` must place every home in `locals` and `arguments` at its `offset` with its
    /// `layout`.
    pub fn new(
        block: FieldLayoutManager,
        locals: Box<[Option<VariableHome>]>,
        arguments: Box<[Option<VariableHome>]>,
    ) -> Self {
        debug_assert!(locals.iter().chain(arguments.iter()).flatten().all(|home| {
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
            locals,
            arguments,
            needs_trace,
        }
    }

    /// The home of local `index`, if it has one.
    #[inline]
    pub fn local(&self, index: usize) -> Option<&VariableHome> {
        self.locals.get(index).and_then(Option::as_ref)
    }

    /// The home of argument `index`, if it has one.
    #[inline]
    pub fn argument(&self, index: usize) -> Option<&VariableHome> {
        self.arguments.get(index).and_then(Option::as_ref)
    }

    /// Indices and homes of the homed arguments.
    pub fn homed_arguments(&self) -> impl Iterator<Item = (usize, &VariableHome)> {
        self.arguments
            .iter()
            .enumerate()
            .filter_map(|(index, home)| home.as_ref().map(|home| (index, home)))
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

/// A homed variable of a live frame.
#[derive(Clone, Copy, Debug)]
pub struct HomeSlot<'a> {
    /// Address of the variable's first byte, derived from the block base.
    pub address: NonNull<u8>,
    /// Serial of the frame that owns the block.
    pub serial: FrameHomeSerial,
    pub home: &'a VariableHome,
}

#[derive(Debug)]
struct HomeBlock {
    block: RawBlock,
    layout: Arc<FrameHomeLayout>,
    serial: FrameHomeSerial,
}

/// The home block of one frame, released with the frame.
#[derive(Debug, Default)]
pub struct FrameHomes {
    homes: Option<HomeBlock>,
}

impl FrameHomes {
    /// Allocate a zeroed block for `layout`, owned by the frame with `serial`. Zero bytes are the
    /// default value of every local; homed arguments are copied in at frame entry.
    pub fn allocate(
        layout: Arc<FrameHomeLayout>,
        serial: FrameHomeSerial,
    ) -> Result<Self, RawBlockError> {
        let block = RawBlock::zeroed(layout.size(), layout.alignment())?;
        Ok(Self {
            homes: Some(HomeBlock {
                block,
                layout,
                serial,
            }),
        })
    }

    /// Release the block. Byrefs to it stop resolving; native-int addresses into it dangle.
    pub fn release(&mut self) {
        self.homes = None;
    }

    /// Whether this frame has no home block.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.homes.is_none()
    }

    /// The layout of this frame's homes, if it has a block.
    #[inline]
    pub fn layout(&self) -> Option<&Arc<FrameHomeLayout>> {
        self.homes.as_ref().map(|homes| &homes.layout)
    }

    fn slot<'a>(homes: &'a HomeBlock, home: &'a VariableHome) -> HomeSlot<'a> {
        HomeSlot {
            address: homes
                .block
                .ptr_at(home.offset)
                .expect("a variable home offset lies within its block"),
            serial: homes.serial,
            home,
        }
    }

    /// The home of local `index`. The address stays valid until the block is released.
    #[inline]
    pub fn local(&self, index: usize) -> Option<HomeSlot<'_>> {
        let homes = self.homes.as_ref()?;
        let home = homes.layout.local(index)?;
        Some(Self::slot(homes, home))
    }

    /// The home of argument `index`. The address stays valid until the block is released.
    #[inline]
    pub fn argument(&self, index: usize) -> Option<HomeSlot<'_>> {
        let homes = self.homes.as_ref()?;
        let home = homes.layout.argument(index)?;
        Some(Self::slot(homes, home))
    }

    /// The base address and length of the block when it belongs to the frame with `serial`.
    #[inline]
    pub fn block_for(&self, serial: FrameHomeSerial) -> Option<(NonNull<u8>, usize)> {
        let homes = self.homes.as_ref().filter(|homes| homes.serial == serial)?;
        Some((homes.block.base(), homes.block.payload_len()))
    }

    /// Whether this offset names an entire byref variable in this live block.
    pub fn is_byref_home(&self, serial: FrameHomeSerial, offset: usize) -> bool {
        self.homes.as_ref().is_some_and(|homes| {
            homes.serial == serial
                && homes
                    .layout
                    .locals
                    .iter()
                    .chain(homes.layout.arguments.iter())
                    .flatten()
                    .any(|home| {
                        home.offset == offset
                            && matches!(
                                &*home.layout,
                                LayoutManager::Scalar(dotnet_value::layout::Scalar::ManagedPtr)
                            )
                    })
        })
    }

    /// Visit every GC reference and managed pointer stored in the block.
    pub fn trace<'gc, Tr: Trace<'gc>>(&self, cc: &mut Tr) {
        let Some(homes) = &self.homes else {
            return;
        };
        if !homes.layout.needs_trace {
            return;
        }
        let block = &homes.block;
        // SAFETY: F10.RawMemoryAccessValid — the block is live and `payload_len` bytes long; it
        // was zero-initialized and is written only with layout encodings of its homes. Tracing
        // runs outside instruction execution, so no write through a home address overlaps this
        // shared view, which ends before `trace` returns.
        let bytes =
            unsafe { std::slice::from_raw_parts(block.base().as_ptr(), block.payload_len()) };
        homes.layout.block.trace(bytes, cc);
    }
}
