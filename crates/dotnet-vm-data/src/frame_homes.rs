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
//! Each block carries a unique serial issued by its owning thread's frame stack. A byref to a home is a
//! [`PointerOrigin::FrameHome`](dotnet_value::pointer::PointerOrigin::FrameHome) pointer naming
//! that serial; it reaches the block only through [`FrameHomes::block_for`], which finds nothing
//! once the frame has released its block.

use crate::frame_alloc::{FrameAllocationError, FrameAllocations, FrameBudget};
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
    /// Present for struct homes whose loads rebuild a value from its fields.
    pub value_type: Option<HomeValueType>,
    /// Declared scalar value-type identity, independent of stack widening.
    pub scalar_type: Option<TypeDescription>,
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

/// The primary and runtime temporary home blocks of one frame, released with the frame.
#[derive(Debug, Default)]
pub struct FrameHomes {
    homes: Option<HomeBlock>,
    temporaries: Vec<HomeBlock>,
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
            temporaries: Vec::new(),
        })
    }

    /// Release all blocks. Their byrefs stop resolving; native-int addresses into them dangle.
    pub fn release(&mut self) {
        self.homes = None;
        self.temporaries.clear();
    }

    /// Whether this frame has no home block.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.homes.is_none() && self.temporaries.is_empty()
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

    /// Allocate additional stable storage owned by this frame. The caller must issue a
    /// fresh serial from the thread's frame stack; it must never reuse a retired serial.
    /// `allocations` must be the budget ledger of this same owning frame.
    /// Temporary blocks remain live until the owning frame releases its homes.
    pub fn allocate_temporary(
        &mut self,
        layout: Arc<FrameHomeLayout>,
        serial: FrameHomeSerial,
        allocations: &mut FrameAllocations,
    ) -> Result<NonNull<u8>, FrameAllocationError> {
        assert!(self.block_for(serial).is_none(), "home serial already live");
        self.temporaries.try_reserve(1).map_err(|_| {
            FrameAllocationError::HostAllocationFailed(RawBlockError::AllocationFailed)
        })?;
        let block = allocations.allocate_external(
            layout.size(),
            layout.alignment(),
            &FrameBudget::DEFAULT,
        )?;
        let base = block.base();
        self.temporaries.push(HomeBlock {
            block,
            layout,
            serial,
        });
        Ok(base)
    }

    fn blocks(&self) -> impl Iterator<Item = &HomeBlock> {
        self.homes.iter().chain(self.temporaries.iter())
    }

    /// The base and length of a primary or temporary block owned by this live frame.
    #[inline]
    pub fn block_for(&self, serial: FrameHomeSerial) -> Option<(NonNull<u8>, usize)> {
        let homes = self.blocks().find(|homes| homes.serial == serial)?;
        Some((homes.block.base(), homes.block.payload_len()))
    }

    /// Whether this offset names an entire byref variable in this live block.
    pub fn is_byref_home(&self, serial: FrameHomeSerial, offset: usize) -> bool {
        self.blocks().any(|homes| {
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
        for homes in self.blocks() {
            if !homes.layout.needs_trace {
                continue;
            }
            let block = &homes.block;
            // SAFETY: F10.RawMemoryAccessValid — the block is live and `payload_len` bytes long; it
            // was zero-initialized and is written only with layout encodings of its homes. Tracing
            // runs outside instruction execution, so no write through a home address overlaps this
            // shared view, which ends before `trace` returns.
            let bytes =
                unsafe { std::slice::from_raw_parts(block.base().as_ptr(), block.payload_len()) };
            // SAFETY: F10.RawMemoryAccessValid — the live block contains its layout encodings and tracing excludes concurrent instruction writes.
            unsafe { homes.layout.block.trace(bytes, cc) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Arc<FrameHomeLayout> {
        Arc::new(FrameHomeLayout::new(
            FieldLayoutManager {
                fields: Default::default(),
                total_size: 32,
                alignment: 32,
                gc_desc: Default::default(),
                has_ref_fields: false,
            },
            Box::new([]),
            Box::new([]),
        ))
    }

    #[test]
    fn temporary_homes_survive_growth_and_move_and_retire_on_release() {
        let mut allocations = FrameAllocations::default();
        let mut homes = FrameHomes::default();
        let serial = FrameHomeSerial::FIRST;
        let base = homes
            .allocate_temporary(layout(), serial, &mut allocations)
            .unwrap();
        assert_eq!(base.as_ptr().addr() % 32, 0);
        // SAFETY: F10.RawMemoryAccessValid — this is a live, zeroed 32-byte block.
        unsafe { base.as_ptr().write(0x5a) };
        for id in 2..100 {
            homes
                .allocate_temporary(layout(), FrameHomeSerial::new(id), &mut allocations)
                .unwrap();
        }
        let mut moved = Box::new(homes);
        assert_eq!(moved.block_for(serial), Some((base, 32)));
        // SAFETY: F10.RawMemoryAccessValid — moving ownership never moves the block.
        assert_eq!(unsafe { base.as_ptr().read() }, 0x5a);
        assert!(!moved.is_empty());
        moved.release();
        assert!(moved.is_empty());
        assert!(moved.block_for(serial).is_none());
        moved
            .allocate_temporary(layout(), FrameHomeSerial::new(100), &mut allocations)
            .unwrap();
        assert!(moved.block_for(serial).is_none());
    }

    #[test]
    fn temporary_and_primary_lookup_are_independent() {
        let mut allocations = FrameAllocations::default();
        let mut homes = FrameHomes::allocate(layout(), FrameHomeSerial::FIRST).unwrap();
        let primary = homes.block_for(FrameHomeSerial::FIRST).unwrap();
        let serial = FrameHomeSerial::new(2);
        let temporary = homes
            .allocate_temporary(layout(), serial, &mut allocations)
            .unwrap();
        assert_eq!(homes.block_for(FrameHomeSerial::FIRST), Some(primary));
        assert_eq!(homes.block_for(serial), Some((temporary, 32)));
        assert!(homes.layout().is_some());
        homes.release();
        assert!(homes.block_for(FrameHomeSerial::FIRST).is_none());
        assert!(homes.block_for(serial).is_none());
    }
}

/// Completion of a value-type constructor whose receiver lives in its caller's homes.
/// This contains metadata only; receiver GC references are traced by the owning home block.
#[derive(Debug)]
pub struct ConstructorResult {
    pub result_slot: dotnet_utils::StackSlotIndex,
    pub serial: FrameHomeSerial,
    pub description: TypeDescription,
    pub generics: GenericLookup,
    pub fields: Arc<FieldLayoutManager>,
}
