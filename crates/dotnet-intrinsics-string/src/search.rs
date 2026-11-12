use crate::{
    IntrinsicStringHost, NULL_REF_MSG,
    simd::{probe_all_ascii_whitespace, probe_index_of_char},
};
use dotnet_macros::dotnet_intrinsic;
use dotnet_types::{generics::GenericLookup, members::MethodDescription};
use dotnet_utils::GcScopeGuard;
use dotnet_value::{
    StackValue,
    object::{HeapStorage, ObjectRef},
    string::CLRString,
    with_string,
};
use dotnet_vm_data::StepResult;
use dotnet_vm_ops::{
    effects::{NativeInvocation, NativeOperandView, NativeResult, NativeSelection, VmEffects},
    ops::{ExceptionOps, RawMemoryOps, TypedStackOps},
};

/// System.String::IndexOf(char)
/// System.String::IndexOf(char, int)
#[dotnet_intrinsic("int System.String::IndexOf(char)")]
#[dotnet_intrinsic("int System.String::IndexOf(char, int)")]
pub fn intrinsic_string_index_of<
    'gc,
    T: TypedStackOps<'gc> + ExceptionOps<'gc> + RawMemoryOps<'gc>,
>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (c, start_at) = if method.signature().parameters.len() == 1 {
        let c = ctx.pop_i32();
        (c as u16, 0usize)
    } else {
        let start_at = ctx.pop_i32();
        let c = ctx.pop_i32();
        (c as u16, start_at as usize)
    };

    let val = ctx.pop();
    let index = with_string!(ctx, val, |s| {
        if start_at >= s.len() {
            None
        } else {
            let haystack = &s[start_at..];
            probe_index_of_char(haystack, c)
                .unwrap_or_else(|| haystack.iter().position(|x| *x == c))
        }
    });

    ctx.push_i32(match index {
        None => -1,
        Some(i) => (i + start_at) as i32,
    });
    StepResult::Continue
}

/// Maximum number of UTF-16 code units copied in one substring chunk.
pub const SUBSTRING_CHUNK_SIZE: usize = 64 * 1024;

/// Per-thread diagnostic counters; never enabled in ordinary throughput runs.
#[cfg(feature = "bench-instrumentation")]
pub mod substring_metrics {
    use std::cell::Cell;

    #[derive(Clone, Copy, Debug, Default)]
    pub struct Snapshot {
        pub copied_units: usize,
        pub buffer_allocations: usize,
        pub buffer_reallocations: usize,
        /// Largest live copy buffer observed, excluding sources/results and allocator overhead.
        pub max_buffer_bytes: usize,
    }
    thread_local! { static COUNTERS: Cell<Snapshot> = Cell::new(Snapshot::default()); }
    pub fn reset() {
        COUNTERS.set(Snapshot::default());
    }
    pub fn snapshot() -> Snapshot {
        COUNTERS.get()
    }
    pub(super) fn allocation(capacity: usize) {
        COUNTERS.with(|slot| {
            let mut c = slot.get();
            c.buffer_allocations += usize::from(capacity != 0);
            c.max_buffer_bytes = c.max_buffer_bytes.max(capacity * size_of::<u16>());
            slot.set(c);
        });
    }
    pub(super) fn copy(count: usize, before: usize, after: usize) {
        COUNTERS.with(|slot| {
            let mut c = slot.get();
            c.copied_units += count;
            c.buffer_reallocations += usize::from(before != after);
            c.max_buffer_bytes = c.max_buffer_bytes.max(after * size_of::<u16>());
            slot.set(c);
        });
    }
}

/// Allocate the copy buffer through the same diagnostic boundary on both paths.
#[inline]
pub fn substring_buffer(length: usize) -> Vec<u16> {
    let buffer = Vec::with_capacity(length);
    #[cfg(feature = "bench-instrumentation")]
    substring_metrics::allocation(buffer.capacity());
    buffer
}

/// Append one bounded chunk of a previously validated substring.
pub fn copy_substring_chunk(
    source: &[u16],
    start: usize,
    length: usize,
    offset: usize,
    destination: &mut Vec<u16>,
) -> usize {
    let count = (length - offset).min(SUBSTRING_CHUNK_SIZE);
    #[cfg(feature = "bench-instrumentation")]
    let before = destination.capacity();
    destination.extend_from_slice(&source[start + offset..start + offset + count]);
    #[cfg(feature = "bench-instrumentation")]
    substring_metrics::copy(count, before, destination.capacity());
    count
}

/// Validate the signed arguments before calculating a bounded substring range.
fn substring_range(
    start: i32,
    length: Option<i32>,
    source_len: usize,
) -> Result<(usize, usize), &'static str> {
    if start < 0 || start as usize > source_len {
        return Err("startIndex");
    }
    let start = start as usize;
    let length = match length {
        None => source_len - start,
        Some(length) => {
            if length < 0 || length as usize > source_len - start {
                return Err("length");
            }
            length as usize
        }
    };
    Ok((start, length))
}

/// Keep valid small copies synchronous; invalid operands are validated by the body.
pub fn substring_select<'gc, T: IntrinsicStringHost<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    let parameter_count = method.signature().parameters.len();
    if !(1..=2).contains(&parameter_count) {
        return NativeSelection::Reject(
            dotnet_types::error::ExecutionError::InternalError(
                "Substring has an invalid parameter count".into(),
            )
            .into(),
        );
    }
    let length = if parameter_count == 2 {
        match operands.peek_at(0) {
            StackValue::Int32(length) => Some(length),
            _ => {
                return NativeSelection::Reject(
                    dotnet_types::error::ExecutionError::InternalError(
                        "Substring length is not an int32".into(),
                    )
                    .into(),
                );
            }
        }
    } else {
        None
    };
    let StackValue::Int32(start) = operands.peek_at(parameter_count - 1) else {
        return NativeSelection::Reject(
            dotnet_types::error::ExecutionError::InternalError(
                "Substring start is not an int32".into(),
            )
            .into(),
        );
    };
    if let StackValue::ObjectRef(ObjectRef(Some(handle))) = operands.peek_at(parameter_count) {
        let inner = handle.borrow();
        if let HeapStorage::Str(source) = &inner.storage
            && let Ok((_, valid_length)) = substring_range(start, length, source.len())
            && valid_length <= SUBSTRING_CHUNK_SIZE
        {
            return NativeSelection::Fallback;
        }
    }
    NativeSelection::Async(NativeInvocation {
        start: start as u32 as usize,
        length: length.unwrap_or(0) as u32 as usize,
        parameter_count,
    })
}

/// Validate and copy a substring without retaining a heap borrow across a yield.
#[dotnet_intrinsic("string System.String::Substring(int)", select = substring_select, fallback = intrinsic_string_substring)]
#[dotnet_intrinsic("string System.String::Substring(int, int)", select = substring_select, fallback = intrinsic_string_substring)]
pub async fn substring_async(mut fx: VmEffects) -> NativeResult {
    let invocation = fx.invocation();
    let (source, start, length) = {
        let mut vm = fx.vm().await?;
        vm.discard_scalars(invocation.parameter_count)?;
        let Some(source) = vm.pop_string_candidate()? else {
            return Err(vm.exception("System.ArgumentException", "The argument must be a string.")?);
        };
        let source_len = vm.with(|region| match region.fetch(&source)? {
            StackValue::ObjectRef(ObjectRef(None)) => {
                Ok::<Option<Option<usize>>, dotnet_types::error::VmError>(None)
            }
            StackValue::ObjectRef(ObjectRef(Some(handle))) => {
                let inner = handle.borrow();
                match &inner.storage {
                    HeapStorage::Str(chars) => Ok(Some(Some(chars.len()))),
                    _ => Ok(Some(None)),
                }
            }
            _ => Err(dotnet_types::error::ExecutionError::InternalError(
                "native substring source invalid".into(),
            )
            .into()),
        })??;
        let Some(source_len) = source_len else {
            return Err(vm.exception("System.NullReferenceException", NULL_REF_MSG)?);
        };
        let Some(source_len) = source_len else {
            return Err(vm.exception("System.ArgumentException", "The argument must be a string.")?);
        };
        let signed_start = invocation.start as u32 as i32;
        let signed_length =
            (invocation.parameter_count == 2).then_some(invocation.length as u32 as i32);
        let (start, length) = match substring_range(signed_start, signed_length, source_len) {
            Ok(range) => range,
            Err(parameter) => return Err(vm.argument_out_of_range(parameter)?),
        };
        (source, start, length)
    };
    let mut buffer = substring_buffer(length);
    while buffer.len() < length {
        {
            let mut vm = fx.vm().await?;
            vm.with(|region| {
                let StackValue::ObjectRef(ObjectRef(Some(handle))) = region.fetch(&source)? else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native substring source invalid".into(),
                    )
                    .into());
                };
                let inner = handle.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native substring source invalid".into(),
                    )
                    .into());
                };
                if start
                    .checked_add(length)
                    .is_none_or(|end| end > chars.len())
                {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native substring range invalid".into(),
                    )
                    .into());
                }
                copy_substring_chunk(chars, start, length, buffer.len(), &mut buffer);
                Ok::<(), dotnet_types::error::VmError>(())
            })??;
        }
        if buffer.len() < length {
            fx.yield_now().await?;
        }
    }
    fx.vm().await?.push_new_string(buffer)
}

#[cfg(test)]
mod substring_chunk_tests {
    use super::{SUBSTRING_CHUNK_SIZE, copy_substring_chunk, substring_async, substring_range};
    use dotnet_types::error::VmError;
    use dotnet_utils::{ArenaId, gc::ThreadSafeLock};
    use dotnet_value::{
        StackValue,
        object::{HeapStorage, ObjectInner, ObjectRef},
        string::CLRString,
    };
    use dotnet_vm_ops::effects::{
        NativeBackend, NativeControl, NativeEffect, NativeInvocation, NativeRegion, poll_native,
    };
    use gc_arena::{Arena, Collect, DynamicRootSet, Gc, GcWeak, Rootable};
    use std::task::Poll;

    #[derive(Collect)]
    #[collect(no_drop)]
    struct Roots<'gc> {
        roots: DynamicRootSet<'gc>,
        weak: GcWeak<'gc, ThreadSafeLock<ObjectInner<'gc>>>,
    }

    #[derive(Default)]
    struct Mock<'gc> {
        operands: Vec<StackValue<'gc>>,
        outputs: Vec<Vec<u16>>,
        boolean: Option<i32>,
        stop: bool,
    }
    impl<'gc> NativeBackend<'gc> for Mock<'gc> {
        fn stop_requested(&self) -> bool {
            self.stop
        }
        fn active_scope_depth(&self) -> usize {
            0
        }
        fn operands(&mut self) -> &mut Vec<StackValue<'gc>> {
            &mut self.operands
        }
        fn push_new_string(&mut self, chars: Vec<u16>) -> Result<(), VmError> {
            self.outputs.push(chars);
            Ok(())
        }
        fn push_output(&mut self, value: StackValue<'gc>) -> Result<(), VmError> {
            self.boolean = Some(value.as_i32());
            Ok(())
        }
    }

    #[test]
    fn whitespace_keeps_source_and_progress_across_collections() {
        let length = super::WHITESPACE_CHUNK_SIZE * 2 + 1;
        for nonwhite in [false, true] {
            let mut chars = vec![b' ' as u16; length];
            chars[super::WHITESPACE_CHUNK_SIZE] = 0x2003;
            if nonwhite {
                chars[length - 1] = b'x' as u16;
            }
            let mut arena = Arena::<Rootable![Roots<'_>]>::new(|mc| {
                let source = Gc::new(
                    mc,
                    ThreadSafeLock::new(ObjectInner::new(
                        HeapStorage::Str(CLRString::new(chars)),
                        ArenaId::new(1),
                    )),
                );
                Roots {
                    roots: DynamicRootSet::new(mc),
                    weak: Gc::downgrade(source),
                }
            });
            let control = NativeControl::new();
            let fx = control
                .effects(NativeInvocation {
                    start: 0,
                    length,
                    parameter_count: 1,
                })
                .unwrap();
            let mut future = Box::pin(super::whitespace_async(fx));
            arena.mutate(|mc, roots| {
                let mut backend = Mock::default();
                backend.operands.push(StackValue::ObjectRef(ObjectRef(Some(
                    roots.weak.upgrade(mc).unwrap(),
                ))));
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            arena.finish_cycle();
            arena.mutate(|mc, roots| {
                assert!(roots.weak.upgrade(mc).is_some());
                let mut backend = Mock::default();
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            arena.finish_cycle();
            arena.mutate(|mc, roots| {
                let mut backend = Mock::default();
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Ready(Ok(())))
                ));
                assert_eq!(backend.boolean, Some(i32::from(!nonwhite)));
            });
            drop(future);
            arena.finish_cycle();
            arena.mutate(|mc, roots| assert!(roots.weak.upgrade(mc).is_none()));
        }
    }

    #[test]
    fn repeated_constructor_keeps_progress_across_yields() {
        let length = dotnet_vm_ops::effects::BULK_CHUNK_SIZE + 1;
        // The closure supplies the higher-ranked lifetime required by Arena::new.
        #[allow(clippy::redundant_closure)]
        let mut arena = Arena::<Rootable![DynamicRootSet<'_>]>::new(|mc| DynamicRootSet::new(mc));
        let control = NativeControl::new();
        let fx = control
            .effects(NativeInvocation {
                start: 0xd800,
                length,
                parameter_count: 2,
            })
            .unwrap();
        let mut future = Box::pin(crate::constructors::string_repeat_async(fx));
        arena.mutate(|mc, roots| {
            let mut backend = Mock::default();
            backend
                .operands
                .extend([StackValue::Int32(0xd800), StackValue::Int32(length as i32)]);
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert!(backend.outputs.is_empty());
        });
        assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
        arena.finish_cycle();
        arena.mutate(|mc, roots| {
            let mut backend = Mock::default();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Pending)
            ));
            assert!(backend.outputs.is_empty());
        });
        assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
        arena.finish_cycle();
        arena.mutate(|mc, roots| {
            let mut backend = Mock::default();
            let mut region = NativeRegion::new(mc, *roots, &mut backend);
            assert!(matches!(
                poll_native(&control, future.as_mut(), &mut region),
                Ok(Poll::Ready(Ok(())))
            ));
            assert_eq!(backend.outputs, vec![vec![0xd800; length]]);
        });
    }

    #[test]
    fn async_body_preserves_source_and_progress_across_collections() {
        let length = 2 * SUBSTRING_CHUNK_SIZE + 17;
        let chars: Vec<u16> = (0..length + 2).map(|i| i as u16).collect();
        for parameter_count in [1, 2] {
            #[cfg(feature = "bench-instrumentation")]
            super::substring_metrics::reset();
            let mut arena = Arena::<Rootable![Roots<'_>]>::new(|mc| {
                let source = Gc::new(
                    mc,
                    ThreadSafeLock::new(ObjectInner::new(
                        HeapStorage::Str(CLRString::new(chars.clone())),
                        ArenaId::new(1),
                    )),
                );
                Roots {
                    roots: DynamicRootSet::new(mc),
                    weak: Gc::downgrade(source),
                }
            });
            let control = NativeControl::new();
            let fx = control
                .effects(NativeInvocation {
                    start: 2,
                    length,
                    parameter_count,
                })
                .unwrap();
            let mut future = Box::pin(substring_async(fx));
            arena.mutate(|mc, roots| {
                let mut backend = Mock::default();
                backend.operands.push(StackValue::ObjectRef(ObjectRef(Some(
                    roots.weak.upgrade(mc).unwrap(),
                ))));
                backend.operands.push(StackValue::Int32(2));
                if parameter_count == 2 {
                    backend.operands.push(StackValue::Int32(length as i32));
                }
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
                assert!(backend.operands.is_empty());
                assert!(backend.outputs.is_empty());
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            #[cfg(feature = "bench-instrumentation")]
            assert_eq!(
                super::substring_metrics::snapshot().copied_units,
                SUBSTRING_CHUNK_SIZE
            );
            arena.finish_cycle();
            arena.mutate(|mc, roots| {
                assert!(roots.weak.upgrade(mc).is_some());
                let mut backend = Mock {
                    stop: true,
                    ..Default::default()
                };
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
                assert!(backend.outputs.is_empty());
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            #[cfg(feature = "bench-instrumentation")]
            assert_eq!(
                super::substring_metrics::snapshot().copied_units,
                SUBSTRING_CHUNK_SIZE
            );
            arena.mutate(|mc, roots| {
                let mut backend = Mock::default();
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Pending)
                ));
                assert!(backend.outputs.is_empty());
            });
            assert_eq!(control.take_effect(), Some(NativeEffect::Yield));
            #[cfg(feature = "bench-instrumentation")]
            assert_eq!(
                super::substring_metrics::snapshot().copied_units,
                2 * SUBSTRING_CHUNK_SIZE
            );
            arena.finish_cycle();
            arena.mutate(|mc, roots| {
                assert!(roots.weak.upgrade(mc).is_some());
                let mut backend = Mock::default();
                let mut region = NativeRegion::new(mc, roots.roots, &mut backend);
                assert!(matches!(
                    poll_native(&control, future.as_mut(), &mut region),
                    Ok(Poll::Ready(Ok(())))
                ));
                assert_eq!(backend.outputs, vec![chars[2..2 + length].to_vec()]);
            });
            #[cfg(feature = "bench-instrumentation")]
            {
                let metrics = super::substring_metrics::snapshot();
                assert_eq!(metrics.copied_units, length);
                assert_eq!(metrics.buffer_allocations, 1);
                assert_eq!(metrics.buffer_reallocations, 0);
            }
            drop(future);
            arena.finish_cycle();
            arena.mutate(|mc, roots| assert!(roots.weak.upgrade(mc).is_none()));
        }
    }

    #[test]
    fn validates_ranges_and_parameter_order() {
        assert_eq!(substring_range(0, None, 5), Ok((0, 5)));
        assert_eq!(substring_range(5, None, 5), Ok((5, 0)));
        assert_eq!(substring_range(5, Some(0), 5), Ok((5, 0)));
        assert_eq!(substring_range(1, Some(4), 5), Ok((1, 4)));
        assert_eq!(substring_range(-1, Some(-1), 5), Err("startIndex"));
        assert_eq!(substring_range(6, Some(-1), 5), Err("startIndex"));
        assert_eq!(substring_range(1, Some(-1), 5), Err("length"));
        assert_eq!(substring_range(1, Some(i32::MAX), 5), Err("length"));
        assert_eq!(substring_range(i32::MAX, None, 5), Err("startIndex"));
    }

    #[test]
    fn copies_bounded_chunks_without_altering_utf16() {
        let mut source = vec![0x1234; SUBSTRING_CHUNK_SIZE + 4];
        source[SUBSTRING_CHUNK_SIZE] = 0xd800;
        source[SUBSTRING_CHUNK_SIZE + 1] = 0xdc00;
        source[SUBSTRING_CHUNK_SIZE + 2] = 0xdfff;
        let length = SUBSTRING_CHUNK_SIZE + 2;
        let mut result = Vec::new();
        let first = copy_substring_chunk(&source, 1, length, 0, &mut result);
        assert_eq!(first, SUBSTRING_CHUNK_SIZE);
        let second = copy_substring_chunk(&source, 1, length, first, &mut result);
        assert_eq!(second, 2);
        assert_eq!(result, source[1..1 + length]);
    }
}

/// System.String::Substring(int)
/// System.String::Substring(int, int)
pub fn intrinsic_string_substring<'gc, T: IntrinsicStringHost<'gc>>(
    ctx: &mut T,
    method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let (start_at, length) = if method.signature().parameters.len() == 1 {
        let start_at = ctx.peek_stack().as_i32();
        (start_at, None)
    } else {
        let length = ctx.peek_stack().as_i32();
        let start_at = ctx.peek_stack_at(1).as_i32();
        (start_at, Some(length))
    };

    let val = ctx.peek_stack_at(if length.is_some() { 2 } else { 1 });

    let result_vec = match val {
        StackValue::ObjectRef(ObjectRef(Some(handle))) => {
            let s_len = {
                let inner = handle.borrow();
                match &inner.storage {
                    HeapStorage::Str(s) => s.len(),
                    _ => {
                        return ctx.throw_by_name_with_message(
                            "System.ArgumentException",
                            "The argument must be a string.",
                        );
                    }
                }
            };

            let (start_at, l) = match substring_range(start_at, length, s_len) {
                Ok(range) => range,
                Err(parameter) => return ctx.throw_argument_out_of_range(parameter),
            };

            let mut result_vec = substring_buffer(l);
            let mut offset = 0;
            while offset < l {
                let current_chunk = {
                    let _gc_scope = GcScopeGuard::enter(
                        ctx.as_borrow_scope(),
                        ctx.as_borrow_scope().gc_ready_token(),
                    );
                    let inner = handle.borrow();
                    match &inner.storage {
                        HeapStorage::Str(s) => {
                            copy_substring_chunk(s, start_at, l, offset, &mut result_vec)
                        }
                        _ => break,
                    }
                };
                offset += current_chunk;
            }
            result_vec
        }
        StackValue::ObjectRef(ObjectRef(None)) => {
            return ctx.throw_by_name_with_message("System.NullReferenceException", NULL_REF_MSG);
        }
        _ => {
            return ctx.throw_by_name_with_message(
                "System.ArgumentException",
                "The argument must be a string.",
            );
        }
    };

    ctx.drop_top(if length.is_some() { 3 } else { 2 });
    ctx.push_string(CLRString::new(result_vec));
    StepResult::Continue
}

/// System.String::IsNullOrEmpty(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrEmpty(string)")]
pub fn intrinsic_string_is_null_or_empty<'gc, T: TypedStackOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let str_val = ctx.pop();
    let is_null_or_empty = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        StackValue::ObjectRef(ObjectRef(Some(obj))) => {
            let heap = obj.borrow();
            match &heap.storage {
                HeapStorage::Str(s) => s.is_empty(),
                _ => {
                    return StepResult::internal_error(
                        "System.String::IsNullOrEmpty called on non-string object",
                    );
                }
            }
        }
        _ => {
            return StepResult::internal_error(
                "System.String::IsNullOrEmpty called on invalid stack value",
            );
        }
    };
    ctx.push_i32(is_null_or_empty as i32);
    StepResult::Continue
}

const WHITESPACE_CHUNK_SIZE: usize = dotnet_vm_ops::effects::BULK_CHUNK_SIZE / 2;

pub fn whitespace_select<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    _ctx: &T,
    operands: NativeOperandView<'_, 'gc>,
    _method: &MethodDescription,
    _generics: &GenericLookup,
) -> NativeSelection {
    if let StackValue::ObjectRef(ObjectRef(Some(handle))) = operands.peek_at(0) {
        let inner = handle.borrow();
        if let HeapStorage::Str(chars) = &inner.storage
            && chars.len() > WHITESPACE_CHUNK_SIZE
        {
            return NativeSelection::Async(NativeInvocation {
                start: 0,
                length: chars.len(),
                parameter_count: 1,
            });
        }
    }
    NativeSelection::Fallback
}

fn all_whitespace(chars: &[u16]) -> bool {
    probe_all_ascii_whitespace(chars).unwrap_or_else(|| {
        chars.iter().all(|&c| {
            char::from_u32(c as u32)
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
        })
    })
}

/// System.String::IsNullOrWhiteSpace(string)
#[dotnet_intrinsic("static bool System.String::IsNullOrWhiteSpace(string)", select = whitespace_select, fallback = intrinsic_string_is_null_or_white_space)]
pub async fn whitespace_async(mut fx: VmEffects) -> NativeResult {
    let length = fx.invocation().length;
    let source = fx.vm().await?.pop_string()?;
    let mut offset = 0;
    while offset < length {
        let chunk = (length - offset).min(WHITESPACE_CHUNK_SIZE);
        let all_white = {
            let mut vm = fx.vm().await?;
            vm.with(|region| {
                let StackValue::ObjectRef(ObjectRef(Some(handle))) = region.fetch(&source)? else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native whitespace source invalid".into(),
                    )
                    .into());
                };
                let inner = handle.borrow();
                let HeapStorage::Str(chars) = &inner.storage else {
                    return Err(dotnet_types::error::ExecutionError::InternalError(
                        "native whitespace source invalid".into(),
                    )
                    .into());
                };
                Ok::<bool, dotnet_types::error::VmError>(all_whitespace(
                    &chars[offset..offset + chunk],
                ))
            })??
        };
        if !all_white {
            return fx.vm().await?.push_bool(false);
        }
        offset += chunk;
        if offset < length {
            fx.yield_now().await?;
        }
    }
    fx.vm().await?.push_bool(true)
}

pub fn intrinsic_string_is_null_or_white_space<'gc, T: TypedStackOps<'gc> + RawMemoryOps<'gc>>(
    ctx: &mut T,
    _method: MethodDescription,
    _generics: &GenericLookup,
) -> StepResult {
    let str_val = ctx.peek_stack();
    let is_null_or_white_space = match &str_val {
        StackValue::ObjectRef(ObjectRef(None)) => true,
        _ => {
            let obj_ref = str_val.as_object_ref();
            let Some(handle) = obj_ref.0 else {
                // This shouldn't happen given the match arms but we must return StepResult
                ctx.pop();
                ctx.push_i32(1);
                return StepResult::Continue;
            };

            let len = {
                let inner = handle.borrow();
                match &inner.storage {
                    HeapStorage::Str(s) => s.len(),
                    _ => {
                        return StepResult::internal_error(
                            "System.String::IsNullOrWhiteSpace called on non-string object",
                        );
                    }
                }
            };

            let mut offset = 0;
            let mut all_white = true;
            const CHUNK_SIZE: usize = 1024;
            while offset < len {
                let current_chunk_size = std::cmp::min(len - offset, CHUNK_SIZE);
                {
                    let _gc_scope = GcScopeGuard::enter(
                        ctx.as_borrow_scope(),
                        ctx.as_borrow_scope().gc_ready_token(),
                    );
                    let inner = handle.borrow();
                    match &inner.storage {
                        HeapStorage::Str(s) => {
                            let chunk = &s[offset..offset + current_chunk_size];
                            let chunk_all_white = all_whitespace(chunk);
                            if !chunk_all_white {
                                all_white = false;
                            }
                        }
                        _ => break,
                    }
                }

                if !all_white {
                    break;
                }

                offset += current_chunk_size;
            }
            all_white
        }
    };
    ctx.pop();
    ctx.push_i32(is_null_or_white_space as i32);
    StepResult::Continue
}
